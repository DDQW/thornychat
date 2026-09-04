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

impl CoreError {
    /// Whether this is the local sqlite store refusing to open — the only
    /// failure that throwing the session away can actually fix.
    ///
    /// Deliberately a whitelist. The session and crypto store are the most
    /// destructive things this app can delete, so anything not positively
    /// identified as a store fault must be left alone: a transient network
    /// error at startup once cost a full re-login (and that device's E2EE
    /// identity) because it was assumed to be one.
    pub fn is_unusable_store(&self) -> bool {
        matches!(
            self,
            CoreError::ClientBuild(error)
                if matches!(**error, matrix_sdk::ClientBuildError::SqliteStore(_))
        )
    }

    /// Whether this is the network or the homeserver being unreachable, so
    /// nothing is wrong locally and the same call may well succeed shortly.
    pub fn is_transient_transport(&self) -> bool {
        use matrix_sdk::{ClientBuildError, HttpError};

        fn http_is_transport(error: &HttpError) -> bool {
            match error {
                HttpError::Reqwest(_) => true,
                HttpError::Cached(inner) => http_is_transport(inner),
                _ => false,
            }
        }

        match self {
            CoreError::ClientBuild(error) => match &**error {
                ClientBuildError::Http(http) => http_is_transport(http),
                // Reaching .well-known failed. We pass a full homeserver URL so
                // this shouldn't fire, but if it does it's a network problem,
                // not a local one.
                ClientBuildError::AutoDiscovery(_) => true,
                _ => false,
            },
            CoreError::Sdk(matrix_sdk::Error::Http(http)) => http_is_transport(http),
            _ => false,
        }
    }
}

impl From<matrix_sdk::ClientBuildError> for CoreError {
    fn from(e: matrix_sdk::ClientBuildError) -> Self {
        Self::ClientBuild(Box::new(e))
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ClientBuildError;

    fn build(error: ClientBuildError) -> CoreError {
        CoreError::ClientBuild(Box::new(error))
    }

    #[test]
    fn only_a_sqlite_failure_counts_as_an_unusable_store() {
        // The whole point of the whitelist: discarding the session and the
        // crypto store is destructive, so only this may trigger it.
        let store = build(ClientBuildError::SqliteStore(
            matrix_sdk_sqlite::OpenStoreError::MissingVersion,
        ));
        assert!(store.is_unusable_store());
        assert!(!store.is_transient_transport());
    }

    #[test]
    fn non_store_build_failures_never_discard_the_session() {
        // This is the 2026-08-24 regression: a launch-time failure that isn't
        // the store must not be allowed to delete the session. Anything not
        // positively identified as a sqlite fault has to answer `false` here.
        for error in [
            build(ClientBuildError::MissingHomeserver),
            build(ClientBuildError::InvalidServerName),
            build(ClientBuildError::Url(url::Url::parse("nonsense").unwrap_err())),
            CoreError::Other("something went wrong".into()),
            CoreError::Io(std::io::Error::other("disk hiccup")),
            CoreError::NoSavedSession,
        ] {
            assert!(!error.is_unusable_store(), "{error} must not discard the session");
        }
    }

    #[test]
    fn configuration_mistakes_are_not_worth_retrying() {
        // Retrying a bad URL forever would just hide the real problem.
        assert!(!build(ClientBuildError::MissingHomeserver).is_transient_transport());
        assert!(!build(ClientBuildError::InvalidServerName).is_transient_transport());
        assert!(!build(ClientBuildError::Url(url::Url::parse("nonsense").unwrap_err()))
            .is_transient_transport());
    }

    #[test]
    fn a_failed_well_known_lookup_is_transient() {
        // We pass a full homeserver URL so this shouldn't normally fire, but
        // if it does it's the network, not our state.
        let error = build(ClientBuildError::AutoDiscovery(
            matrix_sdk::ruma::api::error::FromHttpResponseError::Deserialization(
                matrix_sdk::ruma::api::error::DeserializationError::Json(
                    serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
                ),
            ),
        ));
        assert!(error.is_transient_transport());
        assert!(!error.is_unusable_store());
    }
}
