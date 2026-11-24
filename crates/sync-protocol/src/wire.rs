//! Versioned push, pull, quota, error, and WebSocket wire messages.

use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::collection::validate_ident;
use crate::{Collection, Hlc, ProtocolError, ProtocolResult};

/// Wire protocol version spoken by this crate.
pub const PROTOCOL_VERSION: u16 = 1;

/// Maximum decoded ciphertext size for a single synced item change (1 MiB).
pub const MAX_BLOB_BYTES: usize = 1024 * 1024;

/// Maximum number of changes allowed in a single push batch.
pub const MAX_PUSH_BATCH: usize = 200;

/// Maximum combined decoded ciphertext bytes in a single push batch (8 MiB).
pub const MAX_PUSH_TOTAL_BYTES: usize = 8 * 1024 * 1024;

/// Default and maximum number of changes returned by one pull page.
pub const DEFAULT_PULL_LIMIT: u32 = 500;
pub const MAX_PULL_LIMIT: u32 = 1000;

/// Days deleted-item tombstones are retained before a device must full-resync.
pub const TOMBSTONE_RETENTION_DAYS: u32 = 90;

/// Client-generated UUID v7 identifying one push change for idempotent retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChangeId(pub Uuid);

impl ChangeId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for ChangeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes ciphertext bytes for JSON transport (base64url without padding).
#[must_use]
pub fn encode_blob(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes and size-checks a base64url ciphertext blob.
pub fn decode_blob(blob_b64: &str) -> ProtocolResult<Vec<u8>> {
    // Fast upper-bound reject before allocating.
    let max_encoded = MAX_BLOB_BYTES.div_ceil(3) * 4;
    if blob_b64.is_empty() || blob_b64.len() > max_encoded {
        return Err(ProtocolError::InvalidBlobSize {
            max: MAX_BLOB_BYTES,
        });
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(blob_b64)
        .map_err(|_| ProtocolError::InvalidBlobEncoding)?;
    if decoded.is_empty() || decoded.len() > MAX_BLOB_BYTES {
        return Err(ProtocolError::InvalidBlobSize {
            max: MAX_BLOB_BYTES,
        });
    }
    Ok(decoded)
}

/// One encrypted change uploaded by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushChange {
    pub change_id: ChangeId,
    pub collection: Collection,
    pub item_id: String,
    pub hlc: Hlc,
    pub schema_version: u16,
    pub deleted: bool,
    /// Base64url (no pad) XChaCha20-Poly1305 ciphertext sealed with `item_aad`.
    pub blob_b64: String,
}

impl PushChange {
    pub fn validate(&self) -> ProtocolResult<usize> {
        validate_ident(&self.item_id, "item_id", 128)?;
        self.hlc.validate()?;
        let decoded = decode_blob(&self.blob_b64)?;
        Ok(decoded.len())
    }
}

/// Batch push request from client to sync server (`POST /v1/sync/push`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub protocol: u16,
    pub device_id: String,
    pub changes: Vec<PushChange>,
}

impl PushRequest {
    pub fn validate(&self) -> ProtocolResult<usize> {
        if self.protocol != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedProtocol(self.protocol));
        }
        validate_ident(&self.device_id, "device_id", 64)?;
        if self.changes.is_empty() || self.changes.len() > MAX_PUSH_BATCH {
            return Err(ProtocolError::InvalidBatchSize {
                max: MAX_PUSH_BATCH,
            });
        }
        let mut seen = HashSet::with_capacity(self.changes.len());
        let mut total_bytes: usize = 0;
        for change in &self.changes {
            if !seen.insert(change.change_id) {
                return Err(ProtocolError::DuplicateChangeId);
            }
            let len = change.validate()?;
            total_bytes = total_bytes.saturating_add(len);
            if total_bytes > MAX_PUSH_TOTAL_BYTES {
                return Err(ProtocolError::BatchTooLarge {
                    max: MAX_PUSH_TOTAL_BYTES,
                });
            }
        }
        Ok(total_bytes)
    }
}

/// Per-change acknowledgement returned by the sync server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushAccepted {
    pub change_id: ChangeId,
    pub seq: i64,
    /// True when the server already had `change_id` (idempotent retry).
    pub duplicate: bool,
}

/// Response to `POST /v1/sync/push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResponse {
    pub accepted: Vec<PushAccepted>,
    pub latest_seq: i64,
    pub quota: QuotaUsage,
}

/// Pull request from client to sync server (`GET /v1/sync/pull`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub protocol: u16,
    pub after_seq: i64,
    pub limit: u32,
}

impl PullRequest {
    pub fn validate(&self) -> ProtocolResult<()> {
        if self.protocol != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedProtocol(self.protocol));
        }
        if self.after_seq < 0 {
            return Err(ProtocolError::InvalidCursor);
        }
        if self.limit == 0 || self.limit > MAX_PULL_LIMIT {
            return Err(ProtocolError::InvalidPullLimit {
                max: MAX_PULL_LIMIT,
            });
        }
        Ok(())
    }
}

/// One encrypted change returned by the sync server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteChange {
    pub seq: i64,
    pub change_id: ChangeId,
    pub device_id: String,
    pub collection: Collection,
    pub item_id: String,
    pub hlc: Hlc,
    pub schema_version: u16,
    pub deleted: bool,
    pub blob_b64: String,
    pub created_at_ms: i64,
}

/// Response to `GET /v1/sync/pull`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullResponse {
    pub changes: Vec<RemoteChange>,
    pub next_seq: i64,
    pub has_more: bool,
    /// Lowest sequence still available without requiring a full resync.
    pub oldest_available_seq: i64,
    pub quota: QuotaUsage,
}

/// Storage quota usage reported by the sync server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaUsage {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

impl QuotaUsage {
    #[must_use]
    pub fn is_over_quota(self) -> bool {
        self.limit_bytes > 0 && self.used_bytes >= self.limit_bytes
    }

    #[must_use]
    pub fn percent_used(self) -> u8 {
        if self.limit_bytes == 0 {
            return 0;
        }
        let pct = self.used_bytes.saturating_mul(100) / self.limit_bytes;
        u8::try_from(pct.min(100)).unwrap_or(100)
    }
}

/// Machine-readable sync server error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncErrorCode {
    Unauthorized,
    EntitlementRequired,
    /// Client cursor predates tombstone retention (HTTP 410); client must full-resync.
    CursorExpired,
    QuotaExceeded,
    PayloadTooLarge,
    ProtocolMismatch,
    RateLimited,
    ValidationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncErrorResponse {
    pub code: SyncErrorCode,
    pub message: String,
}

/// Real-time WebSocket notices sent from server to connected devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    SeqAdvanced { latest_seq: i64 },
    DeviceRevoked,
    EntitlementChanged { active: bool },
    Pong,
}

/// Real-time WebSocket messages sent from client to server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    Ping,
    Subscribe { after_seq: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_change(id: ChangeId) -> PushChange {
        PushChange {
            change_id: id,
            collection: Collection::Tasks,
            item_id: "task-1".into(),
            hlc: Hlc::new(1_760_000_000_000, 0, "dev-1").expect("hlc"),
            schema_version: 1,
            deleted: false,
            blob_b64: encode_blob(b"ciphertext-bytes"),
        }
    }

    #[test]
    fn push_request_validates_limits_and_duplicates() {
        let id = ChangeId::new();
        let req = PushRequest {
            protocol: PROTOCOL_VERSION,
            device_id: "dev-1".into(),
            changes: vec![sample_change(id)],
        };
        assert_eq!(req.validate(), Ok(b"ciphertext-bytes".len()));

        // Duplicate change_id rejected.
        let dup = PushRequest {
            protocol: PROTOCOL_VERSION,
            device_id: "dev-1".into(),
            changes: vec![sample_change(id), sample_change(id)],
        };
        assert_eq!(dup.validate(), Err(ProtocolError::DuplicateChangeId));

        // Empty batch rejected.
        let empty = PushRequest {
            protocol: PROTOCOL_VERSION,
            device_id: "dev-1".into(),
            changes: vec![],
        };
        assert_eq!(
            empty.validate(),
            Err(ProtocolError::InvalidBatchSize {
                max: MAX_PUSH_BATCH
            })
        );

        // Wrong protocol version rejected.
        let bad_proto = PushRequest {
            protocol: 99,
            device_id: "dev-1".into(),
            changes: vec![sample_change(ChangeId::new())],
        };
        assert_eq!(
            bad_proto.validate(),
            Err(ProtocolError::UnsupportedProtocol(99))
        );
    }

    #[test]
    fn blob_round_trips_and_enforces_size_limit() {
        let raw = vec![0xAB; 128];
        let encoded = encode_blob(&raw);
        assert_eq!(decode_blob(&encoded), Ok(raw));

        assert_eq!(
            decode_blob(""),
            Err(ProtocolError::InvalidBlobSize {
                max: MAX_BLOB_BYTES
            })
        );
        let too_big = encode_blob(&vec![0u8; MAX_BLOB_BYTES + 1]);
        assert_eq!(
            decode_blob(&too_big),
            Err(ProtocolError::InvalidBlobSize {
                max: MAX_BLOB_BYTES
            })
        );
    }

    #[test]
    fn pull_request_and_ws_messages_round_trip_json() {
        let pull = PullRequest {
            protocol: PROTOCOL_VERSION,
            after_seq: 42,
            limit: DEFAULT_PULL_LIMIT,
        };
        assert_eq!(pull.validate(), Ok(()));
        let json = serde_json::to_string(&pull).expect("json");
        let back: PullRequest = serde_json::from_str(&json).expect("back");
        assert_eq!(back, pull);

        let ws = WsServerMessage::SeqAdvanced { latest_seq: 99 };
        let ws_json = serde_json::to_string(&ws).expect("ws json");
        assert!(ws_json.contains("seq_advanced"));
        assert_eq!(
            serde_json::from_str::<WsServerMessage>(&ws_json).expect("ws"),
            ws
        );

        let quota = QuotaUsage {
            used_bytes: 500,
            limit_bytes: 1000,
        };
        assert_eq!(quota.percent_used(), 50);
        assert!(!quota.is_over_quota());
    }
}
