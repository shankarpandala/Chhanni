use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("keychain operation failed")]
    Keychain(#[source] keyring::Error),

    #[error("oauth flow failed: {reason}")]
    OAuth { reason: String },

    #[error("oauth callback timed out after {seconds}s")]
    CallbackTimeout { seconds: u64 },

    #[error("oauth callback returned state mismatch")]
    StateMismatch,

    #[error("oauth callback returned provider error: {0}")]
    ProviderError(String),

    #[error("token refresh failed")]
    Refresh(#[source] reqwest::Error),

    #[error("malformed token payload")]
    MalformedToken(#[source] serde_json::Error),

    #[error("loopback listener failed to bind")]
    LoopbackBind(#[source] std::io::Error),

    #[error("loopback listener I/O error")]
    LoopbackIo(#[source] std::io::Error),

    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("missing required env var: {0}")]
    MissingEnv(&'static str),
}

impl From<keyring::Error> for AuthError {
    fn from(value: keyring::Error) -> Self {
        AuthError::Keychain(value)
    }
}

pub type AuthResult<T> = Result<T, AuthError>;
