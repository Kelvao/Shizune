use std::sync::Arc;
use std::thread;

use shizune::domain::{AudioConfig, EngineState, FRAME_SIZE, MAX_ATTENUATION_DB, MIN_ATTENUATION_DB};
use shizune::ports::processor::{AudioProcessor, ProcessError};

struct MockAudioProcessor {
    last_atten_db: f32,
    processed_count: usize,
}

impl MockAudioProcessor {
    fn new() -> Self {
        Self {
            last_atten_db: 0.0,
            processed_count: 0,
        }
    }
}

impl AudioProcessor for MockAudioProcessor {
    fn process_frame(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        atten_lim_db: f32,
    ) -> Result<(), ProcessError> {
        if input.len() != self.frame_size() || output.len() != self.frame_size() {
            return Err(ProcessError::InvalidBufferSize {
                expected: self.frame_size(),
                received: input.len(),
            });
        }

        self.last_atten_db = atten_lim_db;
        self.processed_count += 1;

        let factor = 1.0 - (atten_lim_db / 100.0) * 0.5;
        for (i, sample) in input.iter().enumerate() {
            output[i] = sample * factor;
        }

        Ok(())
    }

    fn frame_size(&self) -> usize {
        FRAME_SIZE
    }
}

#[test]
fn test_audio_config_calculations() {
    let config = AudioConfig::default();
    assert_eq!(config.frame_size, 480);
    assert_eq!(config.sample_rate, 48_000);
    assert_eq!(config.frame_duration_ms(), 10.0);
    assert_eq!(config.buffer_duration_ms(), 100.0);
}

#[test]
fn test_mock_audio_processor_port_contract() {
    let mut processor = MockAudioProcessor::new();
    let input = vec![1.0f32; FRAME_SIZE];
    let mut output = vec![0.0f32; FRAME_SIZE];

    let res = processor.process_frame(&input, &mut output, 50.0);
    assert!(res.is_ok());
    assert_eq!(processor.last_atten_db, 50.0);
    assert_eq!(processor.processed_count, 1);
    assert!((output[0] - 0.75).abs() < f32::EPSILON);

    let invalid_input = vec![1.0f32; 100];
    let res = processor.process_frame(&invalid_input, &mut output, 50.0);
    assert!(matches!(
        res,
        Err(ProcessError::InvalidBufferSize {
            expected: 480,
            received: 100
        })
    ));
}

#[test]
fn test_multithreaded_lock_free_state_concurrency() {
    let state = EngineState::new_shared();

    let state_writer = Arc::clone(&state);
    let writer_handle = thread::spawn(move || {
        for db in 0..=100 {
            state_writer.attenuation_limit.set(db as f32);
        }
    });

    let state_reader = Arc::clone(&state);
    let reader_handle = thread::spawn(move || {
        let mut reads = Vec::new();
        for _ in 0..100 {
            let val = state_reader.attenuation_limit.get();
            reads.push(val);
        }
        reads
    });

    writer_handle.join().expect("Writer thread failed");
    let reads = reader_handle.join().expect("Reader thread failed");

    for val in reads {
        assert!((MIN_ATTENUATION_DB..=MAX_ATTENUATION_DB).contains(&val));
    }

    assert_eq!(state.attenuation_limit.get(), 100.0);
}
