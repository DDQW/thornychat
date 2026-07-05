use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("matrix SDK error: {0}")]
    Sdk(#[from] matrix_sdk::Error),

    // Boxed: ClientBuildError is ~160 bytes and would dominate the size of
    // every CoreResult return (clippy::result_large_err).
    #[error("client build error: {0}")]
    ClientBuild(#[source] Box<matrix_sdk::ClientBuildError>),

    #[error("no saved session found")]
    NoSavedSession,

    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("invalid homeserver url: {0}")]
    InvalidHomeserver(#[from] url::ParseError),

    #[error("login failed: {0}")]
    LoginFailed(String),

    #[error("sso login was cancelled or timed out")]
    SsoCancelled,

    #[error("room not found: {0}")]
    RoomNotFound(String),

    #[error("{0}")]
    Other(String),
}

impl From<matrix_sdk::ClientBuildError> for CoreError {
    fn from(e: matrix_sdk::ClientBuildError) -> Self {
        Self::ClientBuild(Box::new(e))
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
