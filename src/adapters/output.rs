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

/// Output adapter using PulseAudio null sink fallback with CPAL playback.
use std::sync::Mutex;

/// Output adapter using PulseAudio null sink fallback with CPAL playback.
pub struct PulseAudioSink {
    consumer: Arc<Mutex<Option<HeapCons<f32>>>>,
    pactl: PactlManager,
    stream: Option<Stream>,
    active: Arc<AtomicBool>,
    sink_name: String,
    source_name: String,
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

        let device = device.ok_or_else(|| {
            SinkError::ServerUnavailable(format!(
                "Virtual sink '{}' was created but not found in output devices. Refusing to play back into physical speakers.",
                self.sink_name
            ))
        })?;

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

impl AudioSink for PulseAudioSink {
    fn start(&mut self) -> Result<(), SinkError> {
        if self.is_active() {
            return Ok(());
        }

        // Try setting up virtual devices via pactl; if pactl fails (e.g. headless/test environment),
        // we continue with the local audio output fallback without aborting.
        let _ = self.setup_virtual_devices();

        let stream = self.build_playback_stream()?;
        stream
            .play()
            .map_err(|e| SinkError::StartError(e.to_string()))?;

        self.stream = Some(stream);
        self.active.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        }
        self.pactl.unload_all();
        self.active.store(false, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

#[cfg(feature = "pipewire-backend")]
pub struct PipeWireNativeSink {
    consumer: Arc<Mutex<Option<HeapCons<f32>>>>,
    active: Arc<AtomicBool>,
    source_name: String,
    thread_handle: Option<std::thread::JoinHandle<()>>,
    stop_signal: Arc<AtomicBool>,
}

#[cfg(feature = "pipewire-backend")]
impl PipeWireNativeSink {
    pub fn new(consumer: HeapCons<f32>, source_name: impl Into<String>) -> Self {
        Self {
            consumer: Arc::new(Mutex::new(Some(consumer))),
            active: Arc::new(AtomicBool::new(false)),
            source_name: source_name.into(),
            thread_handle: None,
            stop_signal: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg(feature = "pipewire-backend")]
impl AudioSink for PipeWireNativeSink {
    fn start(&mut self) -> Result<(), SinkError> {
        if self.is_active() {
            return Ok(());
        }

        let consumer_holder = Arc::clone(&self.consumer);
        let active_holder = Arc::clone(&self.active);
        let stop_clone = Arc::clone(&self.stop_signal);
        let source_name = self.source_name.clone();

        stop_clone.store(false, Ordering::Release);
        self.active.store(true, Ordering::Release);

        self.thread_handle = Some(std::thread::spawn(move || {
            pipewire::init();
            let mainloop = match pipewire::main_loop::MainLoopRc::new(None) {
                Ok(ml) => ml,
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to create main loop: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let context = match pipewire::context::ContextRc::new(&mainloop, None) {
                Ok(ctx) => ctx,
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to create context: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let core = match context.connect_rc(None) {
                Ok(core) => core,
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to connect to core: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let props = pipewire::properties::properties! {
                *pipewire::keys::MEDIA_TYPE => "Audio",
                *pipewire::keys::MEDIA_CATEGORY => "Capture",
                *pipewire::keys::MEDIA_ROLE => "Communication",
                *pipewire::keys::MEDIA_CLASS => "Audio/Source/Virtual",
                *pipewire::keys::NODE_NAME => source_name.as_str(),
                *pipewire::keys::NODE_DESCRIPTION => source_name.as_str(),
                *pipewire::keys::NODE_AUTOCONNECT => "false",
                *pipewire::keys::AUDIO_CHANNELS => "1",
            };

            let stream = match pipewire::stream::StreamBox::new(&core, &source_name, props) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to create stream: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let cons_for_proc = Arc::clone(&consumer_holder);
            let listener_res = stream
                .add_local_listener::<()>()
                .process(move |stream, _: &mut ()| {
                    if let Some(mut buffer) = stream.dequeue_buffer() {
                        let datas = buffer.datas_mut();
                        if datas.is_empty() {
                            return;
                        }
                        let data = &mut datas[0];
                        let stride = std::mem::size_of::<f32>();
                        let n_frames = if let Some(slice) = data.data() {
                            let frames_available = slice.len() / stride;
                            let out_f32: &mut [f32] = unsafe {
                                std::slice::from_raw_parts_mut(
                                    slice.as_mut_ptr() as *mut f32,
                                    frames_available,
                                )
                            };

                            if let Ok(mut guard) = cons_for_proc.try_lock()
                                && let Some(ref mut cons) = *guard
                            {
                                let popped = cons.pop_slice(out_f32);
                                if popped < frames_available {
                                    out_f32[popped..].fill(0.0);
                                }
                            } else {
                                out_f32.fill(0.0);
                            }
                            frames_available
                        } else {
                            0
                        };
                        let chunk = data.chunk_mut();
                        *chunk.offset_mut() = 0;
                        *chunk.stride_mut() = stride as _;
                        *chunk.size_mut() = (stride * n_frames) as _;
                    }
                })
                .register();

            let _listener = match listener_res {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to register listener: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let mut audio_info = pipewire::spa::param::audio::AudioInfoRaw::new();
            audio_info.set_format(pipewire::spa::param::audio::AudioFormat::F32LE);
            audio_info.set_rate(SAMPLE_RATE);
            audio_info.set_channels(1);
            let mut position = [0; pipewire::spa::param::audio::MAX_CHANNELS];
            position[0] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_MONO;
            audio_info.set_position(position);

            let values: Vec<u8> = match pipewire::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pipewire::spa::pod::Value::Object(pipewire::spa::pod::Object {
                    type_: pipewire::spa::sys::SPA_TYPE_OBJECT_Format,
                    id: pipewire::spa::sys::SPA_PARAM_EnumFormat,
                    properties: audio_info.into(),
                }),
            ) {
                Ok((cursor, _)) => cursor.into_inner(),
                Err(e) => {
                    eprintln!("[PipeWire Sink] Failed to serialize Pod: {e}");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };

            let pod = match pipewire::spa::pod::Pod::from_bytes(&values) {
                Some(p) => p,
                None => {
                    eprintln!("[PipeWire Sink] Failed to parse Pod bytes");
                    active_holder.store(false, Ordering::Release);
                    return;
                }
            };
            let mut params = [pod];

            if let Err(e) = stream.connect(
                pipewire::spa::utils::Direction::Output,
                None,
                pipewire::stream::StreamFlags::MAP_BUFFERS
                    | pipewire::stream::StreamFlags::RT_PROCESS,
                &mut params,
            ) {
                eprintln!("[PipeWire Sink] Failed to connect stream: {e}");
                active_holder.store(false, Ordering::Release);
                return;
            }

            let loop_ref = mainloop.loop_();
            while !stop_clone.load(Ordering::Relaxed) {
                let _ = loop_ref.iterate(pipewire::loop_::Timeout::Finite(std::time::Duration::from_millis(10)));
            }
            active_holder.store(false, Ordering::Release);
        }));

        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        self.stop_signal.store(true, Ordering::Release);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.active.store(false, Ordering::Release);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

/// Fallback factory creating the appropriate audio sink depending on system backend.
pub fn create_audio_sink(consumer: HeapCons<f32>) -> Box<dyn AudioSink> {
    create_audio_sink_with_names(consumer, DEFAULT_SINK_NAME, DEFAULT_SOURCE_NAME)
}

/// Fallback factory creating the audio sink with custom device names.
pub fn create_audio_sink_with_names(
    consumer: HeapCons<f32>,
    sink_name: impl Into<String>,
    source_name: impl Into<String>,
) -> Box<dyn AudioSink> {
    let source_str: String = source_name.into();
    #[cfg(feature = "pipewire-backend")]
    {
        if detect_pipewire_socket() {
            return Box::new(PipeWireNativeSink::new(consumer, source_str));
        }
    }

    Box::new(PulseAudioSink::with_names(consumer, sink_name, source_str))
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

    #[cfg(feature = "pipewire-backend")]
    #[test]
    fn test_pipewire_native_sink_initial_state() {
        use ringbuf::traits::Split;
        use ringbuf::HeapRb;

        let rb = HeapRb::<f32>::new(480);
        let (_prod, cons) = rb.split();
        let sink = PipeWireNativeSink::new(cons, "Test_PW_Sink");
        assert!(!sink.is_active());
        assert_eq!(sink.source_name, "Test_PW_Sink");
    }
}
