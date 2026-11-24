//! End-to-end encrypted push/pull sync engine (IO-isolated, no GPUI dependency).

use std::collections::HashMap;

use ottero_core::Timestamp;
use ottero_crypto::sync::{
    open_sync_item, seal_sync_item, unwrap_collection_key, wrap_collection_key,
};
use ottero_crypto::{SecretKey, Zeroizing};
use ottero_storage::{Store, WrappedCollectionKey};
use ottero_sync_protocol::{
    ChangeId, Collection, DEFAULT_PULL_LIMIT, Hlc, HlcClock, MAX_PUSH_BATCH, PROTOCOL_VERSION,
    PullRequest, PushChange, PushRequest, QuotaUsage, decode_blob, encode_blob, item_aad,
};

use crate::sync::client::{SyncClient, SyncClientError};

/// A verified, decrypted remote change ready to apply to local storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedChange {
    pub seq: i64,
    pub change_id: ChangeId,
    pub device_id: String,
    pub collection: Collection,
    pub item_id: String,
    pub hlc: Hlc,
    pub schema_version: u16,
    pub deleted: bool,
    pub payload_json: String,
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
    for entry in pending {
        let key = keys
            .get(&entry.collection)
            .ok_or(ottero_crypto::CryptoError::Format)?;
        let aad = item_aad(
            account_id,
            entry.collection,
            &entry.item_id,
            entry.schema_version,
        )?;
        let ct = seal_sync_item(key, entry.fields_json.as_bytes(), &aad)?;
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
    let keys = ensure_collection_keys(store, account_key, now)?;
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
        for remote in resp.changes {
            let merged = clock.receive(&remote.hlc, now.as_millisecond())?;
            state.last_hlc = Some(merged);

            // Skip echoes of our own device's writes.
            if remote.device_id == device_id {
                continue;
            }
            let key = keys
                .get(&remote.collection)
                .ok_or(ottero_crypto::CryptoError::Format)?;
            let aad = item_aad(
                account_id,
                remote.collection,
                &remote.item_id,
                remote.schema_version,
            )?;
            let ct = decode_blob(&remote.blob_b64)?;
            let pt: Zeroizing<Vec<u8>> = open_sync_item(key, &ct, &aad)?;
            let payload_json = String::from_utf8(pt.to_vec())
                .map_err(|_| SyncClientError::Decode("non-UTF8 sync payload".into()))?;
            report.decrypted.push(DecryptedChange {
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
