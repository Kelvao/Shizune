use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ringbuf::traits::Consumer;
use ringbuf::HeapCons;

use crate::domain::config::SAMPLE_RATE;
use crate::ports::sink::{AudioSink, SinkError};

unsafe extern "C" {
    fn getuid() -> u32;
}

pub const DEFAULT_SINK_NAME: &str = "Shizune_Sink";
pub const DEFAULT_SOURCE_NAME: &str = "Shizune_Virtual_Mic";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputBackend {
    PipeWire,
    PulseAudioFallback,
}

/// Detects whether the PipeWire daemon socket is present in the user runtime directory.
pub fn detect_pipewire_socket() -> bool {
    let uid = unsafe { getuid() };
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{uid}"));
    let socket_path = PathBuf::from(runtime_dir).join("pipewire-0");
    socket_path.exists()
}

/// Retrieves the current system default recording source via pactl.
pub fn get_system_default_source() -> Option<String> {
    if let Ok(output) = Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("get-default-source")
        .output()
        && output.status.success()
    {
        let prev = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !prev.is_empty() {
            return Some(prev);
        }
    }
    None
}

/// RAII manager for modules loaded via pactl.
pub struct PactlManager {
    loaded_module_ids: Vec<String>,
}

impl Default for PactlManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PactlManager {
    pub fn new() -> Self {
        Self {
            loaded_module_ids: Vec::new(),
        }
    }

    /// Loads a PulseAudio/PipeWire module and records its ID for graceful unloading.
    pub fn load_module(&mut self, module_name: &str, args: &[&str]) -> Result<String, SinkError> {
        let mut cmd = Command::new("pactl");
        cmd.arg("load-module").arg(module_name);
        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd.output().map_err(|e| {
            SinkError::ServerUnavailable(format!("Failed to execute pactl: {e}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SinkError::StartError(format!(
                "pactl load-module {module_name} failed: {stderr}"
            )));
        }

        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.loaded_module_ids.push(id.clone());
        Ok(id)
    }

    /// Unloads all recorded modules in reverse order.
    pub fn unload_all(&mut self) {
        while let Some(id) = self.loaded_module_ids.pop() {
            let _ = Command::new("pactl")
                .arg("unload-module")
                .arg(&id)
                .status();
        }
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_module_ids.len()
    }
}

impl Drop for PactlManager {
    fn drop(&mut self) {
        self.unload_all();
    }
}

use std::sync::Mutex;

/// Unloads any stale modules left by previous crashes or unclean shutdowns.
fn unload_stale_modules(sink_name: &str, source_name: &str) {
    let output = match Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("list")
        .arg("modules")
        .arg("short")
        .output()
    {
        Ok(out) if out.status.success() => out.stdout,
        _ => return,
    };

    let text = String::from_utf8_lossy(&output);
    for line in text.lines() {
        let trimmed = line.trim();
        if (trimmed.contains(sink_name) || trimmed.contains(source_name))
            && let Some(id_str) = trimmed.split_whitespace().next()
            && id_str.parse::<u32>().is_ok()
        {
            let _ = Command::new("pactl")
                .arg("unload-module")
                .arg(id_str)
                .status();
        }
    }
}

/// Output adapter using PulseAudio null sink fallback with CPAL playback.
pub struct PulseAudioSink {
    consumer: Arc<Mutex<Option<HeapCons<f32>>>>,
    pactl: PactlManager,
    stream: Option<Stream>,
    active: Arc<AtomicBool>,
    sink_name: String,
    source_name: String,
    previous_default_source: Option<String>,
}

impl PulseAudioSink {
    pub fn new(consumer: HeapCons<f32>) -> Self {
        Self::with_names(consumer, DEFAULT_SINK_NAME, DEFAULT_SOURCE_NAME)
    }

    pub fn with_names(
        consumer: HeapCons<f32>,
        sink_name: impl Into<String>,
        source_name: impl Into<String>,
    ) -> Self {
        Self {
            consumer: Arc::new(Mutex::new(Some(consumer))),
            pactl: PactlManager::new(),
            stream: None,
            active: Arc::new(AtomicBool::new(false)),
            sink_name: sink_name.into(),
            source_name: source_name.into(),
            previous_default_source: None,
        }
    }

    /// Lists all available output audio device names on the default CPAL host.
    pub fn list_output_devices() -> Result<Vec<String>, SinkError> {
        let host = cpal::default_host();
        let devices = host
            .output_devices()
            .map_err(|e| SinkError::ServerUnavailable(e.to_string()))?;
        let names = devices.filter_map(|d| d.name().ok()).collect();
        Ok(names)
    }

    fn setup_virtual_devices(&mut self) -> Result<(), SinkError> {
        unload_stale_modules(&self.sink_name, &self.source_name);

        let sink_prop = format!("sink_properties=device.description={}", self.sink_name);
        let sink_arg = format!("sink_name={}", self.sink_name);
        self.pactl
            .load_module("module-null-sink", &[&sink_arg, &sink_prop])?;

        let monitor = format!("{}.monitor", self.sink_name);
        let source_arg = format!("source_name={}", self.source_name);
        let master_arg = format!("master={monitor}");
        let source_prop = format!("source_properties=device.description={}", self.source_name);

        self.pactl.load_module(
            "module-remap-source",
            &[&source_arg, &master_arg, &source_prop],
        )?;

        // Query and store previous default recording source
        if let Ok(output) = Command::new("pactl").arg("get-default-source").output()
            && output.status.success()
        {
            let prev = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !prev.is_empty() && prev != self.source_name {
                self.previous_default_source = Some(prev);
            }
        }

        // Set the clean virtual microphone as the system default recording device
        let _ = Command::new("pactl")
            .arg("set-default-source")
            .arg(&self.source_name)
            .status();

        Ok(())
    }

    fn build_playback_stream(&mut self) -> Result<Stream, SinkError> {
        let host = cpal::default_host();
        // Wait up to 500ms for PulseAudio/PipeWire-Pulse to register the newly created null sink
        let mut device = None;
        for _ in 0..10 {
            if let Ok(mut devices) = host.output_devices()
                && let Some(d) = devices.find(|d| {
                    d.name()
                        .map(|name| name.contains(&self.sink_name))
                        .unwrap_or(false)
                })
            {
                device = Some(d);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // If no direct null-sink PCM exists in ALSA, fallback to default output device with PULSE_SINK routing
        let device = device
            .or_else(|| host.default_output_device())
            .ok_or_else(|| SinkError::ServerUnavailable("No audio output device available".into()))?;

        let default_config = device
            .default_output_config()
            .map_err(|e| SinkError::ConfigurationError(e.to_string()))?;

        let channels = default_config.channels();
        let config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let err_fn = |err| {
            eprintln!("[PulseAudio Sink] Stream error: {err}");
        };

        let ch_count = channels as usize;
        let consumer_holder = Arc::clone(&self.consumer);
        let stream = match default_config.sample_format() {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut guard) = consumer_holder.try_lock()
                        && let Some(ref mut cons) = *guard
                    {
                        if ch_count == 1 {
                            let popped = cons.pop_slice(data);
                            if popped < data.len() {
                                data[popped..].fill(0.0);
                            }
                        } else {
                            data.fill(0.0);
                            for frame in data.chunks_exact_mut(ch_count) {
                                let sample = cons.try_pop().unwrap_or(0.0);
                                frame.fill(sample);
                            }
                        }
                        return;
                    }
                    data.fill(0.0);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut guard) = consumer_holder.try_lock()
                        && let Some(ref mut cons) = *guard
                    {
                        data.fill(0);
                        for frame in data.chunks_exact_mut(ch_count) {
                            let sample_f32 = cons.try_pop().unwrap_or(0.0);
                            let sample_i16 = (sample_f32 * i16::MAX as f32) as i16;
                            frame.fill(sample_i16);
                        }
                        return;
                    }
                    data.fill(0);
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut guard) = consumer_holder.try_lock()
                        && let Some(ref mut cons) = *guard
                    {
                        data.fill(u16::MAX / 2);
                        for frame in data.chunks_exact_mut(ch_count) {
                            let sample_f32 = cons.try_pop().unwrap_or(0.0);
                            let sample_u16 = (((sample_f32 + 1.0) * 0.5) * u16::MAX as f32) as u16;
                            frame.fill(sample_u16);
                        }
                        return;
                    }
                    data.fill(u16::MAX / 2);
                },
                err_fn,
                None,
            ),
            _ => return Err(SinkError::ConfigurationError("Unsupported sample format".into())),
        }
        .map_err(|e| SinkError::StartError(e.to_string()))?;

        Ok(stream)
    }
}

/// Moves all active playback streams of this process to the specified virtual sink.
pub fn move_current_process_sink_input_to(target_sink_name: &str) {
    let my_pid = std::process::id().to_string();
    let output = match Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("list")
        .arg("sink-inputs")
        .output()
    {
        Ok(out) if out.status.success() => out.stdout,
        _ => return,
    };

    let text = String::from_utf8_lossy(&output);
    let mut current_id = None;
    let mut is_my_process = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(id_str) = trimmed.strip_prefix("Sink Input #") {
            if let (Some(id), true) = (current_id.take(), is_my_process) {
                let _ = Command::new("pactl")
                    .arg("move-sink-input")
                    .arg(&id)
                    .arg(target_sink_name)
                    .status();
            }
            current_id = id_str.trim().parse::<u32>().ok().map(|n| n.to_string());
            is_my_process = false;
        } else if trimmed.contains(&my_pid)
            || trimmed.to_lowercase().contains("shizune")
        {
            is_my_process = true;
        }
    }

    if let (Some(id), true) = (current_id, is_my_process) {
        let _ = Command::new("pactl")
            .arg("move-sink-input")
            .arg(&id)
            .arg(target_sink_name)
            .status();
    }
}

impl AudioSink for PulseAudioSink {
    fn start(&mut self) -> Result<(), SinkError> {
        if self.is_active() {
            return Ok(());
        }

        // Try setting up virtual devices via pactl; if pactl fails (e.g. headless/test environment),
        // we continue with the local audio output fallback without aborting.
        let _ = self.setup_virtual_devices();

        unsafe {
            std::env::set_var("PULSE_SINK", &self.sink_name);
            std::env::set_var("PIPEWIRE_NODE", &self.sink_name);
        }

        let stream = self.build_playback_stream()?;
        stream
            .play()
            .map_err(|e| SinkError::StartError(e.to_string()))?;

        self.stream = Some(stream);
        self.active.store(true, Ordering::Release);

        // Ensure playback stream is routed into the virtual sink
        move_current_process_sink_input_to(&self.sink_name);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        }
        if let Some(ref prev) = self.previous_default_source.take() {
            let _ = Command::new("pactl")
                .arg("set-default-source")
                .arg(prev)
                .status();
        }
        self.pactl.unload_all();
        self.active.store(false, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

impl Drop for PulseAudioSink {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Fallback factory creating the appropriate audio sink depending on system backend.
pub fn create_audio_sink(consumer: HeapCons<f32>) -> Box<dyn AudioSink> {
    create_audio_sink_with_names(consumer, DEFAULT_SINK_NAME, DEFAULT_SOURCE_NAME)
}

/// Factory creating the audio sink with custom device names.
/// Uses the universal PulseAudio/PipeWire null-sink route for rock-solid stability across all Linux distros.
pub fn create_audio_sink_with_names(
    consumer: HeapCons<f32>,
    sink_name: impl Into<String>,
    source_name: impl Into<String>,
) -> Box<dyn AudioSink> {
    Box::new(PulseAudioSink::with_names(consumer, sink_name, source_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_pipewire_socket_does_not_panic() {
        let _ = detect_pipewire_socket();
    }

    #[test]
    fn test_pactl_manager_raii_initial_state() {
        let manager = PactlManager::new();
        assert_eq!(manager.loaded_count(), 0);
    }
}
