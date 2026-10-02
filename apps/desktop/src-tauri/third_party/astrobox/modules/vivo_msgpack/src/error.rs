use std::num::TryFromIntError;

#[derive(Debug, thiserror::Error)]
pub enum MsgpackError {
    #[error("messagepack decode/encode error: {0}")]
    Msgpack(&'static str),
    #[error("messagepack UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("schema decode error: {0}")]
    Schema(#[from] serde_json::Error),
    #[error("integer conversion failed: {0}")]
    IntConversion(#[from] TryFromIntError),
    #[error("vivo response code is not success: {0}")]
    ResponseCode(i32),
}

pub type Result<T> = std::result::Result<T, MsgpackError>;
