//! Error types for `flag-kit`.

/// Errors that can occur in flag operations.
#[derive(Debug, thiserror::Error)]
pub enum FlagError {
    /// Flag not found.
    #[error("flag not found: {0}")]
    NotFound(String),

    /// Flag already exists.
    #[error("flag already exists: {0}")]
    AlreadyExists(String),

    /// Invalid flag name.
    #[error("invalid flag name: {name:?}: {reason}")]
    InvalidName {
        /// The supplied name.
        name: String,
        /// Reason for invalidity.
        reason: String,
    },

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(String),
}

/// Convenience result type for flag operations.
pub type Result<T> = core::result::Result<T, FlagError>;

impl FlagError {
    /// Create an `InvalidName` error.
    pub fn invalid_name(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidName {
            name: name.into(),
            reason: reason.into(),
        }
    }

    /// Create a `NotFound` error.
    pub fn not_found(name: impl Into<String>) -> Self {
        Self::NotFound(name.into())
    }

    /// Create an `AlreadyExists` error.
    pub fn already_exists(name: impl Into<String>) -> Self {
        Self::AlreadyExists(name.into())
    }

    /// Create a `Storage` error.
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage(msg.into())
    }
}

impl From<serde_json::Error> for FlagError {
    fn from(e: serde_json::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for FlagError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for FlagError {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
