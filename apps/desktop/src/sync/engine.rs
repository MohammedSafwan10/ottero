//! End-to-end encrypted push/pull sync engine (IO-isolated, no GPUI dependency).

use std::collections::HashMap;

use ottero_core::Timestamp;
use ottero_crypto::sync::{
    open_sync_item, seal_sync_item, unwrap_collection_key, wrap_collection_key,
};
use ottero_crypto::{SecretKey, Zeroizing};
use ottero_storage::{Store, WrappedCollectionKey};
use ottero_sync_protocol::{
    ChangeId, Collection, CollectionKeyWire, DEFAULT_PULL_LIMIT, Hlc, HlcClock, MAX_PUSH_BATCH,
    MAX_PUSH_TOTAL_BYTES, PROTOCOL_VERSION, ProtocolError, PullRequest, PushChange, PushRequest,
    PutKeysRequest, QuotaUsage, decode_blob, encode_blob, item_aad,
};

use crate::sync::client::{SyncClient, SyncClientError};

/// A verified, decrypted remote change ready to apply to local storage.
#[derive(Clone, PartialEq, Eq)]
pub struct DecryptedChange {
    pub seq: i64,
    pub change_id: ChangeId,
    pub device_id: String,
    pub collection: Collection,
    pub item_id: String,
    pub hlc: Hlc,
    pub schema_version: u16,
    pub deleted: bool,
    pub payload_json: Zeroizing<String>,
}

impl std::fmt::Debug for DecryptedChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecryptedChange")
            .field("seq", &self.seq)
            .field("change_id", &self.change_id)
            .field("device_id", &self.device_id)
            .field("collection", &self.collection)
            .field("item_id", &self.item_id)
            .field("hlc", &self.hlc)
            .field("schema_version", &self.schema_version)
            .field("deleted", &self.deleted)
            .field("payload_json", &"<redacted>")
            .finish()
    }
}

/// Summary of one push + pull sync cycle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncCycleReport {
    pub pushed: usize,
    pub pulled: usize,
    pub latest_seq: i64,
    pub quota: QuotaUsage,
    pub decrypted: Vec<DecryptedChange>,
}

/// Synchronizes wrapped per-collection keys with the server:
/// 1. Pulls any existing wrapped keys from the server and saves them locally.
/// 2. Generates + wraps any collections not yet present on the server or locally.
/// 3. Uploads newly created wrapped keys so other devices can unwrap them.
pub fn sync_collection_keys(
    store: &Store,
    client: &SyncClient,
    bearer_token: &str,
    account_key: &SecretKey,
    now: Timestamp,
) -> Result<HashMap<Collection, SecretKey>, SyncClientError> {
    let remote = client.get_keys(bearer_token)?;
    let mut keys = HashMap::with_capacity(Collection::ALL.len());

    // First adopt server's wrapped keys (authoritative across devices).
    for rk in &remote.keys {
        let wrapped_bytes = decode_blob(&rk.wrapped_key_b64)?;
        let opened = unwrap_collection_key(account_key, rk.collection.code(), &wrapped_bytes)?;
        store.save_sync_collection_key(&WrappedCollectionKey {
            collection: rk.collection,
            wrapped_key: wrapped_bytes,
            key_version: rk.key_version,
            updated_at: now,
        })?;
        keys.insert(rk.collection, opened);
    }

    // Load any remaining local keys and generate any completely missing ones.
    let mut to_upload = Vec::new();
    for local in store.sync_collection_keys()? {
        if let std::collections::hash_map::Entry::Vacant(slot) = keys.entry(local.collection) {
            let opened =
                unwrap_collection_key(account_key, local.collection.code(), &local.wrapped_key)?;
            to_upload.push(CollectionKeyWire {
                collection: local.collection,
                wrapped_key_b64: encode_blob(&local.wrapped_key),
                key_version: local.key_version,
            });
            slot.insert(opened);
        }
    }

    for collection in Collection::ALL {
        if let std::collections::hash_map::Entry::Vacant(slot) = keys.entry(collection) {
            let fresh = SecretKey::generate()?;
            let wrapped = wrap_collection_key(account_key, collection.code(), &fresh)?;
            store.save_sync_collection_key(&WrappedCollectionKey {
                collection,
                wrapped_key: wrapped.clone(),
                key_version: 1,
                updated_at: now,
            })?;
            to_upload.push(CollectionKeyWire {
                collection,
                wrapped_key_b64: encode_blob(&wrapped),
                key_version: 1,
            });
            slot.insert(fresh);
        }
    }

    if !to_upload.is_empty() {
        // Adopt the server's authoritative keys returned by PUT (handles
        // concurrent first-sync races where another device won ON CONFLICT DO NOTHING).
        let authoritative = client.put_keys(
            bearer_token,
            &PutKeysRequest {
                protocol: PROTOCOL_VERSION,
                keys: to_upload,
            },
        )?;
        for rk in &authoritative.keys {
            let wrapped_bytes = decode_blob(&rk.wrapped_key_b64)?;
            let opened = unwrap_collection_key(account_key, rk.collection.code(), &wrapped_bytes)?;
            store.save_sync_collection_key(&WrappedCollectionKey {
                collection: rk.collection,
                wrapped_key: wrapped_bytes,
                key_version: rk.key_version,
                updated_at: now,
            })?;
            keys.insert(rk.collection, opened);
        }
    }

    Ok(keys)
}

/// Loads all wrapped collection keys from `store` and generates+wraps any
/// missing ones under `account_key`.
pub fn ensure_collection_keys(
    store: &Store,
    account_key: &SecretKey,
    now: Timestamp,
) -> Result<HashMap<Collection, SecretKey>, SyncClientError> {
    let mut keys = HashMap::with_capacity(Collection::ALL.len());
    for wrapped in store.sync_collection_keys()? {
        let key =
            unwrap_collection_key(account_key, wrapped.collection.code(), &wrapped.wrapped_key)?;
        keys.insert(wrapped.collection, key);
    }
    for collection in Collection::ALL {
        if let std::collections::hash_map::Entry::Vacant(slot) = keys.entry(collection) {
            let fresh = SecretKey::generate()?;
            let wrapped = wrap_collection_key(account_key, collection.code(), &fresh)?;
            store.save_sync_collection_key(&WrappedCollectionKey {
                collection,
                wrapped_key: wrapped,
                key_version: 1,
                updated_at: now,
            })?;
            slot.insert(fresh);
        }
    }
    Ok(keys)
}

/// Seals up to `limit` pending outbox rows into a validated `PushRequest`.
pub fn build_push_batch(
    store: &Store,
    account_id: &str,
    device_id: &str,
    keys: &HashMap<Collection, SecretKey>,
    limit: usize,
) -> Result<Option<PushRequest>, SyncClientError> {
    let pending = store.pending_sync_changes(limit.min(MAX_PUSH_BATCH))?;
    if pending.is_empty() {
        return Ok(None);
    }
    let mut changes = Vec::with_capacity(pending.len());
    let mut total_bytes: usize = 0;
    for entry in pending {
        let key = keys
            .get(&entry.collection)
            .ok_or(ottero_crypto::CryptoError::Format)?;
        let hlc_str = entry.hlc.to_string();
        let aad = item_aad(
            account_id,
            entry.collection,
            &entry.item_id,
            entry.schema_version,
            &hlc_str,
            entry.deleted,
        )?;
        let ct = seal_sync_item(key, entry.fields_json.as_bytes(), &aad)?;
        if !changes.is_empty() && total_bytes.saturating_add(ct.len()) > MAX_PUSH_TOTAL_BYTES {
            break;
        }
        total_bytes = total_bytes.saturating_add(ct.len());
        changes.push(PushChange {
            change_id: entry.change_id,
            collection: entry.collection,
            item_id: entry.item_id,
            hlc: entry.hlc,
            schema_version: entry.schema_version,
            deleted: entry.deleted,
            blob_b64: encode_blob(&ct),
        });
    }
    let req = PushRequest {
        protocol: PROTOCOL_VERSION,
        device_id: device_id.to_owned(),
        changes,
    };
    req.validate()?;
    Ok(Some(req))
}

/// Runs one full push (draining `sync_outbox`) + pull (advancing `last_pull_seq`)
/// cycle against `client`.
pub fn sync_once(
    store: &mut Store,
    client: &SyncClient,
    bearer_token: &str,
    account_id: &str,
    device_id: &str,
    account_key: &SecretKey,
    now: Timestamp,
) -> Result<SyncCycleReport, SyncClientError> {
    sync_once_with_apply(
        store,
        client,
        bearer_token,
        account_id,
        device_id,
        account_key,
        now,
        |_, _| Ok(()),
    )
}

/// Runs one full push + pull cycle, invoking `on_page` with each page of
/// decrypted remote changes *before* persisting `last_pull_seq` so a crash or
/// apply failure never silently skips a page.
#[allow(clippy::too_many_arguments)]
pub fn sync_once_with_apply(
    store: &mut Store,
    client: &SyncClient,
    bearer_token: &str,
    account_id: &str,
    device_id: &str,
    account_key: &SecretKey,
    now: Timestamp,
    mut on_page: impl FnMut(&mut Store, &[DecryptedChange]) -> Result<(), SyncClientError>,
) -> Result<SyncCycleReport, SyncClientError> {
    let keys = sync_collection_keys(store, client, bearer_token, account_key, now)?;
    let mut state = store.sync_state()?;
    state.account_id = account_id.to_owned();
    state.device_id = device_id.to_owned();

    let mut clock = match &state.last_hlc {
        Some(last) => HlcClock::from_last(device_id, last)?,
        None => HlcClock::new(device_id)?,
    };

    let mut report = SyncCycleReport {
        latest_seq: state.last_pull_seq,
        ..SyncCycleReport::default()
    };

    // 1. Drain outbox in validated batches.
    while let Some(req) = build_push_batch(store, account_id, device_id, &keys, MAX_PUSH_BATCH)? {
        let resp = client.push(bearer_token, &req)?;
        let ack_ids: Vec<ChangeId> = resp.accepted.iter().map(|a| a.change_id).collect();
        let acked = store.ack_sync_changes(&ack_ids)?;
        report.pushed += acked;
        report.latest_seq = report.latest_seq.max(resp.latest_seq);
        report.quota = resp.quota;
        if acked == 0 {
            break;
        }
    }

    // 2. Pull remote changes until caught up.
    loop {
        let pull_req = PullRequest {
            protocol: PROTOCOL_VERSION,
            after_seq: state.last_pull_seq,
            limit: DEFAULT_PULL_LIMIT,
        };
        let resp = match client.pull(bearer_token, &pull_req) {
            Ok(resp) => resp,
            Err(SyncClientError::CursorExpired) => {
                state.needs_full_resync = true;
                store.save_sync_state(&state)?;
                return Err(SyncClientError::CursorExpired);
            }
            Err(err) => return Err(err),
        };

        report.quota = resp.quota;
        let mut page_decrypted = Vec::with_capacity(resp.changes.len());
        for remote in resp.changes {
            let merged = match clock.receive(&remote.hlc, now.as_millisecond()) {
                Ok(hlc) => hlc,
                Err(ProtocolError::ClockDrift) => {
                    // Never wedge the pull cursor on a historical drifted HLC;
                    // server validates drift on push.
                    clock.tick(now.as_millisecond())?
                }
                Err(err) => return Err(err.into()),
            };
            state.last_hlc = Some(merged);

            // Skip echoes of our own device's writes.
            if remote.device_id == device_id {
                continue;
            }
            let key = keys
                .get(&remote.collection)
                .ok_or(ottero_crypto::CryptoError::Format)?;
            let hlc_str = remote.hlc.to_string();
            let aad = item_aad(
                account_id,
                remote.collection,
                &remote.item_id,
                remote.schema_version,
                &hlc_str,
                remote.deleted,
            )?;
            let ct = decode_blob(&remote.blob_b64)?;
            let mut pt: Zeroizing<Vec<u8>> = open_sync_item(key, &ct, &aad)?;
            let raw_bytes = std::mem::take(&mut *pt);
            let payload_json = Zeroizing::new(
                String::from_utf8(raw_bytes)
                    .map_err(|_| SyncClientError::Decode("non-UTF8 sync payload".into()))?,
            );
            page_decrypted.push(DecryptedChange {
                seq: remote.seq,
                change_id: remote.change_id,
                device_id: remote.device_id,
                collection: remote.collection,
                item_id: remote.item_id,
                hlc: remote.hlc,
                schema_version: remote.schema_version,
                deleted: remote.deleted,
                payload_json,
            });
            report.pulled += 1;
        }

        // Apply the page BEFORE advancing last_pull_seq so a failure never
        // skips unapplied changes.
        on_page(store, &page_decrypted)?;
        report.decrypted.extend(page_decrypted);

        state.last_pull_seq = state.last_pull_seq.max(resp.next_seq);
        report.latest_seq = report.latest_seq.max(state.last_pull_seq);
        state.last_synced_at = Some(now);
        state.needs_full_resync = false;
        store.save_sync_state(&state)?;

        if !resp.has_more {
            break;
        }
    }

    Ok(report)
}
