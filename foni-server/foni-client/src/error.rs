//! `FoniError` — unified error type for all `FoniClient` operations.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FoniError {
    /// The server returned a non-2xx status code.
    #[error("HTTP {0}: {1}")]
    Http(u16, String),

    /// Failed to decode a response body (JSON parse or base64).
    #[error("decode error: {0}")]
    Decode(String),

    /// Network or I/O error sending the request.
    #[error("request error: {0}")]
    Request(String),
}

impl FoniError {
    // Takes by value to work as map_err(FoniError::request) function pointer.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn request(e: reqwest::Error) -> Self {
        Self::Request(e.to_string())
    }

    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn decode(e: reqwest::Error) -> Self {
        Self::Decode(e.to_string())
    }
}
