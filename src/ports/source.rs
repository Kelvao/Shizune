use std::fmt;

#[derive(Debug)]
pub enum SourceError {
    DeviceNotFound,
    StreamConfigError(String),
    StartError(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound => write!(f, "Audio input device not found"),
            Self::StreamConfigError(msg) => write!(f, "Stream configuration error: {msg}"),
            Self::StartError(msg) => write!(f, "Failed to start audio stream: {msg}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Contract for physical or mock audio sources.
pub trait AudioSource {
    fn start(&mut self) -> Result<(), SourceError>;
    fn stop(&mut self) -> Result<(), SourceError>;
    fn is_active(&self) -> bool;
}
