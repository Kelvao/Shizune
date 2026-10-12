use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use shizune::adapters::cpal_source::CpalAudioSource;
use shizune::adapters::output::{create_audio_sink, detect_pipewire_socket};
#[cfg(feature = "gui")]
use shizune::adapters::ui::launch_gui;
use shizune::domain::{AudioConfig, AudioEngine, BufferPipeline, DeepFilterProcessor, EngineState};
use shizune::ports::source::AudioSource;

fn print_help() {
    println!("Usage: shizune [OPTIONS]");
    println!("Options:");
    println!("  --list-devices            List all available input and output audio devices");
    println!("  --input <name>            Substring match for the input microphone device");
    println!("  --attenuation <dB>        Initial attenuation limit in dB [0.0 - 100.0] (default: 100.0)");
    println!("  --headless                Run in headless mode without graphical interface");
    println!("  --help                    Show this help message");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    shizune::domain::state::setup_signal_handlers();
    let args: Vec<String> = std::env::args().collect();
    let mut input_device: Option<String> = None;
    let mut initial_attenuation: Option<f32> = None;
    let mut headless = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => {
                headless = true;
            }
            "--list-devices" => {
                println!("=== Available Input Devices ===");
                let devices = shizune::adapters::cpal_source::list_system_input_devices();
                if devices.is_empty() {
                    println!("  (No physical input microphones detected)");
                } else {
                    for d in devices {
                        println!("  - {} (device: {})", d.description, d.name);
                    }
                }
                println!("\n=== Available Output Devices ===");
                match shizune::adapters::output::PulseAudioSink::list_output_devices() {
                    Ok(devices) => {
                        for d in devices {
                            println!("  - {d}");
                        }
                    }
                    Err(e) => eprintln!("Error listing output devices: {e}"),
                }
                return Ok(());
            }
            "--input" => {
                if i + 1 < args.len() {
                    input_device = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--attenuation" => {
                if i + 1 < args.len() {
                    if let Ok(val) = args[i + 1].parse::<f32>() {
                        initial_attenuation = Some(val);
                    }
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    let config = AudioConfig::default();
    let state = EngineState::new_shared();
    if let Some(atten) = initial_attenuation {
        state.attenuation_limit.set(atten);
    }

    println!("=======================================================");
    println!("  Shizune: Deep Virtual Mic (Rust)                     ");
    println!("=======================================================");
    println!(
        "Audio Config: {} Hz | Frame: {} samples ({:.1} ms) | Buffer: {} samples ({:.1} ms)",
        config.sample_rate,
        config.frame_size,
        config.frame_duration_ms(),
        config.ring_buffer_capacity,
        config.buffer_duration_ms()
    );
    println!(
        "Initial State: Enabled={} | Attenuation Limit={:.1} dB",
        state.is_enabled(),
        state.attenuation_limit.get()
    );

    let pipewire_detected = detect_pipewire_socket();
    println!(
        "Server Handshake: PipeWire socket present = {pipewire_detected} (fallback route: PulseAudio null-sink)"
    );

    let pipeline = BufferPipeline::new(config.ring_buffer_capacity);
    let processor = DeepFilterProcessor::new();

    let stop_signal = Arc::new(AtomicBool::new(false));
    let engine = AudioEngine::new(
        pipeline.input_cons,
        pipeline.output_prod,
        Arc::clone(&state),
        processor,
    );
    let engine_handle = engine.spawn(Arc::clone(&stop_signal));

    let sys_inputs = shizune::adapters::cpal_source::list_system_input_devices();
    let resolved_device = input_device.as_ref().and_then(|query| {
        sys_inputs
            .iter()
            .find(|d| d.description.to_lowercase().contains(&query.to_lowercase()) || d.name.contains(query))
            .map(|d| d.name.clone())
    }).or(input_device);

    if let Some(ref dev_name) = resolved_device {
        unsafe {
            std::env::set_var("PULSE_SOURCE", dev_name);
        }
    }

    let source = Rc::new(RefCell::new(CpalAudioSource::new_with_device(
        pipeline.input_prod,
        resolved_device,
    )));

    if let Ok(mut src) = source.try_borrow_mut() {
        if let Err(e) = src.start() {
            eprintln!("[Warning] Audio capture input not started: {e}");
        } else {
            println!("Microphone capture stream started.");
        }
    }

    let mut sink = create_audio_sink(pipeline.output_cons);
    if let Err(e) = sink.start() {
        eprintln!("[Warning] Audio output sink not started: {e}");
    } else {
        println!("Virtual microphone output sink started: Shizune_Virtual_Mic");
        println!("[Info] Shizune_Virtual_Mic is active and set as system default recording device.");
        println!("[Info] In voice apps (Discord, OBS, Steam), select 'Shizune_Virtual_Mic' or 'Default' to use filtered audio.");
    }

    println!("=== Available Microphone Inputs ===");
    if sys_inputs.is_empty() {
        println!("  (No physical input microphones detected; using system default)");
    } else {
        for (idx, d) in sys_inputs.iter().enumerate() {
            println!("  [{}] {} (device: {})", idx + 1, d.description, d.name);
        }
    }

    println!("Audio engine and I/O adapters initialized and running.");

    #[allow(unused_mut)]
    let mut run_headless_loop = headless;
    #[cfg(feature = "gui")]
    if !headless {
        println!("Starting Slint control panel...");
        if let Err(e) = launch_gui(Arc::clone(&state), Rc::clone(&source)) {
            eprintln!("[Warning] GUI initialization failed ({e}). Falling back to headless terminal mode.");
            run_headless_loop = true;
        }
    }

    if run_headless_loop {
        println!("Running in headless mode. Monitoring stream... Press Enter to stop.");
        let (stdin_tx, stdin_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut input = String::new();
            let _ = std::io::stdin().read_line(&mut input);
            let _ = stdin_tx.send(());
        });

        loop {
            if shizune::domain::state::TERMINATE_SIGNAL.load(Ordering::Acquire)
                || stdin_rx.try_recv().is_ok()
            {
                break;
            }
            if let Ok(mut src) = source.try_borrow_mut()
                && !src.is_active()
            {
                match src.reconnect_if_needed() {
                    Ok(true) => println!("[Auto-Reconnect] Microphone reconnected successfully."),
                    Ok(false) => {}
                    Err(e) => eprintln!("[Auto-Reconnect] Failed to reconnect microphone: {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    println!("Shutting down Shizune...");
    stop_signal.store(true, Ordering::Relaxed);
    if let Ok(mut src) = source.try_borrow_mut() {
        let _ = src.stop();
    }
    let _ = sink.stop();
    let _ = engine_handle.join();
    println!("Clean shutdown complete.");

    Ok(())
}
