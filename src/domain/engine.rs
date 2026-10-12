use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use std::time::Duration;

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::domain::config::RING_BUFFER_CAPACITY;
use crate::domain::state::EngineState;
use crate::ports::processor::AudioProcessor;

/// Ring buffer pipeline connecting capture, AI engine, and audio output.
pub struct BufferPipeline {
    pub input_prod: HeapProd<f32>,
    pub input_cons: HeapCons<f32>,
    pub output_prod: HeapProd<f32>,
    pub output_cons: HeapCons<f32>,
}

impl Default for BufferPipeline {
    fn default() -> Self {
        Self::new(RING_BUFFER_CAPACITY)
    }
}

impl BufferPipeline {
    pub fn new(capacity: usize) -> Self {
        let input_rb = HeapRb::<f32>::new(capacity);
        let (input_prod, input_cons) = input_rb.split();

        let output_rb = HeapRb::<f32>::new(capacity);
        let (output_prod, output_cons) = output_rb.split();

        Self {
            input_prod,
            input_cons,
            output_prod,
            output_cons,
        }
    }
}

/// Real-time audio engine running on a dedicated thread.
pub struct AudioEngine<P: AudioProcessor> {
    input_cons: HeapCons<f32>,
    output_prod: HeapProd<f32>,
    state: Arc<EngineState>,
    processor: P,
    frame_size: usize,
}

impl<P: AudioProcessor> AudioEngine<P> {
    pub fn new(
        input_cons: HeapCons<f32>,
        output_prod: HeapProd<f32>,
        state: Arc<EngineState>,
        processor: P,
    ) -> Self {
        let frame_size = processor.frame_size();
        Self {
            input_cons,
            output_prod,
            state,
            processor,
            frame_size,
        }
    }

    /// Processes a single frame if input has enough samples and output has room.
    #[inline]
    pub fn process_one_frame(&mut self, in_buf: &mut [f32], out_buf: &mut [f32]) -> bool {
        if self.input_cons.occupied_len() < self.frame_size {
            return false;
        }

        if self.output_prod.vacant_len() < self.frame_size {
            return false;
        }

        // Prevent creeping latency: if input buffer accumulates a large backlog (> 4 frames = 40ms),
        // skip older stale samples to bound real-time latency.
        let occupied = self.input_cons.occupied_len();
        if occupied > 4 * self.frame_size {
            let excess = occupied - (2 * self.frame_size);
            self.input_cons.skip(excess);
        }

        self.input_cons.pop_slice(&mut in_buf[..self.frame_size]);

        if self.state.is_enabled() {
            let atten_db = self.state.attenuation_limit.get();
            if let Err(err) = self.processor.process_frame(
                &in_buf[..self.frame_size],
                &mut out_buf[..self.frame_size],
                atten_db,
            ) {
                eprintln!("[AudioEngine] Inference error: {err}. Bypassing frame.");
                out_buf[..self.frame_size].copy_from_slice(&in_buf[..self.frame_size]);
            }
        } else {
            out_buf[..self.frame_size].copy_from_slice(&in_buf[..self.frame_size]);
        }

        let pushed = self.output_prod.push_slice(&out_buf[..self.frame_size]);
        debug_assert_eq!(pushed, self.frame_size);
        true
    }

    /// Spawns the audio engine loop on a dedicated thread with pre-allocated frame buffers.
    pub fn spawn(mut self, stop_signal: Arc<AtomicBool>) -> JoinHandle<()>
    where
        P: 'static,
    {
        thread::spawn(move || {
            let mut in_buf = vec![0.0f32; self.frame_size];
            let mut out_buf = vec![0.0f32; self.frame_size];

            while !stop_signal.load(Ordering::Relaxed) {
                if !self.process_one_frame(&mut in_buf, &mut out_buf) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::FRAME_SIZE;
    use crate::ports::processor::ProcessError;
    use ringbuf::traits::{Consumer, Producer};

    struct TestEchoProcessor;
    impl AudioProcessor for TestEchoProcessor {
        fn process_frame(
            &mut self,
            input: &[f32],
            output: &mut [f32],
            atten_lim_db: f32,
        ) -> Result<(), ProcessError> {
            let scale = if atten_lim_db >= 100.0 { 0.0 } else { 1.0 };
            for (i, &sample) in input.iter().enumerate() {
                output[i] = sample * scale;
            }
            Ok(())
        }

        fn frame_size(&self) -> usize {
            FRAME_SIZE
        }
    }

    #[test]
    fn test_buffer_pipeline_preallocation() {
        let pipeline = BufferPipeline::new(RING_BUFFER_CAPACITY);
        assert_eq!(pipeline.input_prod.capacity().get(), RING_BUFFER_CAPACITY);
        assert_eq!(pipeline.output_prod.capacity().get(), RING_BUFFER_CAPACITY);
    }

    #[test]
    fn test_audio_engine_frame_consumption_and_yield() {
        let pipeline = BufferPipeline::new(RING_BUFFER_CAPACITY);
        let mut input_prod = pipeline.input_prod;
        let mut output_cons = pipeline.output_cons;
        let state = EngineState::new_shared();

        let mut engine = AudioEngine::new(
            pipeline.input_cons,
            pipeline.output_prod,
            Arc::clone(&state),
            TestEchoProcessor,
        );

        let mut in_buf = [0.0f32; FRAME_SIZE];
        let mut out_buf = [0.0f32; FRAME_SIZE];

        assert!(!engine.process_one_frame(&mut in_buf, &mut out_buf));
        assert_eq!(output_cons.occupied_len(), 0);

        let sample_data = [0.8f32; FRAME_SIZE];
        assert_eq!(input_prod.push_slice(&sample_data), FRAME_SIZE);

        state.attenuation_limit.set(0.0);
        assert!(engine.process_one_frame(&mut in_buf, &mut out_buf));
        assert_eq!(output_cons.occupied_len(), FRAME_SIZE);

        let mut received = [0.0f32; FRAME_SIZE];
        assert_eq!(output_cons.pop_slice(&mut received), FRAME_SIZE);
        assert_eq!(received[0], 0.8f32);
    }

    #[test]
    fn test_audio_engine_holds_frame_when_output_buffer_full() {
        let pipeline = BufferPipeline::new(FRAME_SIZE); // capacity is exactly 1 frame
        let mut input_prod = pipeline.input_prod;
        let mut output_prod = pipeline.output_prod;
        let state = EngineState::new_shared();

        // Fill output buffer so vacant_len < FRAME_SIZE
        let dummy = [0.1f32; FRAME_SIZE];
        assert_eq!(output_prod.push_slice(&dummy), FRAME_SIZE);

        let mut engine = AudioEngine::new(
            pipeline.input_cons,
            output_prod,
            Arc::clone(&state),
            TestEchoProcessor,
        );

        // Put 1 frame in input
        assert_eq!(input_prod.push_slice(&dummy), FRAME_SIZE);

        let mut in_buf = [0.0f32; FRAME_SIZE];
        let mut out_buf = [0.0f32; FRAME_SIZE];

        // Should return false and not discard input frame
        assert!(!engine.process_one_frame(&mut in_buf, &mut out_buf));
        assert_eq!(engine.input_cons.occupied_len(), FRAME_SIZE);
    }
}
