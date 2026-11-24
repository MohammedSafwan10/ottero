//! Client-side Ottero Cloud sync state, wrapped collection keys, outbox,
//! and per-field HLC clocks.

use ottero_core::Timestamp;
use ottero_sync_protocol::{ChangeId, Collection, Hlc};
use rusqlite::{OptionalExtension as _, params};

use crate::codec::{opt_ts_from_sql, ts_from_sql, ts_to_sql, uuid_from_sql};
use crate::{StorageError, StorageResult, Store};

/// Persistent sync cursor and metadata for the current signed-in account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncState {
    pub account_id: String,
    pub device_id: String,
    pub last_pull_seq: i64,
    pub last_hlc: Option<Hlc>,
    pub last_synced_at: Option<Timestamp>,
    pub needs_full_resync: bool,
}

/// One collection's encryption key, wrapped under `account_key` (`ottero-crypto::sync`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedCollectionKey {
    pub collection: Collection,
    pub wrapped_key: Vec<u8>,
    pub key_version: u32,
    pub updated_at: Timestamp,
}

/// One local write queued in `sync_outbox` awaiting upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub change_id: ChangeId,
    pub collection: Collection,
    pub item_id: String,
    pub hlc: Hlc,
    pub schema_version: u16,
    pub deleted: bool,
    pub fields_json: String,
    pub created_at: Timestamp,
}

impl Store {
    pub fn sync_state(&self) -> StorageResult<SyncState> {
        let row: (String, String, i64, Option<String>, Option<i64>, i64) = self.conn.query_row(
            "SELECT account_id, device_id, last_pull_seq, last_hlc, last_synced_at, needs_full_resync
             FROM sync_state WHERE id = 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
        let last_hlc = row
            .3
            .map(|s| {
                s.parse::<Hlc>()
                    .map_err(|_| StorageError::Corrupt("sync_state.last_hlc"))
            })
            .transpose()?;
        let last_synced_at = opt_ts_from_sql(row.4, "sync_state.last_synced_at")?;
        Ok(SyncState {
            account_id: row.0,
            device_id: row.1,
            last_pull_seq: row.2,
            last_hlc,
            last_synced_at,
            needs_full_resync: row.5 != 0,
        })
    }

    pub fn save_sync_state(&self, state: &SyncState) -> StorageResult<()> {
        self.conn.execute(
            "UPDATE sync_state
             SET account_id = ?1,
                 device_id = ?2,
                 last_pull_seq = ?3,
                 last_hlc = ?4,
                 last_synced_at = ?5,
                 needs_full_resync = ?6
             WHERE id = 1",
            params![
                state.account_id,
                state.device_id,
                state.last_pull_seq.max(0),
                state.last_hlc.as_ref().map(ToString::to_string),
                state.last_synced_at.map(ts_to_sql),
                i64::from(state.needs_full_resync),
            ],
        )?;
        Ok(())
    }

    pub fn save_sync_collection_key(&self, key: &WrappedCollectionKey) -> StorageResult<()> {
        self.conn.execute(
            "INSERT INTO sync_collection_keys (collection, wrapped_key, key_version, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(collection) DO UPDATE SET
                 wrapped_key = excluded.wrapped_key,
                 key_version = excluded.key_version,
                 updated_at = excluded.updated_at",
            params![
                key.collection.code(),
                key.wrapped_key,
                i64::from(key.key_version.max(1)),
                ts_to_sql(key.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn sync_collection_keys(&self) -> StorageResult<Vec<WrappedCollectionKey>> {
        let mut stmt = self.conn.prepare(
            "SELECT collection, wrapped_key, key_version, updated_at
             FROM sync_collection_keys
             ORDER BY collection ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (col_str, wrapped_key, key_version, updated_at_ms) in rows {
            let collection = Collection::from_code(&col_str)
                .ok_or(StorageError::Corrupt("sync_collection_keys.collection"))?;
            let updated_at = ts_from_sql(updated_at_ms, "sync_collection_keys.updated_at")?;
            out.push(WrappedCollectionKey {
                collection,
                wrapped_key,
                key_version: u32::try_from(key_version.max(1)).unwrap_or(u32::MAX),
                updated_at,
            });
        }
        Ok(out)
    }

    pub fn enqueue_sync_change(&self, entry: &OutboxEntry) -> StorageResult<()> {
        self.conn.execute(
            "INSERT INTO sync_outbox (
                change_id, collection, item_id, hlc, schema_version, deleted, fields_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.change_id.0.to_string(),
                entry.collection.code(),
                entry.item_id,
                entry.hlc.to_string(),
                i64::from(entry.schema_version.max(1)),
                i64::from(entry.deleted),
                entry.fields_json,
                ts_to_sql(entry.created_at),
            ],
        )?;
        Ok(())
    }

    pub fn pending_sync_changes(&self, limit: usize) -> StorageResult<Vec<OutboxEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, collection, item_id, hlc, schema_version, deleted, fields_json, created_at
             FROM sync_outbox
             ORDER BY hlc ASC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (cid_str, col_str, item_id, hlc_str, schema_v, deleted, fields_json, created_ms) in rows
        {
            let change_id = ChangeId(uuid_from_sql(&cid_str, "sync_outbox.change_id")?);
            let collection = Collection::from_code(&col_str)
                .ok_or(StorageError::Corrupt("sync_outbox.collection"))?;
            let hlc = hlc_str
                .parse::<Hlc>()
                .map_err(|_| StorageError::Corrupt("sync_outbox.hlc"))?;
            let created_at = ts_from_sql(created_ms, "sync_outbox.created_at")?;
            out.push(OutboxEntry {
                change_id,
                collection,
                item_id,
                hlc,
                schema_version: u16::try_from(schema_v.max(1)).unwrap_or(u16::MAX),
                deleted: deleted != 0,
                fields_json,
                created_at,
            });
        }
        Ok(out)
    }

    pub fn ack_sync_changes(&mut self, change_ids: &[ChangeId]) -> StorageResult<usize> {
        if change_ids.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut removed = 0usize;
        {
            let mut stmt = tx.prepare("DELETE FROM sync_outbox WHERE change_id = ?1")?;
            for id in change_ids {
                removed += stmt.execute(params![id.0.to_string()])?;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn pending_sync_count(&self) -> StorageResult<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))?;
        Ok(u64::try_from(count.max(0)).unwrap_or(0))
    }

    /// Reads the last winning HLC for `(collection, item_id, field)`.
    pub fn field_clock(
        &self,
        collection: Collection,
        item_id: &str,
        field: &str,
    ) -> StorageResult<Option<Hlc>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT hlc FROM sync_field_clocks
                 WHERE collection = ?1 AND item_id = ?2 AND field = ?3",
                params![collection.code(), item_id, field],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| {
            s.parse::<Hlc>()
                .map_err(|_| StorageError::Corrupt("sync_field_clocks.hlc"))
        })
        .transpose()
    }

    /// Per-field LWW update: records `hlc` and returns `true` only if no clock
    /// exists yet or `hlc` is strictly newer than the stored clock.
    pub fn set_field_clock_if_newer(
        &self,
        collection: Collection,
        item_id: &str,
        field: &str,
        hlc: &Hlc,
    ) -> StorageResult<bool> {
        let encoded = hlc.to_string();
        let changed = self.conn.execute(
            "INSERT INTO sync_field_clocks (collection, item_id, field, hlc)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(collection, item_id, field) DO UPDATE SET
                 hlc = excluded.hlc
             WHERE excluded.hlc > sync_field_clocks.hlc",
            params![collection.code(), item_id, field, encoded],
        )?;
        Ok(changed > 0)
    }

    /// Clears all local sync metadata (used on sign-out or account switch).
    pub fn clear_sync_state(&mut self) -> StorageResult<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE sync_state
             SET account_id = '', device_id = '', last_pull_seq = 0,
                 last_hlc = NULL, last_synced_at = NULL, needs_full_resync = 0
             WHERE id = 1",
            [],
        )?;
        tx.execute("DELETE FROM sync_collection_keys", [])?;
        tx.execute("DELETE FROM sync_outbox", [])?;
        tx.execute("DELETE FROM sync_field_clocks", [])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ottero_core::Timestamp;
    use ottero_sync_protocol::{ChangeId, Collection, Hlc};

    use super::*;

    fn ts(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).expect("ts")
    }

    #[test]
    fn sync_state_keys_outbox_and_clear_round_trip() {
        let mut store = Store::open_in_memory().expect("db");

        let mut state = store.sync_state().expect("default state");
        assert_eq!(state.last_pull_seq, 0);
        assert!(!state.needs_full_resync);

        let h1 = Hlc::new(1_760_000_000_000, 0, "dev-a").expect("h1");
        let h2 = Hlc::new(1_760_000_000_005, 1, "dev-a").expect("h2");
        state.account_id = "acct-1".into();
        state.device_id = "dev-a".into();
        state.last_pull_seq = 42;
        state.last_hlc = Some(h2.clone());
        state.last_synced_at = Some(ts(1_760_000_000_005));
        store.save_sync_state(&state).expect("save state");
        assert_eq!(store.sync_state().expect("reload"), state);

        let key = WrappedCollectionKey {
            collection: Collection::Notes,
            wrapped_key: vec![1, 2, 3, 4],
            key_version: 1,
            updated_at: ts(1_760_000_000_000),
        };
        store.save_sync_collection_key(&key).expect("save key");
        assert_eq!(store.sync_collection_keys().expect("keys"), vec![key]);

        // Enqueue out of order; pending_sync_changes returns in HLC order.
        let e2 = OutboxEntry {
            change_id: ChangeId::new(),
            collection: Collection::Tasks,
            item_id: "t-1".into(),
            hlc: h2.clone(),
            schema_version: 1,
            deleted: false,
            fields_json: r#"{"title":"B"}"#.into(),
            created_at: ts(1_760_000_000_005),
        };
        let e1 = OutboxEntry {
            change_id: ChangeId::new(),
            collection: Collection::Tasks,
            item_id: "t-1".into(),
            hlc: h1.clone(),
            schema_version: 1,
            deleted: false,
            fields_json: r#"{"title":"A"}"#.into(),
            created_at: ts(1_760_000_000_000),
        };
        store.enqueue_sync_change(&e2).expect("e2");
        store.enqueue_sync_change(&e1).expect("e1");
        assert_eq!(store.pending_sync_count().expect("count"), 2);

        let batch = store.pending_sync_changes(10).expect("batch");
        assert_eq!(batch, vec![e1.clone(), e2.clone()]);

        assert_eq!(store.ack_sync_changes(&[e1.change_id]).expect("ack"), 1);
        assert_eq!(store.pending_sync_count().expect("count"), 1);

        store.clear_sync_state().expect("clear");
        assert_eq!(store.pending_sync_count().expect("count"), 0);
        assert!(store.sync_collection_keys().expect("keys").is_empty());
        assert_eq!(store.sync_state().expect("state"), SyncState::default());
    }

    #[test]
    fn field_clocks_enforce_strict_per_field_lww() {
        let store = Store::open_in_memory().expect("db");
        let older = Hlc::new(1_760_000_000_000, 0, "dev-a").expect("older");
        let newer = Hlc::new(1_760_000_000_000, 1, "dev-a").expect("newer");

        assert!(
            store
                .set_field_clock_if_newer(Collection::Tasks, "t-1", "title", &older)
                .expect("first")
        );
        assert!(
            !store
                .set_field_clock_if_newer(Collection::Tasks, "t-1", "title", &older)
                .expect("equal rejected")
        );
        assert!(
            store
                .set_field_clock_if_newer(Collection::Tasks, "t-1", "title", &newer)
                .expect("newer wins")
        );
        assert!(
            !store
                .set_field_clock_if_newer(Collection::Tasks, "t-1", "title", &older)
                .expect("older rejected")
        );
        assert_eq!(
            store
                .field_clock(Collection::Tasks, "t-1", "title")
                .expect("read"),
            Some(newer)
        );
    }
}
