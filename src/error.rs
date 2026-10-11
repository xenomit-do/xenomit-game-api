use thiserror::Error;

/// Errors returned by a game client implementation.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("game client transport failed: {0}")]
    Transport(String),
    #[error("game client is unavailable on this platform: {0}")]
    UnsupportedPlatform(String),
    #[error("game client returned an invalid response: {0}")]
    Protocol(String),
    #[error("game action failed: {message}{stack_suffix}", stack_suffix = stack.as_deref().map(|s| format!("\n{s}")).unwrap_or_default())]
    ActionFailed {
        message: String,
        stack: Option<String>,
    },
    #[error("invalid function call: {0}")]
    InvalidCall(String),
    #[error("game state is currently unavailable: {0}")]
    Unavailable(String),
    #[error("operation is not supported by this backend: {0}")]
    Unsupported(String),
    #[error("bridge response exceeded the {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
}

pub type Result<T> = std::result::Result<T, ApiError>;
