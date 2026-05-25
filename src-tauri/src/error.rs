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

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error")]
    Sqlite(#[source] rusqlite::Error),

    #[error("io error")]
    Io(#[source] std::io::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("could not determine platform data directory")]
    DataDirUnavailable,

    #[error("row not found")]
    NotFound,

    #[error("malformed row payload")]
    Malformed(#[source] serde_json::Error),
}

impl From<rusqlite::Error> for DbError {
    fn from(value: rusqlite::Error) -> Self {
        match value {
            rusqlite::Error::QueryReturnedNoRows => DbError::NotFound,
            other => DbError::Sqlite(other),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),

    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("gmail api: {0}")]
    Gmail(#[source] reqwest::Error),

    #[error("gmail api returned http {status}: {body}")]
    GmailStatus { status: u16, body: String },

    #[error("malformed gmail response")]
    GmailMalformed(#[source] serde_json::Error),

    #[error("sync was cancelled")]
    Cancelled,
}

pub type SyncResult<T> = Result<T, SyncError>;
