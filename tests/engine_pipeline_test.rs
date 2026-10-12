use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ringbuf::traits::{Consumer, Observer, Producer};
use shizune::domain::{
    AudioEngine, BufferPipeline, DeepFilterProcessor, EngineState, FRAME_SIZE, RING_BUFFER_CAPACITY,
};

#[test]
fn test_end_to_end_engine_pipeline_threaded() {
    let pipeline = BufferPipeline::new(RING_BUFFER_CAPACITY);
    let mut input_prod = pipeline.input_prod;
    let mut output_cons = pipeline.output_cons;
    let state = EngineState::new_shared();
    let stop_signal = Arc::new(AtomicBool::new(false));

    let processor = DeepFilterProcessor::new();
    let engine = AudioEngine::new(
        pipeline.input_cons,
        pipeline.output_prod,
        Arc::clone(&state),
        processor,
    );
    let engine_handle = engine.spawn(Arc::clone(&stop_signal));

    let frame_count = 3;
    let test_frame = [0.25f32; FRAME_SIZE];
    for _ in 0..frame_count {
        assert_eq!(input_prod.push_slice(&test_frame), FRAME_SIZE);
    }

    let mut total_received = 0;
    let mut received_buf = [0.0f32; FRAME_SIZE];
    let start = std::time::Instant::now();

    while total_received < frame_count * FRAME_SIZE && start.elapsed() < Duration::from_millis(500) {
        if output_cons.occupied_len() >= FRAME_SIZE {
            let popped = output_cons.pop_slice(&mut received_buf);
            total_received += popped;
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }

    assert_eq!(total_received, frame_count * FRAME_SIZE);

    state.attenuation_limit.set(20.0);
    assert_eq!(state.attenuation_limit.get(), 20.0);

    assert_eq!(input_prod.push_slice(&test_frame), FRAME_SIZE);

    let start = std::time::Instant::now();
    let mut received_new = 0;
    while received_new < FRAME_SIZE && start.elapsed() < Duration::from_millis(500) {
        if output_cons.occupied_len() >= FRAME_SIZE {
            received_new += output_cons.pop_slice(&mut received_buf);
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(received_new, FRAME_SIZE);

    stop_signal.store(true, Ordering::Relaxed);
    engine_handle.join().expect("Engine thread failed to join");
}
