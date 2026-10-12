use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ringbuf::traits::Producer;
use ringbuf::HeapProd;

use std::process::Command;

use crate::domain::config::SAMPLE_RATE;
use crate::ports::source::{AudioSource, SourceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInputDevice {
    pub name: String,
    pub description: String,
}

impl AudioInputDevice {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

/// Parses output of `LC_ALL=C pactl list sources` to extract real physical recording devices.
pub fn parse_pactl_sources(text: &str) -> Vec<AudioInputDevice> {
    let mut devices = Vec::new();
    let mut current_name = String::new();
    let mut current_desc = String::new();
    let mut in_source = false;

    let push_device = |devs: &mut Vec<AudioInputDevice>, name: &str, desc: &str| {
        if !name.is_empty()
            && !name.ends_with(".monitor")
            && !name.contains("Shizune")
            && !name.contains("null-sink")
        {
            let final_desc = if desc.is_empty() {
                name.to_string()
            } else {
                desc.to_string()
            };
            devs.push(AudioInputDevice {
                name: name.to_string(),
                description: final_desc,
            });
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Source #") {
            if in_source {
                push_device(&mut devices, &current_name, &current_desc);
            }
            in_source = true;
            current_name.clear();
            current_desc.clear();
        } else if in_source {
            if let Some(val) = trimmed.strip_prefix("Name: ") {
                current_name = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("device.description = ") {
                let cleaned = val.trim().trim_matches('"').to_string();
                if !cleaned.is_empty() {
                    current_desc = cleaned;
                }
            } else if current_desc.is_empty() && let Some(val) = trimmed.strip_prefix("Description: ") {
                current_desc = val.trim().to_string();
            }
        }
    }

    if in_source {
        push_device(&mut devices, &current_name, &current_desc);
    }

    devices
}

/// Queries PipeWire/PulseAudio via pactl to retrieve human-friendly device descriptions,
/// falling back to raw CPAL host devices if pactl is unavailable.
pub fn list_system_input_devices() -> Vec<AudioInputDevice> {
    if let Ok(output) = Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("list")
        .arg("sources")
        .output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let devices = parse_pactl_sources(&text);
        if !devices.is_empty() {
            return devices;
        }
    }

    let raw_names = CpalAudioSource::list_input_devices().unwrap_or_default();
    raw_names
        .into_iter()
        .map(|name| AudioInputDevice {
            description: name.clone(),
            name,
        })
        .collect()
}

/// Moves all active recording streams of this process to the specified PipeWire/PulseAudio source.
pub fn move_current_process_source_output_to(target_source_name: &str) {
    let my_pid = std::process::id().to_string();
    let output = match Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("list")
        .arg("source-outputs")
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
        if let Some(id_str) = trimmed.strip_prefix("Source Output #") {
            if let (Some(id), true) = (current_id.take(), is_my_process) {
                let _ = Command::new("pactl")
                    .arg("move-source-output")
                    .arg(&id)
                    .arg(target_source_name)
                    .status();
            }
            current_id = id_str.trim().parse::<u32>().ok().map(|n| n.to_string());
            is_my_process = false;
        } else if trimmed.contains(&format!("application.process.id = \"{my_pid}\""))
            || trimmed.contains("application.process.binary = \"shizune\"")
            || trimmed.contains("application.name = \"shizune\"")
        {
            is_my_process = true;
        }
    }

    if let (Some(id), true) = (current_id, is_my_process) {
        let _ = Command::new("pactl")
            .arg("move-source-output")
            .arg(&id)
            .arg(target_source_name)
            .status();
    }
}

unsafe extern "C" {
    fn snd_lib_error_set_handler(handler: Option<extern "C" fn(*const i8, i32, *const i8, i32, *const i8)>) -> i32;
}

extern "C" fn noop_alsa_error_handler(
    _file: *const i8,
    _line: i32,
    _function: *const i8,
    _err: i32,
    _fmt: *const i8,
) {}

pub fn silence_alsa_logging() {
    unsafe {
        let _ = snd_lib_error_set_handler(Some(noop_alsa_error_handler));
    }
}

pub struct CpalAudioSource {
    producer: Arc<Mutex<Option<HeapProd<f32>>>>,
    stream: Option<Stream>,
    active: Arc<AtomicBool>,
    device_name: Option<String>,
}

impl CpalAudioSource {
    pub fn new(producer: HeapProd<f32>) -> Self {
        Self::new_with_device(producer, None)
    }

    pub fn new_with_device(producer: HeapProd<f32>, device_name: Option<String>) -> Self {
        silence_alsa_logging();
        Self {
            producer: Arc::new(Mutex::new(Some(producer))),
            stream: None,
            active: Arc::new(AtomicBool::new(false)),
            device_name,
        }
    }

    /// Lists all available physical input audio device names on the default CPAL host,
    /// excluding virtual sinks and monitors to prevent audio feedback loops.
    pub fn list_input_devices() -> Result<Vec<String>, SourceError> {
        silence_alsa_logging();
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| SourceError::StreamConfigError(e.to_string()))?;
        let names = devices
            .filter_map(|d| d.name().ok())
            .filter(|name| !name.contains("Shizune") && !name.contains("null-sink"))
            .collect();
        Ok(names)
    }

    fn build_stream(&mut self) -> Result<Stream, SourceError> {
        let host = cpal::default_host();
        let device = if let Some(ref name) = self.device_name {
            host.input_devices()
                .map_err(|e| SourceError::StreamConfigError(e.to_string()))?
                .find(|d| d.name().map(|n| n == *name || n.contains(name)).unwrap_or(false))
                .ok_or(SourceError::DeviceNotFound)?
        } else {
            host.default_input_device()
                .ok_or(SourceError::DeviceNotFound)?
        };

        let default_config = device
            .default_input_config()
            .map_err(|e| SourceError::StreamConfigError(e.to_string()))?;

        let channels = default_config.channels();
        let supported_format = default_config.sample_format();

        let config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let active_clone = Arc::clone(&self.active);
        let err_fn = move |err| {
            eprintln!("[CPAL Input] Stream error: {err}. Hardware may have disconnected.");
            active_clone.store(false, Ordering::Release);
        };

        let ch_count = channels as usize;
        let producer_holder = Arc::clone(&self.producer);
        let stream = match supported_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut guard) = producer_holder.try_lock()
                        && let Some(ref mut prod) = *guard
                    {
                        if ch_count == 1 {
                            prod.push_slice(data);
                        } else {
                            for frame in data.chunks_exact(ch_count) {
                                let mono: f32 = frame.iter().sum::<f32>() / ch_count as f32;
                                let _ = prod.try_push(mono);
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut guard) = producer_holder.try_lock()
                        && let Some(ref mut prod) = *guard
                    {
                        if ch_count == 1 {
                            for &sample in data {
                                let sample_f32 = sample as f32 / i16::MAX as f32;
                                let _ = prod.try_push(sample_f32);
                            }
                        } else {
                            for frame in data.chunks_exact(ch_count) {
                                let sum: f32 = frame
                                    .iter()
                                    .map(|&s| s as f32 / i16::MAX as f32)
                                    .sum();
                                let _ = prod.try_push(sum / ch_count as f32);
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut guard) = producer_holder.try_lock()
                        && let Some(ref mut prod) = *guard
                    {
                        if ch_count == 1 {
                            for &sample in data {
                                let sample_f32 = (sample as f32 / u16::MAX as f32) * 2.0 - 1.0;
                                let _ = prod.try_push(sample_f32);
                            }
                        } else {
                            for frame in data.chunks_exact(ch_count) {
                                let sum: f32 = frame
                                    .iter()
                                    .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                                    .sum();
                                let _ = prod.try_push(sum / ch_count as f32);
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            _ => return Err(SourceError::StreamConfigError("Unsupported sample format".into())),
        }
        .map_err(|e| SourceError::StartError(e.to_string()))?;

        Ok(stream)
    }

    /// Attempts to recreate and start the stream if it was disconnected.
    pub fn reconnect_if_needed(&mut self) -> Result<bool, SourceError> {
        if !self.is_active() {
            let has_prod = self
                .producer
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false);
            if has_prod
                && let Ok(stream) = self.build_stream()
                && stream.play().is_ok()
            {
                self.stream = Some(stream);
                self.active.store(true, Ordering::Release);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Switches to a different physical microphone device, stopping the current stream and rebuilding it.
    pub fn switch_device(&mut self, new_device_name: Option<String>) -> Result<(), SourceError> {
        let _ = self.stop();
        self.device_name = new_device_name;
        self.start()?;
        Ok(())
    }

    /// Switches the input stream to a specific AudioInputDevice.
    /// Sets PULSE_SOURCE, moves existing PipeWire source-outputs, and rebuilds CPAL stream if needed.
    pub fn switch_to_device(&mut self, device: Option<&AudioInputDevice>) -> Result<(), SourceError> {
        if let Some(dev) = device {
            unsafe {
                std::env::set_var("PULSE_SOURCE", &dev.name);
            }
            move_current_process_source_output_to(&dev.name);

            // If the device directly corresponds to a CPAL ALSA device name, rebind CPAL stream
            let is_direct_cpal_match = Self::list_input_devices()
                .map(|list| list.iter().any(|d| d == &dev.name || d.contains(&dev.name)))
                .unwrap_or(false);

            if is_direct_cpal_match {
                self.switch_device(Some(dev.name.clone()))?;
            }
        } else {
            unsafe {
                std::env::remove_var("PULSE_SOURCE");
            }
            move_current_process_source_output_to("@DEFAULT_SOURCE@");
            let _ = self.switch_device(None);
        }
        Ok(())
    }
}

impl AudioSource for CpalAudioSource {
    fn start(&mut self) -> Result<(), SourceError> {
        if self.is_active() {
            return Ok(());
        }

        let stream = self.build_stream()?;
        stream
            .play()
            .map_err(|e| SourceError::StartError(e.to_string()))?;

        self.stream = Some(stream);
        self.active.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SourceError> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        }
        self.active.store(false, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::Split;
    use ringbuf::HeapRb;

    #[test]
    fn test_cpal_source_initial_state_and_stop() {
        let rb = HeapRb::<f32>::new(480);
        let (prod, _cons) = rb.split();
        let mut source = CpalAudioSource::new(prod);
        assert!(!source.is_active());
        assert!(source.stop().is_ok());
        assert!(!source.is_active());
    }

    #[test]
    fn test_cpal_reconnect_nonexistent_device_returns_false() {
        let rb = HeapRb::<f32>::new(480);
        let (prod, _cons) = rb.split();
        let mut source = CpalAudioSource::new_with_device(
            prod,
            Some("NonExistentDevice_XYZ_12345".to_string()),
        );
        assert!(!source.is_active());
        // Attempting reconnection to a nonexistent device should safely return Ok(false)
        let reconnected = source.reconnect_if_needed();
        assert_eq!(reconnected.unwrap(), false);
        assert!(!source.is_active());
    }

    #[test]
    fn test_cpal_switch_device_nonexistent() {
        let rb = HeapRb::<f32>::new(480);
        let (prod, _cons) = rb.split();
        let mut source = CpalAudioSource::new(prod);
        let result = source.switch_device(Some("NonExistentDevice_XYZ_99999".to_string()));
        assert!(result.is_err());
        assert!(!source.is_active());
    }

    #[test]
    fn test_parse_pactl_sources_extracts_clean_descriptions() {
        let sample = r#"
Source #1
	State: SUSPENDED
	Name: alsa_input.pci-0000_00_1f.3.analog-stereo
	Description: Built-in Audio Analog Stereo
	Properties:
		device.description = "Audio interno Estéreo analógico"

Source #2
	State: RUNNING
	Name: alsa_output.pci-0000_00_1f.3.analog-stereo.monitor
	Description: Monitor of Built-in Audio Analog Stereo

Source #3
	State: RUNNING
	Name: alsa_input.usb-Kingston_HyperX_SoloCast-00.mono-fallback
	Description: HyperX SoloCast
	Properties:
		device.description = "HyperX SoloCast"

Source #4
	State: IDLE
	Name: Shizune_Virtual_Mic
	Description: Shizune Virtual Mic
"#;

        let devices = parse_pactl_sources(sample);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "alsa_input.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(devices[0].description, "Audio interno Estéreo analógico");
        assert_eq!(devices[1].name, "alsa_input.usb-Kingston_HyperX_SoloCast-00.mono-fallback");
        assert_eq!(devices[1].description, "HyperX SoloCast");
    }

    #[test]
    fn test_silence_alsa_logging_does_not_panic() {
        silence_alsa_logging();
    }
}
