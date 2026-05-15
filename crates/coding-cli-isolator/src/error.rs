use thiserror::Error;

#[derive(Debug, Error)]
pub enum IsolatorError {
    #[error("spawn failed: {0}")]
    Spawn(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pty error: {0}")]
    Pty(String),

    #[error("process already exited")]
    AlreadyExited,

    #[cfg(feature = "docker")]
    #[error("docker error: {0}")]
    Docker(String),

    #[error("not supported in this isolator: {0}")]
    Unsupported(&'static str),
}

#[cfg(feature = "docker")]
impl From<bollard::errors::Error> for IsolatorError {
    fn from(e: bollard::errors::Error) -> Self {
        IsolatorError::Docker(e.to_string())
    }
}
