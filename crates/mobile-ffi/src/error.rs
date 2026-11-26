//! User-safe error type bridged across UniFFI to Kotlin and Swift.

use ottero_core::DomainError;
use ottero_storage::StorageError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    #[error("{reason}")]
    InvalidInput { reason: String },
    #[error("{reason}")]
    NotFound { reason: String },
    #[error("{reason}")]
    Storage { reason: String },
    #[error("Internal lock error")]
    LockPoisoned,
}

impl From<DomainError> for MobileError {
    fn from(err: DomainError) -> Self {
        Self::InvalidInput {
            reason: err.to_string(),
        }
    }
}

impl From<StorageError> for MobileError {
    fn from(err: StorageError) -> Self {
        let reason = match err {
            StorageError::Sqlite(_) => "Could not access local database",
            StorageError::Migration(_) => "Database migration failed",
            StorageError::Io(_) => "Could not access local storage",
            StorageError::Corrupt(_) => "Stored data is invalid",
        };
        Self::Storage {
            reason: reason.to_owned(),
        }
    }
}
