pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 1;
pub const FRAME_SIZE: usize = 480;
pub const RING_BUFFER_CAPACITY: usize = 4_800;
pub const MAX_LATENCY_MS: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_size: usize,
    pub ring_buffer_capacity: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            frame_size: FRAME_SIZE,
            ring_buffer_capacity: RING_BUFFER_CAPACITY,
        }
    }
}

impl AudioConfig {
    #[inline]
    pub const fn frame_duration_ms(&self) -> f32 {
        (self.frame_size as f32 / self.sample_rate as f32) * 1000.0
    }

    #[inline]
    pub const fn buffer_duration_ms(&self) -> f32 {
        (self.ring_buffer_capacity as f32 / self.sample_rate as f32) * 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_audio_config() {
        let config = AudioConfig::default();
        assert_eq!(config.sample_rate, 48_000);
        assert_eq!(config.channels, 1);
        assert_eq!(config.frame_size, 480);
        assert_eq!(config.ring_buffer_capacity, 4_800);
    }

    #[test]
    fn test_frame_and_buffer_durations() {
        let config = AudioConfig::default();
        let frame_ms = config.frame_duration_ms();
        let buffer_ms = config.buffer_duration_ms();

        assert!((frame_ms - 10.0).abs() < f32::EPSILON);
        assert!((buffer_ms - 100.0).abs() < f32::EPSILON);
        assert!(frame_ms < MAX_LATENCY_MS);
    }
}
