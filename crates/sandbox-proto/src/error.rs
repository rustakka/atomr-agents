use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("frame too large: {0} bytes (max {max})", max = crate::frame::MAX_FRAME_BYTES)]
    FrameTooLarge(usize),
}
