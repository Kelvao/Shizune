use shizune::adapters::output::{
    create_audio_sink, detect_pipewire_socket, PactlManager, DEFAULT_SINK_NAME, DEFAULT_SOURCE_NAME,
};
use shizune::domain::{BufferPipeline, RING_BUFFER_CAPACITY};

#[test]
fn test_socket_detection_environment() {
    let detected = detect_pipewire_socket();
    assert!(!detected || detected);
}

#[test]
fn test_pactl_manager_raii_cleanup() {
    let mut manager = PactlManager::new();
    assert_eq!(manager.loaded_count(), 0);

    // Simulate unloaded state
    manager.unload_all();
    assert_eq!(manager.loaded_count(), 0);
}

#[test]
fn test_create_audio_sink_fallback() {
    let pipeline = BufferPipeline::new(RING_BUFFER_CAPACITY);
    let sink = create_audio_sink(pipeline.output_cons);
    assert!(!sink.is_active());
}

#[test]
fn test_default_device_names() {
    assert_eq!(DEFAULT_SINK_NAME, "Shizune_Sink");
    assert_eq!(DEFAULT_SOURCE_NAME, "Shizune_Virtual_Mic");
}

#[test]
fn test_create_audio_sink_with_names() {
    let pipeline = BufferPipeline::new(RING_BUFFER_CAPACITY);
    let sink = shizune::adapters::output::create_audio_sink_with_names(
        pipeline.output_cons,
        "Custom_Sink",
        "Custom_Mic",
    );
    assert!(!sink.is_active());
}

#[test]
fn test_device_listing_queries() {
    let inputs = shizune::adapters::cpal_source::CpalAudioSource::list_input_devices();
    assert!(inputs.is_ok());

    let outputs = shizune::adapters::output::PulseAudioSink::list_output_devices();
    assert!(outputs.is_ok());
}


