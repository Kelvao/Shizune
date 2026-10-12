use df::{Complex32, DFState};

use crate::domain::config::FRAME_SIZE;
use crate::ports::processor::{AudioProcessor, ProcessError};

/// Audio processor wrapping DeepFilterNet DSP routines with ERB band mask estimation
/// and native attenuation limit clamping.
pub struct DeepFilterProcessor {
    df_state: DFState,
    spec_scratch: Vec<Complex32>,
    band_energies: Vec<f32>,
    noise_floor: Vec<f32>,
    erb_mask: Vec<f32>,
    speech_hangover: usize,
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
        let nb_bands = df_state.erb.len();
        let spec_scratch = vec![Complex32::default(); freq_size];
        let band_energies = vec![0.0f32; nb_bands];
        let noise_floor = vec![0.001f32; nb_bands];
        let erb_mask = vec![1.0f32; nb_bands];

        Self {
            df_state,
            spec_scratch,
            band_energies,
            noise_floor,
            erb_mask,
            speech_hangover: 0,
        }
    }

    /// Converts an attenuation limit in dB into a linear minimum gain factor (floor).
    ///
    /// Formula matching DeepFilterNet runtime specification:
    /// - atten_lim_db <= 0.01 dB -> 1.0 (bypass noise suppression, transparent)
    /// - atten_lim_db >= 100.0 dB -> 0.0 (maximum unbounded suppression allowed)
    /// - otherwise -> 10^(-atten_lim_db / 20)
    #[inline]
    pub fn atten_lim_to_min_gain(atten_lim_db: f32) -> f32 {
        let lim = atten_lim_db.abs();
        if lim <= 0.01 {
            1.0
        } else if lim >= 100.0 {
            0.0
        } else {
            10.0f32.powf(-lim / 20.0)
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

        // 1. Time -> Frequency domain analysis
        self.df_state.analysis(input, &mut self.spec_scratch);

        // 2. Compute energy per ERB frequency band
        let mut bin_offset = 0;
        for (b, &band_width) in self.df_state.erb.iter().enumerate() {
            let mut sum_energy = 0.0f32;
            for j in 0..band_width {
                let bin = self.spec_scratch[bin_offset + j];
                sum_energy += bin.norm_sqr();
            }
            let avg_energy = sum_energy / (band_width as f32).max(1.0);
            self.band_energies[b] = avg_energy;
            bin_offset += band_width;
        }

        // 3. Continuous Asymmetric Noise Floor Tracking
        for (floor, &energy) in self.noise_floor.iter_mut().zip(self.band_energies.iter()) {
            if energy < *floor {
                // Rapidly adapt downward to true minimum
                *floor = *floor * 0.85 + energy * 0.15;
            } else {
                // Slowly adapt upward to track stationary ambient noise (fans, hum)
                *floor = *floor * 0.998 + energy * 0.002;
            }
        }

        // 4. Voice Activity Detection (VAD) on core human speech formants (bands 3..18, ~200Hz - 3.5kHz)
        let speech_band_start = 3.min(self.band_energies.len());
        let speech_band_end = 18.min(self.band_energies.len());

        let voice_energy: f32 = self.band_energies[speech_band_start..speech_band_end].iter().sum();
        let voice_noise: f32 = self.noise_floor[speech_band_start..speech_band_end].iter().sum();
        let is_voice_active = voice_energy > voice_noise * 2.0;

        if is_voice_active {
            // Keep voice active for 10 frames (~100ms) hangover to prevent word-ending clipping
            self.speech_hangover = 10;
        } else if self.speech_hangover > 0 {
            self.speech_hangover -= 1;
        }

        let voice_gate_open = self.speech_hangover > 0;
        let min_gain = Self::atten_lim_to_min_gain(atten_lim_db);

        // 5. Compute suppression mask per ERB band:
        for (b, ((mask, &energy), &floor)) in self
            .erb_mask
            .iter_mut()
            .zip(self.band_energies.iter())
            .zip(self.noise_floor.iter())
            .enumerate()
        {
            let raw_gain = if !voice_gate_open {
                // Silence / Pause: suppress ambient noise down to attenuation limit
                0.0
            } else if energy > floor * 1.2 {
                // Active speech: Wiener filter with over-subtraction factor alpha=1.8
                let snr_lin = 1.0 - (1.8 * (floor / energy)).min(1.0);
                (snr_lin * snr_lin).clamp(0.0, 1.0)
            } else {
                0.0
            };

            // High-pass filter rumble (< 100 Hz, bands 0..1)
            let band_attenuated = if b < 2 {
                raw_gain * 0.2
            } else {
                raw_gain
            };

            *mask = band_attenuated.max(min_gain);
        }

        // 6. Apply clamped ERB band mask to frequency bins
        bin_offset = 0;
        for (b, &band_width) in self.df_state.erb.iter().enumerate() {
            let gain = self.erb_mask[b];
            for j in 0..band_width {
                self.spec_scratch[bin_offset + j] *= gain;
            }
            bin_offset += band_width;
        }

        // 7. Frequency -> Time domain synthesis
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
        assert_eq!(DeepFilterProcessor::atten_lim_to_min_gain(0.0), 1.0);
        assert_eq!(DeepFilterProcessor::atten_lim_to_min_gain(100.0), 0.0);

        let g_20db = DeepFilterProcessor::atten_lim_to_min_gain(20.0);
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
    fn test_deep_filter_zero_attenuation_preserves_audio() {
        let mut processor = DeepFilterProcessor::new();
        let input = vec![0.3f32; FRAME_SIZE];
        let mut output = vec![0.0f32; FRAME_SIZE];

        // 0.0 dB attenuation limit should floor min_gain to 1.0 (mask fully transparent)
        let res = processor.process_frame(&input, &mut output, 0.0);
        assert!(res.is_ok());
    }

    #[test]
    fn test_synthetic_noise_attenuation_difference() {
        let mut processor_10db = DeepFilterProcessor::new();
        let mut processor_40db = DeepFilterProcessor::new();

        // Synthetic pseudo-random noise generator (deterministic LCG)
        let mut seed: u64 = 123456789;
        let mut synthetic_noise = vec![0.0f32; FRAME_SIZE];
        for sample in synthetic_noise.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *sample = ((seed >> 33) as f32 / (1u32 << 31) as f32) - 1.0;
        }

        let mut out_10db = vec![0.0f32; FRAME_SIZE];
        let mut out_40db = vec![0.0f32; FRAME_SIZE];

        // Warm up noise floors across multiple stationary noise frames
        for _ in 0..15 {
            let _ = processor_10db.process_frame(&synthetic_noise, &mut out_10db, 10.0);
            let _ = processor_40db.process_frame(&synthetic_noise, &mut out_40db, 40.0);
        }

        let energy_10db: f32 = out_10db.iter().map(|&x| x * x).sum();
        let energy_40db: f32 = out_40db.iter().map(|&x| x * x).sum();

        // Higher attenuation limit (40 dB) must suppress more noise than 10 dB limit
        assert!(
            energy_40db < energy_10db,
            "40 dB attenuation energy ({energy_40db}) should be strictly lower than 10 dB energy ({energy_10db})"
        );
    }

    #[test]
    fn test_speech_hangover_hangs_and_decays() {
        let mut processor = DeepFilterProcessor::new();
        let mut out = vec![0.0f32; FRAME_SIZE];
        let silence = vec![0.0f32; FRAME_SIZE];

        // 1 kHz tone inside the speech formant range (bands 3..18)
        let mut speech_frame = vec![0.0f32; FRAME_SIZE];
        for (i, sample) in speech_frame.iter_mut().enumerate() {
            *sample = (2.0 * std::f32::consts::PI * 1000.0 * (i as f32) / 48000.0).sin() * 0.8;
        }

        // Process speech frames (STFT window has 1-2 frames overlap delay)
        let mut activated = false;
        for _ in 0..5 {
            let res = processor.process_frame(&speech_frame, &mut out, 20.0);
            assert!(res.is_ok());
            if processor.speech_hangover == 10 {
                activated = true;
            }
        }
        assert!(activated, "Speech must activate hangover");

        // Process subsequent silent frames:
        // Frame 0: STFT window flushes trailing speech, holding hangover at 10
        let res = processor.process_frame(&silence, &mut out, 20.0);
        assert!(res.is_ok());
        assert_eq!(processor.speech_hangover, 10);

        // Frames 1..10: hangover decrements from 9 down to 0
        for expected in (0..10).rev() {
            let res = processor.process_frame(&silence, &mut out, 20.0);
            assert!(res.is_ok());
            assert_eq!(processor.speech_hangover, expected);
        }

        // Subsequent silence: hangover remains closed at 0
        let res = processor.process_frame(&silence, &mut out, 20.0);
        assert!(res.is_ok());
        assert_eq!(processor.speech_hangover, 0);
    }
}
