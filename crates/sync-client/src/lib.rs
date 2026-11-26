//! Shared end-to-end encrypted Ottero Cloud sync client and engine.
//!
//! Used by both Desktop (`ottero-desktop`) and Mobile (`ottero-mobile-ffi`) so
//! wire envelopes, per-field LWW conflict resolution, and vault-meta sync stay
//! 100% identical across macOS, Windows, Linux, Android, and iOS.

pub mod client;
pub mod engine;
pub mod record;

pub use client::{DEFAULT_SYNC_URL, SyncClient, SyncClientError, resolve_sync_url};
pub use engine::{
    DecryptedChange, SyncCycleReport, ensure_collection_keys, sync_collection_keys, sync_once,
    sync_once_with_apply,
};
pub use record::{
    SyncEventPayload, SyncHabitPayload, SyncNotePayload, SyncProjectPayload, SyncTaskPayload,
    SyncVaultMetaPayload, SyncVaultPayload, VAULT_META_SYNC_ID, apply_decrypted_change,
    apply_decrypted_page, enqueue_full_snapshot, enqueue_local_change,
};
