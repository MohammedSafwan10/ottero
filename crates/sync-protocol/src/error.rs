//! Protocol validation and HLC errors.

use thiserror::Error;

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("unsupported sync protocol version {0}")]
    UnsupportedProtocol(u16),

    #[error("invalid identifier for {0}")]
    InvalidIdentifier(&'static str),

    #[error("invalid HLC timestamp")]
    InvalidHlc,

    #[error("remote clock is too far ahead of local time")]
    ClockDrift,

    #[error("HLC logical counter overflow")]
    CounterOverflow,

    #[error("push batch must contain between 1 and {max} changes")]
    InvalidBatchSize { max: usize },

    #[error("duplicate change_id in push batch")]
    DuplicateChangeId,

    #[error("duplicate collection in put-keys request")]
    DuplicateCollection,

    #[error("invalid base64url ciphertext blob")]
    InvalidBlobEncoding,

    #[error("ciphertext blob must be between 1 and {max} bytes")]
    InvalidBlobSize { max: usize },

    #[error("push batch exceeds maximum total size of {max} bytes")]
    BatchTooLarge { max: usize },

    #[error("pull cursor must be non-negative")]
    InvalidCursor,

    #[error("pull limit must be between 1 and {max}")]
    InvalidPullLimit { max: u32 },
}
