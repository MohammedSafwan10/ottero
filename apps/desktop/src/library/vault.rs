//! Password vault lifecycle, items, archives, and account linking on [`Library`].

use std::time::SystemTime;

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::vault::lock::{LockState, Opener};
use ottero_core::vault::{ItemFields, VaultItem, recovery};
use ottero_core::{VaultItemId, clock};
use ottero_crypto::{KdfParams, SecretString, Zeroizing};

use crate::vault::{
    AccountUnlock, ArchiveFile, ArchiveOpener, KdfInput, LinkResult, NewOpener, RECOVERY_KDF,
    VaultFilter, VaultSession, VaultSort, derive,
};

use super::{ArchiveSecret, Library, LibraryEvent};

impl Library {
    // ---- Vault -----------------------------------------------------------

    pub fn vault(&self) -> &VaultSession {
        &self.vault
    }

    pub fn vault_state(&self) -> LockState {
        self.vault.state()
    }

    pub fn vault_is_unlocked(&self) -> bool {
        self.vault.is_unlocked()
    }

    #[doc(hidden)]
    pub fn set_kdf_params_for_tests(&mut self, params: KdfParams) {
        self.kdf = params;
    }

    #[doc(hidden)]
    pub fn backdate_vault_activity_for_tests(&mut self, by: std::time::Duration) {
        if let Some(earlier) = SystemTime::now().checked_sub(by) {
            self.vault_activity = earlier;
        }
    }

    /// Records activity, postponing auto-lock.
    pub fn touch_vault(&mut self) {
        self.vault_activity = SystemTime::now();
    }

    /// Time since the last activity. A clock moved backwards counts as
    /// long idle, so the vault locks rather than stays open.
    pub fn vault_idle(&self) -> std::time::Duration {
        SystemTime::now()
            .duration_since(self.vault_activity)
            .unwrap_or(std::time::Duration::MAX)
    }

    pub fn set_archive_dir(&mut self, dir: std::path::PathBuf) {
        self.archive_dir = dir;
    }

    /// Creates the vault and its recovery key. Argon2 runs in the
    /// background; resolves to the recovery key to show the user once.
    pub fn set_up_vault(
        &mut self,
        password: SecretString,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<Zeroizing<String>>> {
        let inputs =
            KdfInput::fresh(self.kdf).and_then(|a| Ok((a, KdfInput::fresh(RECOVERY_KDF)?)));
        let ((input, rinput), recovery) = match inputs.and_then(|i| Ok((i, new_recovery_key()?))) {
            Ok(v) => v,
            Err(err) => return gpui_kit::Task::ready(Err(err)),
        };
        cx.spawn(async move |this, cx| {
            let normalized =
                recovery::normalize(&recovery).context("Couldn't make a recovery key")?;
            let (kek, rkek) = cx
                .background_executor()
                .spawn(async move {
                    let rk = SecretString::from(normalized.to_string());
                    anyhow::Ok((
                        derive(&password, &input.salt, input.params)?,
                        derive(&rk, &rinput.salt, rinput.params)?,
                    ))
                })
                .await?;
            this.update(cx, |lib, cx| {
                lib.vault.finish_setup(
                    &mut lib.store,
                    input,
                    &kek,
                    Some((rinput, &rkek)),
                    clock::now(),
                )?;
                lib.touch_vault();
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(recovery)
            })?
        })
    }

    pub fn vault_has_recovery(&self) -> bool {
        VaultSession::has_recovery(&self.store)
    }

    /// Makes a new recovery key (the old one stops working). Unlocked only.
    pub fn new_recovery_key(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<Zeroizing<String>>> {
        let made = KdfInput::fresh(RECOVERY_KDF).and_then(|i| Ok((i, new_recovery_key()?)));
        let (rinput, recovery) = match made {
            Ok(v) => v,
            Err(err) => return gpui_kit::Task::ready(Err(err)),
        };
        cx.spawn(async move |this, cx| {
            let normalized =
                recovery::normalize(&recovery).context("Couldn't make a recovery key")?;
            let rkek = cx
                .background_executor()
                .spawn(async move {
                    derive(
                        &SecretString::from(normalized.to_string()),
                        &rinput.salt,
                        rinput.params,
                    )
                })
                .await?;
            this.update(cx, |lib, cx| {
                lib.vault
                    .set_recovery(&mut lib.store, rinput, &rkek, clock::now())?;
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(recovery)
            })?
        })
    }

    /// Unlocks with the recovery key. With `new_password`, that becomes the
    /// master password (and a link to an account is removed); without it,
    /// the account open now opens the vault from then on. Resolves to
    /// `false` for a wrong recovery key; nothing changes then.
    pub fn recover_vault(
        &mut self,
        recovery_key: &str,
        new_password: Option<SecretString>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<bool>> {
        let Some(normalized) = recovery::normalize(recovery_key) else {
            return gpui_kit::Task::ready(Ok(false));
        };
        let account = if new_password.is_none() {
            match self.open_account() {
                Some((email, epoch)) => Some((email, epoch)),
                None => {
                    return gpui_kit::Task::ready(Err(anyhow::anyhow!(
                        "Unlock your Ottero account first, or choose a new master password."
                    )));
                }
            }
        } else {
            None
        };
        let inputs = VaultSession::recovery_kdf(&self.store)
            .and_then(|r| Ok((r, KdfInput::fresh(self.kdf)?)));
        let (rinput, new_input) = match inputs {
            Ok(v) => v,
            Err(err) => return gpui_kit::Task::ready(Err(err)),
        };
        cx.spawn(async move |this, cx| {
            let (rkek, new_kek) = cx
                .background_executor()
                .spawn(async move {
                    let rkek = derive(
                        &SecretString::from(normalized.to_string()),
                        &rinput.salt,
                        rinput.params,
                    );
                    let new_kek = new_password
                        .map(|p| derive(&p, &new_input.salt, new_input.params))
                        .transpose();
                    (rkek, new_kek)
                })
                .await;
            let new_kek = new_kek?;
            this.update(cx, |lib, cx| {
                let now = clock::now();
                let ok = match (&new_kek, &account) {
                    (Some(kek), _) => lib.vault.recover(
                        &mut lib.store,
                        rkek,
                        NewOpener::MasterPassword(new_input, kek),
                        now,
                    )?,
                    (None, Some((email, epoch))) => {
                        let Some(key) = lib.account.key().filter(|_| {
                            lib.account.epoch() == *epoch
                                && lib.account.signed_in_email().as_deref() == Some(email.as_str())
                        }) else {
                            anyhow::bail!("The account changed in the meantime. Try again.");
                        };
                        lib.vault.recover(
                            &mut lib.store,
                            rkek,
                            NewOpener::Account {
                                email,
                                account_key: key,
                            },
                            now,
                        )?
                    }
                    (None, None) => false,
                };
                if ok {
                    lib.vault_account_problem = None;
                    lib.touch_vault();
                    if new_kek.is_none() {
                        lib.record_vault_meta_and_items_sync(cx);
                    }
                }
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(ok)
            })?
        })
    }

    /// Archives the encrypted vault to the archive folder and empties it.
    /// Returns the archive's path.
    pub fn start_new_vault(&mut self, cx: &mut Context<Self>) -> Result<std::path::PathBuf> {
        let now = clock::now();
        let stamp = now
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d-%H%M%S")
            .to_string();
        let path = self.archive_dir.join(format!("vault-{stamp}.otvault"));
        self.vault.archive_and_reset(&mut self.store, &path, now)?;
        cx.emit(LibraryEvent::VaultChanged);
        Ok(path)
    }

    /// Writes an encrypted backup of the (unlocked) vault to `path`. It opens
    /// with the current master password via "Restore from backup".
    pub fn export_vault(&self, path: &std::path::Path) -> Result<()> {
        self.vault.export(&self.store, path, clock::now())?;
        Ok(())
    }

    /// Archived vaults, newest first: (path, archived at, item count).
    pub fn vault_archives(&self) -> Vec<(std::path::PathBuf, ottero_core::Timestamp, usize)> {
        let Ok(entries) = std::fs::read_dir(&self.archive_dir) else {
            return Vec::new();
        };
        let mut out: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| crate::vault::ARCHIVE_EXTENSIONS.iter().any(|e| x == *e))
            })
            .filter_map(|p| {
                let a = ArchiveFile::read(&p).ok()?;
                Some((p, a.archived_at(), a.item_count()))
            })
            .collect();
        out.sort_by_key(|(_, t, _)| std::cmp::Reverse(*t));
        out
    }

    /// Opens an archive or backup and adds its items to the current
    /// (unlocked) vault. Resolves to `None` when `secret` doesn't open it.
    pub fn import_vault_archive(
        &mut self,
        path: std::path::PathBuf,
        secret: ArchiveSecret,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<Option<usize>>> {
        if !self.vault.is_unlocked() {
            return gpui_kit::Task::ready(Err(ottero_core::DomainError::VaultState {
                state: self.vault.state().label(),
            }
            .into()));
        }
        // The account key never leaves the main thread's session: opening
        // with it is fast (HKDF + AEAD), so it happens here, not in a task.
        let opened_now = if matches!(secret, ArchiveSecret::Account) {
            let result =
                (|| {
                    let archive = ArchiveFile::read(&path)?;
                    let ArchiveOpener::Account(email) = archive.opener() else {
                        return Ok(None);
                    };
                    let Some(key) = self.account.key().filter(|_| {
                        self.account.signed_in_email().as_deref() == Some(email.as_str())
                    }) else {
                        return Ok(None);
                    };
                    VaultSession::open_archive_with_account(&archive, key)
                })();
            Some(result)
        } else {
            None
        };
        cx.spawn(async move |this, cx| {
            let items = match opened_now {
                Some(result) => result?,
                None => {
                    cx.background_executor()
                        .spawn(async move {
                            let archive = ArchiveFile::read(&path)?;
                            match secret {
                                ArchiveSecret::Password(password) => {
                                    let k = archive.kdf()?;
                                    let kek = derive(&password, &k.salt, k.params)?;
                                    VaultSession::open_archive(&archive, &kek)
                                }
                                ArchiveSecret::RecoveryKey(text) => {
                                    let Some(normalized) = recovery::normalize(&text) else {
                                        return Ok(None);
                                    };
                                    let k = archive.recovery_kdf()?;
                                    let rkek = derive(
                                        &SecretString::from(normalized.to_string()),
                                        &k.salt,
                                        k.params,
                                    )?;
                                    VaultSession::open_archive_with_recovery(&archive, &rkek)
                                }
                                ArchiveSecret::Account => Ok(None),
                            }
                        })
                        .await?
                }
            };
            let Some(items) = items else {
                return Ok(None);
            };
            this.update(cx, |lib, cx| {
                // New IDs, so importing twice can't overwrite live items.
                // Items identical to one already here are skipped, so
                // restoring the same backup twice doesn't double the vault.
                let now = clock::now();
                let copies: Vec<VaultItem> = items
                    .iter()
                    .filter(|i| {
                        !lib.vault
                            .items()
                            .iter()
                            .any(|live| !live.is_trashed() && live.fields() == i.fields())
                    })
                    .filter_map(|i| {
                        let mut copy = VaultItem::new(i.fields(), now).ok()?;
                        if i.is_favorite() {
                            copy.set_favorite(true, now).ok()?;
                        }
                        Some(copy)
                    })
                    .collect();
                let imported_ids: Vec<VaultItemId> = copies.iter().map(VaultItem::id).collect();
                let n = copies.len();
                lib.vault.put(&mut lib.store, copies)?;
                for id in imported_ids {
                    lib.record_vault_sync(id);
                }
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(Some(n))
            })?
        })
    }

    /// Records that a value was copied from `id` ("Recently used").
    pub fn mark_vault_used(&mut self, id: VaultItemId, cx: &mut Context<Self>) {
        let now = clock::now();
        let result = self.vault.update(&mut self.store, id, |i| {
            i.mark_used(now);
            Ok(true)
        });
        if result.is_ok() {
            self.touch_vault();
            cx.emit(LibraryEvent::VaultChanged);
        }
    }

    pub fn vault_tags(&self) -> Vec<(String, usize)> {
        self.vault.tags()
    }

    pub fn vault_recent(&self, n: usize) -> Vec<VaultItemId> {
        self.vault.recent(n)
    }

    /// Unlocks; resolves to `false` for a wrong password.
    pub fn unlock_vault(
        &mut self,
        password: SecretString,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<bool>> {
        let input = match self.vault.start_unlock(&self.store) {
            Ok(i) => i,
            Err(err) => return gpui_kit::Task::ready(Err(err)),
        };
        cx.emit(LibraryEvent::VaultChanged);
        cx.spawn(async move |this, cx| {
            let kek = cx
                .background_executor()
                .spawn(async move { derive(&password, &input.salt, input.params) })
                .await;
            this.update(cx, |lib, cx| {
                let ok = lib.vault.finish_unlock(&lib.store, kek)?;
                if ok {
                    lib.touch_vault();
                }
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(ok)
            })?
        })
    }

    /// Locks Passwords and, when signed in, the account key too: with a
    /// linked vault the account key opens Passwords, so leaving it open
    /// would make the lock meaningless (ADR 0014). Manual lock, idle,
    /// sleep and "lock in background" all come through here.
    pub fn lock_vault(&mut self, cx: &mut Context<Self>) {
        self.stop_sync_ws();
        self.lock_vault_only(cx);
        self.lock_account_key(cx);
    }

    /// Locks the vault and nothing else (the account side calls this when
    /// it locks or signs out).
    pub(crate) fn lock_vault_only(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.vault.state(),
            LockState::Unlocked | LockState::Unlocking
        ) {
            self.vault.lock();
            cx.emit(LibraryEvent::VaultChanged);
        }
    }

    /// What opens the vault on this device (`None` before setup).
    pub fn vault_opener(&self) -> Option<Opener> {
        self.vault.opener()
    }

    /// The account a linked vault opens with.
    pub fn vault_linked_email(&self) -> Option<&str> {
        self.vault.linked_email()
    }

    /// Why the open account didn't open the linked vault, if it tried.
    pub fn vault_account_problem(&self) -> Option<&AccountUnlock> {
        self.vault_account_problem.as_ref()
    }

    /// The signed-in account's email and session epoch, when its key is open.
    /// Whether the Ottero account key is currently unlocked in memory.
    #[must_use]
    pub fn account_key_open(&self) -> bool {
        self.account.key().is_some()
    }

    fn open_account(&self) -> Option<(String, u64)> {
        self.account.key()?;
        Some((self.account.signed_in_email()?, self.account.epoch()))
    }

    /// The account key just opened (sign-in, unlock, a password check):
    /// a locked vault linked to this account opens with it, so one unlock
    /// opens both. Also starts auto-sync + live WebSocket notices when enabled.
    pub(crate) fn account_key_opened(&mut self, cx: &mut Context<Self>) {
        self.touch_vault();
        self.vault_account_problem = None;
        if self.vault.state() == LockState::Locked
            && self.vault.opener() == Some(Opener::Account)
            && let (Some(key), Some(email)) = (self.account.key(), self.account.signed_in_email())
        {
            match self.vault.unlock_with_account(&self.store, &email, key) {
                Ok(AccountUnlock::Opened) => {}
                Ok(problem) => self.vault_account_problem = Some(problem),
                Err(err) => eprintln!("ottero: couldn't open the vault with the account: {err:#}"),
            }
        }
        cx.emit(LibraryEvent::VaultChanged);
        self.on_account_unlocked_for_sync(cx);
    }

    /// The account key is gone (locked, signed out, session ended).
    pub(crate) fn account_key_closed(&mut self, cx: &mut Context<Self>) {
        self.stop_sync_ws();
        self.vault_account_problem = None;
        self.lock_vault_only(cx);
        cx.emit(LibraryEvent::VaultChanged);
    }

    /// Sets the vault up under the open account key (no master password and no
    /// redundant second recovery key — the Account Emergency Kit covers recovery).
    pub fn set_up_vault_with_account(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<()>> {
        let Some((email, _epoch)) = self.open_account() else {
            return gpui_kit::Task::ready(Err(anyhow::anyhow!(
                "Unlock your Ottero account first."
            )));
        };
        let Some(key) = self.account.key() else {
            return gpui_kit::Task::ready(Err(anyhow::anyhow!(
                "Unlock your Ottero account first."
            )));
        };
        let res =
            self.vault
                .finish_setup_with_account(&mut self.store, &email, key, None, clock::now());
        if res.is_ok() {
            self.touch_vault();
            self.record_vault_meta_sync(cx);
            cx.emit(LibraryEvent::VaultChanged);
        }
        gpui_kit::Task::ready(res)
    }

    /// Links a master-password vault to the open account: checks
    /// `master_password` in the background, then re-wraps the vault key
    /// under the account key and retires the master password in one
    /// transaction. Any existing local recovery key is kept; no second
    /// recovery key is forced. A locked vault ends up unlocked. Results that
    /// arrive after the account changed, or after the vault was locked, are
    /// dropped and change nothing.
    pub fn link_vault_to_account(
        &mut self,
        master_password: SecretString,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<(LinkResult, Option<Zeroizing<String>>)>> {
        let Some((email, account_epoch)) = self.open_account() else {
            return gpui_kit::Task::ready(Err(anyhow::anyhow!(
                "Unlock your Ottero account first."
            )));
        };
        let (input, vault_epoch) = match self.vault.start_link(&self.store) {
            Ok(v) => v,
            Err(err) => return gpui_kit::Task::ready(Err(err)),
        };
        cx.emit(LibraryEvent::VaultChanged);
        cx.spawn(async move |this, cx| {
            let kek = cx
                .background_executor()
                .spawn(async move { derive(&master_password, &input.salt, input.params) })
                .await;
            this.update(cx, |lib, cx| {
                let still = lib.account.epoch() == account_epoch
                    && lib.account.signed_in_email().as_deref() == Some(email.as_str());
                let Some(key) = lib.account.key().filter(|_| still) else {
                    lib.vault.cancel_link(vault_epoch);
                    cx.emit(LibraryEvent::VaultChanged);
                    return anyhow::Ok((LinkResult::Stale, None));
                };
                let result = lib.vault.finish_link(
                    &mut lib.store,
                    vault_epoch,
                    kek,
                    crate::vault::LinkAccount { email: &email, key },
                    None,
                    clock::now(),
                );
                cx.emit(LibraryEvent::VaultChanged);
                let result = result?;
                if result == LinkResult::Linked {
                    lib.vault_account_problem = None;
                    lib.touch_vault();
                    lib.record_vault_meta_and_items_sync(cx);
                }
                anyhow::Ok((result, None))
            })?
        })
    }

    /// Changes the master password; resolves to `false` if `current` is wrong.
    pub fn change_master_password(
        &mut self,
        current: SecretString,
        new: SecretString,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<Result<bool>> {
        let (current_input, new_input) = match (
            VaultSession::current_kdf(&self.store),
            KdfInput::fresh(self.kdf),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return gpui_kit::Task::ready(Err(e)),
        };
        cx.spawn(async move |this, cx| {
            let (old_kek, new_kek) = cx
                .background_executor()
                .spawn(async move {
                    anyhow::Ok((
                        derive(&current, &current_input.salt, current_input.params)?,
                        derive(&new, &new_input.salt, new_input.params)?,
                    ))
                })
                .await?;
            this.update(cx, |lib, cx| {
                let ok = lib.vault.change_password(
                    &mut lib.store,
                    &old_kek,
                    new_input,
                    &new_kek,
                    clock::now(),
                )?;
                cx.emit(LibraryEvent::VaultChanged);
                anyhow::Ok(ok)
            })?
        })
    }

    pub fn vault_list(
        &mut self,
        filter: VaultFilter,
        tag: Option<&str>,
        query: &str,
        sort: VaultSort,
    ) -> Vec<VaultItemId> {
        self.vault.list(filter, tag, query, sort, clock::now())
    }

    pub fn vault_item(&self, id: VaultItemId) -> Option<VaultItem> {
        self.vault.item(id).cloned()
    }

    fn record_vault_sync(&mut self, id: VaultItemId) {
        // Only sync vault items when the vault is linked to the signed-in
        // Ottero Cloud account; a master-password-only vault cannot be
        // decrypted on other devices without its account-wrapped vault key.
        if self.vault.opener() != Some(Opener::Account) {
            return;
        }
        if let Ok(Some(sealed)) = self.store.vault_item(id) {
            self.record_sync(
                ottero_sync_protocol::Collection::Vault,
                &id.to_string(),
                &["record"],
                false,
                &crate::sync::SyncVaultPayload::from(&sealed),
            );
        }
    }

    pub fn create_vault_item(
        &mut self,
        fields: &ItemFields,
        cx: &mut Context<Self>,
    ) -> Result<VaultItemId> {
        let id = self.vault.create(&mut self.store, fields, clock::now())?;
        self.record_vault_sync(id);
        self.touch_vault();
        cx.emit(LibraryEvent::VaultChanged);
        Ok(id)
    }

    pub fn update_vault_item(
        &mut self,
        id: VaultItemId,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut VaultItem) -> ottero_core::DomainResult<bool>,
    ) -> Result<()> {
        self.vault.update(&mut self.store, id, change)?;
        self.record_vault_sync(id);
        self.touch_vault();
        cx.emit(LibraryEvent::VaultChanged);
        Ok(())
    }

    pub fn duplicate_vault_item(
        &mut self,
        id: VaultItemId,
        cx: &mut Context<Self>,
    ) -> Result<VaultItemId> {
        let copy = self
            .vault
            .item(id)
            .context("That item no longer exists")?
            .duplicate(clock::now())?;
        let new_id = copy.id();
        self.vault.put(&mut self.store, vec![copy])?;
        self.record_vault_sync(new_id);
        cx.emit(LibraryEvent::VaultChanged);
        Ok(new_id)
    }

    pub fn delete_vault_items(
        &mut self,
        ids: &[VaultItemId],
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.vault.delete(&mut self.store, ids)?;
        if self.vault.opener() == Some(Opener::Account) {
            for id in ids {
                self.record_sync(
                    ottero_sync_protocol::Collection::Vault,
                    &id.to_string(),
                    &["record"],
                    true,
                    &serde_json::json!({}),
                );
            }
        }
        cx.emit(LibraryEvent::VaultChanged);
        Ok(())
    }

    /// Imports parsed rows; returns how many were added.
    pub fn import_vault_items(
        &mut self,
        rows: Vec<(ItemFields, bool)>,
        cx: &mut Context<Self>,
    ) -> Result<usize> {
        let now = clock::now();
        let mut items = Vec::with_capacity(rows.len());
        for (fields, favorite) in rows {
            if let Ok(mut item) = VaultItem::new(&fields, now) {
                if favorite {
                    item.set_favorite(true, now)?;
                }
                items.push(item);
            }
        }
        let ids: Vec<VaultItemId> = items.iter().map(VaultItem::id).collect();
        let n = items.len();
        self.vault.put(&mut self.store, items)?;
        for id in ids {
            self.record_vault_sync(id);
        }
        cx.emit(LibraryEvent::VaultChanged);
        Ok(n)
    }

    pub fn vault_health(&mut self) -> ottero_core::vault::health::Report {
        self.vault.health(clock::now())
    }

    pub fn vault_strength(
        &mut self,
        id: VaultItemId,
    ) -> Option<ottero_core::vault::health::Strength> {
        self.vault.strength_of(id)
    }
}

/// A fresh recovery key, formatted for the user.
fn new_recovery_key() -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new([0u8; recovery::KEY_BYTES]);
    ottero_crypto::random_bytes(&mut bytes[..]).context("Couldn't get secure random numbers")?;
    Ok(recovery::format(&bytes))
}
