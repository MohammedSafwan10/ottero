//! Shared, IO-free wire protocol for Ottero Cloud end-to-end encrypted sync.
//!
//! Used by both the desktop client and the Rust sync server (`sync.nexdark.com`)
//! so the open-source client documents the exact ciphertext envelopes, metadata,
//! and limits sent over the wire. Plaintext item contents never appear here.

pub mod collection;
pub mod error;
pub mod hlc;
pub mod wire;

pub use collection::{Collection, item_aad};
pub use error::{ProtocolError, ProtocolResult};
pub use hlc::{Hlc, HlcClock, MAX_CLOCK_DRIFT_MS};
pub use wire::{
    ChangeId, CollectionKeyWire, DEFAULT_PULL_LIMIT, KeysResponse, MAX_BLOB_BYTES, MAX_PULL_LIMIT,
    MAX_PUSH_BATCH, MAX_PUSH_TOTAL_BYTES, PROTOCOL_VERSION, PullRequest, PullResponse,
    PushAccepted, PushChange, PushRequest, PushResponse, PutKeysRequest, QuotaUsage, RemoteChange,
    SyncErrorCode, SyncErrorResponse, TOMBSTONE_RETENTION_DAYS, WsClientMessage, WsServerMessage,
    decode_blob, encode_blob,
};
