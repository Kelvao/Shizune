use df::{Complex32, DFState};

use crate::domain::config::FRAME_SIZE;
use crate::ports::processor::{AudioProcessor, ProcessError};

/// Audio processor wrapping DeepFilterNet DSP routines.
pub struct DeepFilterProcessor {
    df_state: DFState,
    spec_scratch: Vec<Complex32>,
}

impl Default for DeepFilterProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DeepFilterProcessor {
    pub fn new() -> Self {
        let df_state = DFState::default();
        let freq_size = df_state.freq_size;
        let spec_scratch = vec![Complex32::default(); freq_size];

        Self {
            df_state,
            spec_scratch,
        }
    }

    /// Converts an attenuation limit in dB into a linear minimum gain factor.
    #[inline]
    pub fn atten_lim_to_linear_gain(atten_lim_db: f32) -> f32 {
        if atten_lim_db <= 0.01 {
            1.0
        } else if atten_lim_db >= 100.0 {
            0.0
        } else {
            10.0f32.powf(-atten_lim_db / 20.0)
        }
    }
}

impl AudioProcessor for DeepFilterProcessor {
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

        self.df_state.analysis(input, &mut self.spec_scratch);

        let min_gain = Self::atten_lim_to_linear_gain(atten_lim_db);
        for bin in self.spec_scratch.iter_mut() {
            *bin *= min_gain;
        }

        self.df_state.synthesis(&mut self.spec_scratch, output);
        Ok(())
    }

    #[inline]
    fn frame_size(&self) -> usize {
        FRAME_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atten_lim_gain_conversion() {
        assert_eq!(DeepFilterProcessor::atten_lim_to_linear_gain(0.0), 1.0);
        assert_eq!(DeepFilterProcessor::atten_lim_to_linear_gain(100.0), 0.0);

        let g_20db = DeepFilterProcessor::atten_lim_to_linear_gain(20.0);
        assert!((g_20db - 0.1).abs() < 1e-4);
    }

    #[test]
    fn test_deep_filter_process_frame_in_out() {
        let mut processor = DeepFilterProcessor::new();
        assert_eq!(processor.frame_size(), FRAME_SIZE);

        let input = vec![0.5f32; FRAME_SIZE];
        let mut output = vec![0.0f32; FRAME_SIZE];

        let res = processor.process_frame(&input, &mut output, 0.0);
        assert!(res.is_ok());
        assert_eq!(output.len(), FRAME_SIZE);
    }

    #[test]
    fn test_deep_filter_maximum_attenuation_silences() {
        let mut processor = DeepFilterProcessor::new();
        let input = vec![0.8f32; FRAME_SIZE];
        let mut output = vec![1.0f32; FRAME_SIZE];

        let res = processor.process_frame(&input, &mut output, 100.0);
        assert!(res.is_ok());
        for &sample in &output {
            assert!(sample.abs() < 1e-6, "Sample should be attenuated to silence: {sample}");
        }
    }
}
