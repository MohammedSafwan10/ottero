//! SQLite persistence for Ottero.
//!
//! One [`Store`] owns one connection. It is `Send`, so the app can move it to a
//! background thread later without API changes. All writes that touch more
//! than one table run in a transaction.

mod codec;
mod developer;
mod error;
mod events;
mod focus;
mod habits;
mod notes;
mod schema;
mod sync;
mod tasks;
mod vault;

use std::path::Path;

use rusqlite::Connection;

pub use error::{StorageError, StorageResult};
pub use notes::NoteSummary;
pub use sync::{OutboxEntry, SyncState, WrappedCollectionKey};
pub use tasks::{Progress, TaskCounts, TaskFilter};
pub use vault::{AccountWrap, LinkOutcome, MasterWrap, SealedItem, VaultMeta};

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Consistent SQLite snapshot, including committed WAL pages. Never copy a
    /// live .sqlite3 file directly or overwrite an existing recovery snapshot.
    pub fn backup_for_update(&self, path: &Path) -> StorageResult<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(StorageError::Io)?;
        let result = self
            .conn
            .backup(rusqlite::MAIN_DB, path, None)
            .map_err(StorageError::from)
            .and_then(|()| file.sync_all().map_err(StorageError::Io));
        if result.is_err() {
            _ = std::fs::remove_file(path);
        }
        result
    }

    /// Opens (or creates) the database at `path` and migrates it to the latest schema.
    pub fn open(path: &Path) -> StorageResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
        }
        let conn = Connection::open(path)?;
        // WAL keeps reads fast while a write is in progress and survives crashes well.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    /// A private in-memory database. Used by tests and previews.
    pub fn open_in_memory() -> StorageResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> StorageResult<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Current schema version, for diagnostics.
    pub fn schema_version(&self) -> StorageResult<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }
}

#[cfg(test)]
mod update_backup_tests {
    use super::*;

    #[test]
    fn snapshot_includes_wal_data_and_search_without_overwriting_existing_files() {
        let dir = tempfile::tempdir().expect("dir");
        let mut store = Store::open(&dir.path().join("live.sqlite3")).expect("store");
        let note = ottero_core::Note::new(
            "Recovery",
            "recently committed text",
            ottero_core::clock::now(),
        )
        .expect("note");
        store.save_note(&note).expect("save");
        let path = dir.path().join("snapshot.sqlite3");
        store.backup_for_update(&path).expect("snapshot");
        assert!(
            store.backup_for_update(&path).is_err(),
            "never overwrite a recovery file"
        );
        let backup = Store::open(&path).expect("backup");
        assert_eq!(backup.note(note.id()).expect("read"), Some(note));
        assert_eq!(
            backup.search_notes("recently", 10).expect("search").len(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
