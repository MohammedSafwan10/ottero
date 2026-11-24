//! Vault rows. Storage never sees plaintext: it stores the header needed to
//! derive the key and opaque sealed blobs, and doesn't depend on the crypto
//! crate.

use rusqlite::{OptionalExtension, params};

use crate::codec::uuid_from_sql;
use crate::{StorageError, StorageResult, Store};
use ottero_core::VaultItemId;

/// The vault header (`vault_meta`). It holds up to three wrappings of the
/// same random vault key; every one is ciphertext. Migration 0012 enforces
/// that at least one way in remains: the master password, or the account
/// key together with a recovery key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultMeta {
    pub version: u32,
    /// The master-password wrapping; `None` once the vault is linked to an
    /// Ottero Cloud account (ADR 0014).
    pub master: Option<MasterWrap>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Salt for the recovery key, and the vault key sealed with it.
    pub recovery_salt: Option<Vec<u8>>,
    pub recovery_wrapped: Option<Vec<u8>>,
    /// The vault key sealed with a key derived from an account key.
    pub account: Option<AccountWrap>,
}

/// How to turn the master password into the key that unwraps the vault key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterWrap {
    pub salt: Vec<u8>,
    pub kdf_memory_kib: u32,
    pub kdf_iterations: u32,
    pub kdf_parallelism: u32,
    /// The vault key, sealed with the key derived from the master password.
    pub wrapped_key: Vec<u8>,
}

/// The vault key wrapped for an Ottero Cloud account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountWrap {
    /// The account's email (normalized), to say which account opens it.
    pub email: String,
    pub wrapped_key: Vec<u8>,
}

/// One sealed item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedItem {
    pub id: VaultItemId,
    pub blob: Vec<u8>,
    pub updated_at: i64,
}

/// What [`Store::link_vault_to_account`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked,
    /// The header changed since the master password was checked (or there
    /// is none, or no recovery key): nothing was written.
    Changed,
}

const META_COLUMNS: &str =
    "version, salt, kdf_memory, kdf_time, kdf_lanes, wrapped_key, created_at, updated_at,
     recovery_salt, recovery_wrapped, account_email, account_wrapped";

type MasterColumns = (
    Option<Vec<u8>>,
    Option<u32>,
    Option<u32>,
    Option<u32>,
    Option<Vec<u8>>,
);

fn master_from(columns: MasterColumns) -> StorageResult<Option<MasterWrap>> {
    match columns {
        (Some(salt), Some(m), Some(t), Some(p), Some(wrapped_key)) => Ok(Some(MasterWrap {
            salt,
            kdf_memory_kib: m,
            kdf_iterations: t,
            kdf_parallelism: p,
            wrapped_key,
        })),
        (None, None, None, None, None) => Ok(None),
        _ => Err(StorageError::Corrupt("vault_meta.salt")),
    }
}

impl Store {
    pub fn vault_meta(&self) -> StorageResult<Option<VaultMeta>> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT {META_COLUMNS} FROM vault_meta WHERE id = 1"),
                [],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?),
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<Vec<u8>>>(8)?,
                        r.get::<_, Option<Vec<u8>>>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, Option<Vec<u8>>>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((version, master, created_at, updated_at, rs, rw, email, aw)) = row else {
            return Ok(None);
        };
        let account = match (email, aw) {
            (Some(email), Some(wrapped_key)) => Some(AccountWrap { email, wrapped_key }),
            (None, None) => None,
            _ => return Err(StorageError::Corrupt("vault_meta.account_email")),
        };
        Ok(Some(VaultMeta {
            version,
            master: master_from(master)?,
            created_at,
            updated_at,
            recovery_salt: rs,
            recovery_wrapped: rw,
            account,
        }))
    }

    /// Writes the whole header in one statement (setup, a master-password
    /// change, a new recovery key, recovery).
    pub fn save_vault_meta(&mut self, m: &VaultMeta) -> StorageResult<()> {
        let master = m.master.as_ref();
        let account = m.account.as_ref();
        self.conn.execute(
            "INSERT INTO vault_meta (id, version, salt, kdf_memory, kdf_time, kdf_lanes, wrapped_key, created_at, updated_at,
                                     recovery_salt, recovery_wrapped, account_email, account_wrapped)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (id) DO UPDATE SET
                version = excluded.version, salt = excluded.salt,
                kdf_memory = excluded.kdf_memory, kdf_time = excluded.kdf_time,
                kdf_lanes = excluded.kdf_lanes, wrapped_key = excluded.wrapped_key,
                updated_at = excluded.updated_at,
                recovery_salt = excluded.recovery_salt,
                recovery_wrapped = excluded.recovery_wrapped,
                account_email = excluded.account_email,
                account_wrapped = excluded.account_wrapped",
            params![
                m.version,
                master.map(|w| &w.salt),
                master.map(|w| w.kdf_memory_kib),
                master.map(|w| w.kdf_iterations),
                master.map(|w| w.kdf_parallelism),
                master.map(|w| &w.wrapped_key),
                m.created_at,
                m.updated_at,
                m.recovery_salt,
                m.recovery_wrapped,
                account.map(|a| &a.email),
                account.map(|a| &a.wrapped_key),
            ],
        )?;
        Ok(())
    }

    /// Links the vault to an account in one transaction: optionally stores
    /// a new recovery wrapping, then stores the account wrapping and retires
    /// the master-password wrapping. Runs only if the master wrapping is
    /// still `expected_master` (the one the caller just opened), so a
    /// concurrent password change is never overwritten. Any failure rolls
    /// everything back and the old master password keeps working.
    pub fn link_vault_to_account(
        &mut self,
        expected_master: &[u8],
        account: &AccountWrap,
        new_recovery: Option<(&[u8], &[u8])>,
        updated_at: i64,
    ) -> StorageResult<LinkOutcome> {
        let tx = self.conn.transaction()?;
        if let Some((salt, wrapped)) = new_recovery {
            let n = tx.execute(
                "UPDATE vault_meta SET recovery_salt = ?1, recovery_wrapped = ?2
                 WHERE id = 1 AND wrapped_key = ?3",
                params![salt, wrapped, expected_master],
            )?;
            if n != 1 {
                return Ok(LinkOutcome::Changed); // dropping `tx` rolls back
            }
        }
        let n = tx.execute(
            "UPDATE vault_meta SET
                account_email = ?1, account_wrapped = ?2,
                salt = NULL, kdf_memory = NULL, kdf_time = NULL, kdf_lanes = NULL,
                wrapped_key = NULL, updated_at = ?3
             WHERE id = 1 AND wrapped_key = ?4
               AND recovery_salt IS NOT NULL AND recovery_wrapped IS NOT NULL",
            params![
                account.email,
                account.wrapped_key,
                updated_at,
                expected_master
            ],
        )?;
        if n != 1 {
            return Ok(LinkOutcome::Changed);
        }
        tx.commit()?;
        Ok(LinkOutcome::Linked)
    }

    pub fn vault_item(&self, id: VaultItemId) -> StorageResult<Option<SealedItem>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, blob, updated_at FROM vault_items WHERE id = ?1")?;
        stmt.query_row(params![id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .optional()?
        .map(|(id, blob, updated_at)| {
            Ok(SealedItem {
                id: VaultItemId::from_uuid(uuid_from_sql(&id, "vault_items.id")?),
                blob,
                updated_at,
            })
        })
        .transpose()
    }

    pub fn vault_items(&self) -> StorageResult<Vec<SealedItem>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, blob, updated_at FROM vault_items ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, blob, updated_at) = row?;
            Ok(SealedItem {
                id: VaultItemId::from_uuid(uuid_from_sql(&id, "vault_items.id")?),
                blob,
                updated_at,
            })
        })
        .collect()
    }

    /// Saves several sealed items in one transaction (an import, say).
    pub fn save_vault_items(&mut self, items: &[SealedItem]) -> StorageResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO vault_items (id, blob, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET blob = excluded.blob, updated_at = excluded.updated_at",
            )?;
            for item in items {
                stmt.execute(params![item.id.to_string(), item.blob, item.updated_at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes the header and every item in one transaction (after the app
    /// has written an archive). The next launch shows setup.
    pub fn reset_vault(&mut self) -> StorageResult<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM vault_items", [])?;
        tx.execute("DELETE FROM vault_meta", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_vault_items(&mut self, ids: &[VaultItemId]) -> StorageResult<usize> {
        let tx = self.conn.transaction()?;
        let mut n = 0;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM vault_items WHERE id = ?1")?;
            for id in ids {
                n += stmt.execute([id.to_string()])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master(wrapped: u8) -> MasterWrap {
        MasterWrap {
            salt: vec![1; 16],
            kdf_memory_kib: 65_536,
            kdf_iterations: 3,
            kdf_parallelism: 4,
            wrapped_key: vec![wrapped; 72],
        }
    }

    fn meta() -> VaultMeta {
        VaultMeta {
            version: 1,
            master: Some(master(9)),
            created_at: 1,
            updated_at: 1,
            recovery_salt: None,
            recovery_wrapped: None,
            account: None,
        }
    }

    fn account() -> AccountWrap {
        AccountWrap {
            email: "sam@example.com".into(),
            wrapped_key: vec![5; 75],
        }
    }

    fn with_recovery() -> VaultMeta {
        VaultMeta {
            recovery_salt: Some(vec![3; 16]),
            recovery_wrapped: Some(vec![4; 75]),
            ..meta()
        }
    }

    #[test]
    fn meta_and_items_round_trip() {
        let mut store = Store::open_in_memory().expect("store");
        assert_eq!(store.vault_meta().expect("meta"), None);
        store.save_vault_meta(&meta()).expect("save");
        let mut m = meta();
        m.master = Some(master(7));
        m.updated_at = 2;
        store.save_vault_meta(&m).expect("update");
        let loaded = store.vault_meta().expect("meta").expect("some");
        assert_eq!(loaded.master.expect("master").wrapped_key, vec![7; 72]);
        assert_eq!(loaded.created_at, 1, "created_at kept");

        let a = SealedItem {
            id: VaultItemId::new(),
            blob: vec![1, 2, 3],
            updated_at: 5,
        };
        let b = SealedItem {
            id: VaultItemId::new(),
            blob: vec![4],
            updated_at: 6,
        };
        store
            .save_vault_items(&[a.clone(), b.clone()])
            .expect("save");
        let mut b2 = b.clone();
        b2.blob = vec![5, 5];
        store.save_vault_items(&[b2.clone()]).expect("update");
        let items = store.vault_items().expect("items");
        assert_eq!(items.len(), 2);
        assert!(items.contains(&b2));
        assert_eq!(store.delete_vault_items(&[a.id]).expect("delete"), 1);
        assert_eq!(store.vault_items().expect("items"), vec![b2]);

        let mut m = store.vault_meta().expect("meta").expect("some");
        m.recovery_salt = Some(vec![3; 16]);
        m.recovery_wrapped = Some(vec![4; 72]);
        store.save_vault_meta(&m).expect("recovery");
        assert_eq!(
            store
                .vault_meta()
                .expect("meta")
                .expect("some")
                .recovery_wrapped,
            Some(vec![4; 72])
        );

        store.reset_vault().expect("reset");
        assert_eq!(store.vault_meta().expect("meta"), None);
        assert!(store.vault_items().expect("items").is_empty());
    }

    #[test]
    fn only_one_header_row_exists() {
        let store = Store::open_in_memory().expect("store");
        let r = store.conn.execute(
            "INSERT INTO vault_meta (id, version, salt, kdf_memory, kdf_time, kdf_lanes, wrapped_key, created_at, updated_at)
             VALUES (2, 1, x'00', 1, 1, 1, x'00', 0, 0)",
            [],
        );
        assert!(r.is_err());
    }

    #[test]
    fn linking_swaps_the_master_wrapping_for_the_account_wrapping() {
        let mut store = Store::open_in_memory().expect("store");
        store.save_vault_meta(&with_recovery()).expect("save");
        let outcome = store
            .link_vault_to_account(&[9; 72], &account(), None, 5)
            .expect("link");
        assert_eq!(outcome, LinkOutcome::Linked);
        let m = store.vault_meta().expect("meta").expect("some");
        assert_eq!(m.master, None, "master password retired");
        assert_eq!(m.account, Some(account()));
        assert_eq!(m.recovery_wrapped, Some(vec![4; 75]), "recovery kept");
        assert_eq!((m.created_at, m.updated_at), (1, 5));
        // A second link finds no master wrapping: nothing changes.
        assert_eq!(
            store
                .link_vault_to_account(&[9; 72], &account(), None, 6)
                .expect("again"),
            LinkOutcome::Changed
        );
    }

    #[test]
    fn linking_refuses_a_changed_header_and_a_vault_without_recovery() {
        let mut store = Store::open_in_memory().expect("store");
        store.save_vault_meta(&meta()).expect("save");
        // No recovery key and none supplied: refused, nothing written.
        assert_eq!(
            store
                .link_vault_to_account(&[9; 72], &account(), None, 5)
                .expect("link"),
            LinkOutcome::Changed
        );
        assert_eq!(store.vault_meta().expect("meta"), Some(meta()));
        // The master password changed in the meantime: refused.
        assert_eq!(
            store
                .link_vault_to_account(&[8; 72], &account(), Some((&[3; 16], &[4; 75])), 5)
                .expect("link"),
            LinkOutcome::Changed
        );
        assert_eq!(
            store.vault_meta().expect("meta"),
            Some(meta()),
            "recovery not written either"
        );
        // With a new recovery key in the same transaction: linked.
        assert_eq!(
            store
                .link_vault_to_account(&[9; 72], &account(), Some((&[3; 16], &[4; 75])), 5)
                .expect("link"),
            LinkOutcome::Linked
        );
        let m = store.vault_meta().expect("meta").expect("some");
        assert_eq!(m.recovery_salt, Some(vec![3; 16]));
        assert_eq!(m.master, None);
    }

    /// Crash simulation: the second statement of the link fails (an abort
    /// trigger stands in for the process dying mid-transaction). The new
    /// recovery wrapping written by the first statement must roll back too,
    /// and the master wrapping must be intact, on disk, after reopening.
    #[test]
    fn an_interrupted_link_leaves_the_old_header_on_disk() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("vault.sqlite3");
        {
            let mut store = Store::open(&path).expect("store");
            store.save_vault_meta(&meta()).expect("save");
            store
                .conn
                .execute_batch(
                    "CREATE TEMP TRIGGER die BEFORE UPDATE OF account_wrapped ON vault_meta
                     BEGIN SELECT RAISE(ABORT, 'simulated crash'); END;",
                )
                .expect("trigger");
            let result =
                store.link_vault_to_account(&[9; 72], &account(), Some((&[3; 16], &[4; 75])), 5);
            assert!(result.is_err(), "the failure surfaces");
            assert_eq!(store.vault_meta().expect("meta"), Some(meta()));
        }
        let reopened = Store::open(&path).expect("reopen");
        assert_eq!(reopened.vault_meta().expect("meta"), Some(meta()));
    }

    #[test]
    fn the_schema_never_allows_a_vault_without_a_way_in() {
        let mut store = Store::open_in_memory().expect("store");
        // No master and no account wrapping.
        let none = VaultMeta {
            master: None,
            ..with_recovery()
        };
        assert!(store.save_vault_meta(&none).is_err());
        // Account wrapping but no recovery key.
        let no_recovery = VaultMeta {
            master: None,
            account: Some(account()),
            ..meta()
        };
        assert!(store.save_vault_meta(&no_recovery).is_err());
        // Half a master wrapping.
        store.save_vault_meta(&meta()).expect("save");
        assert!(
            store
                .conn
                .execute("UPDATE vault_meta SET salt = NULL", [])
                .is_err()
        );
        // Account email without its wrapping.
        assert!(
            store
                .conn
                .execute("UPDATE vault_meta SET account_email = 'x@y.z'", [])
                .is_err()
        );
        // A vault set up under the account key with a recovery key is fine.
        let linked = VaultMeta {
            master: None,
            account: Some(account()),
            ..with_recovery()
        };
        store.save_vault_meta(&linked).expect("linked");
        assert_eq!(store.vault_meta().expect("meta"), Some(linked));
    }
}
