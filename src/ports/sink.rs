use std::fmt;

#[derive(Debug)]
pub enum SinkError {
    ServerUnavailable(String),
    ConfigurationError(String),
    StartError(String),
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServerUnavailable(msg) => write!(f, "Audio server unavailable: {msg}"),
            Self::ConfigurationError(msg) => write!(f, "Output configuration error: {msg}"),
            Self::StartError(msg) => write!(f, "Failed to start audio output: {msg}"),
        }
    }
}

impl std::error::Error for SinkError {}

/// Contract for virtual nodes or physical audio sinks.
pub trait AudioSink {
    fn start(&mut self) -> Result<(), SinkError>;
    fn stop(&mut self) -> Result<(), SinkError>;
    fn is_active(&self) -> bool;
}
