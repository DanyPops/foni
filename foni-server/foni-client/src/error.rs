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
    pub(crate) fn request(e: reqwest::Error) -> Self {
        Self::Request(e.to_string())
    }

    pub(crate) fn decode(e: reqwest::Error) -> Self {
        Self::Decode(e.to_string())
    }
}
