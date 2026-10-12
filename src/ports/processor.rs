use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessError {
    InvalidBufferSize { expected: usize, received: usize },
    InferenceFailed,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBufferSize { expected, received } => {
                write!(
                    f,
                    "Invalid buffer size: expected {expected}, received {received}"
                )
            }
            Self::InferenceFailed => write!(f, "Audio model inference failed"),
        }
    }
}

impl std::error::Error for ProcessError {}

/// Contract for frame-based audio processors.
pub trait AudioProcessor: Send {
    fn process_frame(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        atten_lim_db: f32,
    ) -> Result<(), ProcessError>;

    fn frame_size(&self) -> usize;
}
