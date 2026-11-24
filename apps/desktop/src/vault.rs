//! The unlocked vault in memory, and the crypto glue between the domain
//! (`ottero_core::vault`), the crypto crate and storage.
//!
//! Key hierarchy (ADR 0005): master password → Argon2id → KEK → unwraps the
//! random vault key → seals each item (AAD = item id). The same vault key is
//! also wrapped by the recovery key (ADR 0006) and, once the vault is linked
//! to an Ottero Cloud account, by a key derived from the account key
//! (`ottero_crypto::vault_link`, ADR 0014), which then replaces the master
//! password. Linking re-wraps the key only; no item is re-encrypted. The slow step,
//! [`derive`], is a free function so callers run it on a background thread;
//! everything else here is fast and runs on the main thread.
//!
//! State follows `ottero_core::vault::lock::next`, and what opens the vault
//! follows `next_opener`. Locking drops the vault
//! key and every decrypted item (both zero their memory on drop).

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use ottero_core::vault::health::{self, Report, Strength};
use ottero_core::vault::lock::{LockEvent, LockState, Opener, OpenerEvent, next, next_opener};
use ottero_core::vault::search::Query;
use ottero_core::vault::{ItemFields, ItemKind, VaultItem};
use ottero_core::{DomainError, Timestamp, VaultItemId};
use ottero_crypto::{
    CryptoError, KdfParams, Salt, SecretKey, SecretString, Zeroizing, derive_key, open, seal,
    vault_link,
};
use ottero_storage::{AccountWrap, LinkOutcome, MasterWrap, SealedItem, Store, VaultMeta};

/// Associated data binding the wrapped vault key to its purpose.
// Format identifiers from the Keepstone days, kept through the Calmfold and
// Ottero renames. Never change them: existing vaults and archives are bound
// to these exact bytes.
const KEY_AAD: &[u8] = b"keepstone-vault-key-v1";
/// The same vault key wrapped by the recovery key (ADR 0006).
const RECOVERY_AAD: &[u8] = b"keepstone-vault-recovery-v1";
/// Argon2 cost for recovery keys. The key has 160 random bits, so it needs
/// no stretching for strength; the minimum cost keeps recovery quick.
pub const RECOVERY_KDF: KdfParams = KdfParams::MINIMUM;
const META_VERSION: u32 = 1;

/// Runs Argon2id. Slow by design: call from a background thread.
pub fn derive(password: &SecretString, salt: &Salt, params: KdfParams) -> Result<SecretKey> {
    derive_key(password, salt, params).map_err(|_| anyhow::anyhow!("Couldn't derive the key."))
}

/// What setup and password changes need to derive on the background thread.
#[derive(Clone, Copy)]
pub struct KdfInput {
    pub salt: Salt,
    pub params: KdfParams,
}

impl KdfInput {
    /// A fresh salt with `params` (the app uses `KdfParams::RECOMMENDED`).
    pub fn fresh(params: KdfParams) -> Result<Self> {
        Ok(Self {
            salt: Salt::generate().context("Couldn't get random bytes")?,
            params,
        })
    }
}

/// Item list filter (the sidebar children of Passwords).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VaultFilter {
    #[default]
    All,
    Favorites,
    Kind(ItemKind),
    /// Items with a one-time code (the authenticator view).
    Codes,
    /// Items flagged by password health.
    Health,
    Trash,
}

/// List order when not searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSort {
    #[default]
    Name,
    RecentlyUsed,
    RecentlyEdited,
    Created,
}

impl VaultSort {
    pub const ALL: [Self; 4] = [
        Self::Name,
        Self::RecentlyUsed,
        Self::RecentlyEdited,
        Self::Created,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::RecentlyUsed => "Recently used",
            Self::RecentlyEdited => "Recently edited",
            Self::Created => "Date created",
        }
    }
}

/// What opens the vault after recovery (ADR 0006, 0014).
pub enum NewOpener<'a> {
    /// A new master password (not signed in, or the account is locked).
    MasterPassword(KdfInput, &'a SecretKey),
    /// The Ottero Cloud account that is open now.
    Account {
        email: &'a str,
        account_key: &'a SecretKey,
    },
}

/// Result of [`VaultSession::unlock_with_account`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountUnlock {
    Opened,
    /// The vault is linked to another account (its email).
    OtherAccount(String),
    /// Same email, but this account key doesn't open it (a re-created
    /// account, or a damaged header). Nothing was changed.
    WrongKey,
}

/// The Ottero Cloud account a link is for: its normalized email and the
/// open account key (borrowed from the account session, never copied).
#[derive(Clone, Copy)]
pub struct LinkAccount<'a> {
    pub email: &'a str,
    pub key: &'a SecretKey,
}

/// Result of [`VaultSession::finish_link`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkResult {
    Linked,
    WrongPassword,
    /// Locked, quit or changed while the password was being checked:
    /// nothing was written.
    Stale,
}

/// Where a link started (it must still be there when it finishes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingLink {
    from_locked: bool,
    epoch: u64,
}

pub struct VaultSession {
    state: LockState,
    /// What opens the vault on this device; `None` before setup.
    opener: Option<Opener>,
    /// The account a linked vault opens with (its normalized email).
    linked_email: Option<String>,
    /// Bumped on every lock, so a slow link or unlock that started before
    /// it can't land afterwards.
    epoch: u64,
    linking: Option<PendingLink>,
    key: Option<SecretKey>,
    items: Vec<VaultItem>,
    /// Items that failed to decrypt (shown as a warning, never deleted).
    unreadable: usize,
    /// Strength per item, invalidated when the password changes.
    strength: HashMap<VaultItemId, (u64, Strength)>,
}

impl VaultSession {
    /// The state on launch: no header → setup, otherwise locked.
    pub fn load(store: &Store) -> Result<Self> {
        let meta = store.vault_meta()?;
        let mut this = Self::locked();
        match meta {
            None => this.state = LockState::Uninitialized,
            Some(meta) => this.read_opener(&meta),
        }
        Ok(this)
    }

    /// A locked session (used when the header can't be read).
    #[must_use]
    pub fn locked() -> Self {
        Self {
            state: LockState::Locked,
            opener: None,
            linked_email: None,
            epoch: 0,
            linking: None,
            key: None,
            items: Vec::new(),
            unreadable: 0,
            strength: HashMap::new(),
        }
    }

    fn read_opener(&mut self, meta: &VaultMeta) {
        match &meta.account {
            Some(a) => {
                self.opener = Some(Opener::Account);
                self.linked_email = Some(a.email.clone());
            }
            None => {
                self.opener = meta.master.as_ref().map(|_| Opener::MasterPassword);
                self.linked_email = None;
            }
        }
    }

    #[must_use]
    pub fn state(&self) -> LockState {
        self.state
    }

    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.state == LockState::Unlocked
    }

    /// What opens the vault on this device.
    #[must_use]
    pub fn opener(&self) -> Option<Opener> {
        self.opener
    }

    /// The account a linked vault opens with.
    #[must_use]
    pub fn linked_email(&self) -> Option<&str> {
        self.linked_email.as_deref()
    }

    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }

    fn transition(&mut self, event: LockEvent) -> Result<()> {
        match next(self.state, event) {
            Some(to) => {
                self.state = to;
                Ok(())
            }
            None => Err(self.state_error()),
        }
    }

    /// Checks an opener change against `ottero_core::vault::lock::next_opener`.
    fn opener_allowed(&self, event: OpenerEvent) -> Result<Option<Opener>> {
        next_opener(self.state, self.opener, event).ok_or_else(|| self.state_error())
    }

    fn state_error(&self) -> anyhow::Error {
        DomainError::VaultState {
            state: self.state.label(),
        }
        .into()
    }

    fn key(&self) -> Result<&SecretKey> {
        self.key.as_ref().ok_or_else(|| self.state_error())
    }

    fn master_only(&self) -> Result<()> {
        if self.opener == Some(Opener::Account) {
            bail!("Passwords opens with your Ottero account, not a master password.");
        }
        Ok(())
    }

    // ---- Setup / unlock / lock ------------------------------------------

    /// Finishes setup with the KEK derived from the new master password and
    /// the one derived from the new recovery key.
    pub fn finish_setup(
        &mut self,
        store: &mut Store,
        input: KdfInput,
        kek: &SecretKey,
        recovery: Option<(KdfInput, &SecretKey)>,
        now: Timestamp,
    ) -> Result<()> {
        if store.vault_meta()?.is_some() {
            bail!("A vault already exists.");
        }
        let opener = self.opener_allowed(OpenerEvent::SetUp(Opener::MasterPassword))?;
        let vault_key = SecretKey::generate().context("Couldn't get random bytes")?;
        let master = master_wrap(input, kek, &vault_key)?;
        let (recovery_salt, recovery_wrapped) = recovery_wrap(recovery, &vault_key)?;
        let at = now.as_millisecond();
        store.save_vault_meta(&VaultMeta {
            version: META_VERSION,
            master: Some(master),
            created_at: at,
            updated_at: at,
            recovery_salt,
            recovery_wrapped,
            account: None,
        })?;
        self.transition(LockEvent::SetUp)?;
        self.opener = opener;
        self.linked_email = None;
        self.key = Some(vault_key);
        self.items.clear();
        Ok(())
    }

    /// Sets the vault up under an Ottero Cloud account key: no master
    /// password. The recovery key is required, so the vault never depends
    /// on the account alone.
    pub fn finish_setup_with_account(
        &mut self,
        store: &mut Store,
        email: &str,
        account_key: &SecretKey,
        recovery: (KdfInput, &SecretKey),
        now: Timestamp,
    ) -> Result<()> {
        if store.vault_meta()?.is_some() {
            bail!("A vault already exists.");
        }
        let opener = self.opener_allowed(OpenerEvent::SetUp(Opener::Account))?;
        let vault_key = SecretKey::generate().context("Couldn't get random bytes")?;
        let account = account_wrap(email, account_key, &vault_key)?;
        let (recovery_salt, recovery_wrapped) = recovery_wrap(Some(recovery), &vault_key)?;
        let at = now.as_millisecond();
        store.save_vault_meta(&VaultMeta {
            version: META_VERSION,
            master: None,
            created_at: at,
            updated_at: at,
            recovery_salt,
            recovery_wrapped,
            account: Some(account),
        })?;
        self.transition(LockEvent::SetUp)?;
        self.opener = opener;
        self.linked_email = Some(email.to_owned());
        self.key = Some(vault_key);
        self.items.clear();
        Ok(())
    }

    /// Whether a recovery key was set up for this vault.
    pub fn has_recovery(store: &Store) -> bool {
        store
            .vault_meta()
            .ok()
            .flatten()
            .is_some_and(|m| m.recovery_wrapped.is_some() && m.recovery_salt.is_some())
    }

    /// The recovery key's KDF input (fixed cost: the key itself is random).
    pub fn recovery_kdf(store: &Store) -> Result<KdfInput> {
        let meta = store.vault_meta()?.context("No vault is set up")?;
        recovery_input(meta.recovery_salt.as_deref())
    }

    /// Replaces the recovery key (the old one stops working). Unlocked only.
    pub fn set_recovery(
        &mut self,
        store: &mut Store,
        input: KdfInput,
        rkek: &SecretKey,
        now: Timestamp,
    ) -> Result<()> {
        let key = self.key()?;
        let mut meta = store.vault_meta()?.context("No vault is set up")?;
        meta.recovery_salt = Some(input.salt.0.to_vec());
        meta.recovery_wrapped =
            Some(seal(rkek, key.expose(), RECOVERY_AAD).context("Couldn't encrypt the vault key")?);
        meta.updated_at = now.as_millisecond();
        store.save_vault_meta(&meta)?;
        Ok(())
    }

    /// Opens the vault with the recovery key's KEK, then makes `new` what
    /// opens it from now on (a new master password, or the open account).
    /// The other opener is removed in the same write. `Ok(false)`: wrong
    /// recovery key, still locked, nothing changed.
    pub fn recover(
        &mut self,
        store: &mut Store,
        rkek: Result<SecretKey>,
        new: NewOpener<'_>,
        now: Timestamp,
    ) -> Result<bool> {
        let target = match new {
            NewOpener::MasterPassword(..) => Opener::MasterPassword,
            NewOpener::Account { .. } => Opener::Account,
        };
        let opener = self.opener_allowed(OpenerEvent::Recover(target))?;
        let mut meta = store.vault_meta()?.context("No vault is set up")?;
        let opened = rkek
            .ok()
            .and_then(|rkek| open(&rkek, meta.recovery_wrapped.as_deref()?, RECOVERY_AAD).ok());
        let Some(key) = opened.and_then(|b| key_from(&b)) else {
            return Ok(false);
        };
        match new {
            NewOpener::MasterPassword(input, kek) => {
                meta.master = Some(master_wrap(input, kek, &key)?);
                meta.account = None;
            }
            NewOpener::Account { email, account_key } => {
                meta.account = Some(account_wrap(email, account_key, &key)?);
                meta.master = None;
            }
        }
        meta.updated_at = now.as_millisecond();
        store.save_vault_meta(&meta)?;
        self.transition(LockEvent::StartUnlock)?;
        self.opener = opener;
        self.read_opener(&meta);
        if let Err(err) = self.load_items(store, &key) {
            _ = self.transition(LockEvent::Failed);
            return Err(err);
        }
        self.key = Some(key);
        self.transition(LockEvent::Unlocked)?;
        Ok(true)
    }

    /// "Start a new vault": writes the current encrypted vault to `archive`
    /// (still encrypted; opens later with its old password, its account or
    /// its recovery key), then empties the vault so setup runs again.
    /// Locked only, and never deletes before the archive is safely on disk.
    pub fn archive_and_reset(
        &mut self,
        store: &mut Store,
        archive: &std::path::Path,
        now: Timestamp,
    ) -> Result<()> {
        if self.state != LockState::Locked {
            return Err(self.state_error());
        }
        let opener = self.opener_allowed(OpenerEvent::Reset)?;
        write_archive(store, archive, now)?;
        store.reset_vault()?;
        self.transition(LockEvent::Reset)?;
        self.opener = opener;
        self.linked_email = None;
        Ok(())
    }

    /// "Export encrypted backup": the same file format as an archive,
    /// written while unlocked and left in place. It opens with the master
    /// password the vault has now, or (linked vault) with its account or its
    /// recovery key. Returns the number of sealed items.
    pub fn export(&self, store: &Store, path: &std::path::Path, now: Timestamp) -> Result<usize> {
        if self.state != LockState::Unlocked {
            return Err(self.state_error());
        }
        write_archive(store, path, now)
    }

    /// Where to derive from for unlocking (moves the vault to `Unlocking`).
    pub fn start_unlock(&mut self, store: &Store) -> Result<KdfInput> {
        self.master_only()?;
        let meta = store.vault_meta()?.context("No vault is set up")?;
        let input = kdf_input(meta.master.as_ref().context("No master password is set")?)?;
        self.transition(LockEvent::StartUnlock)?;
        Ok(input)
    }

    /// Finishes unlocking with the derived KEK. A wrong password returns
    /// `Ok(false)` and the vault stays locked.
    pub fn finish_unlock(&mut self, store: &Store, kek: Result<SecretKey>) -> Result<bool> {
        if self.state != LockState::Unlocking || self.linking.is_some() {
            // Locked again (or quit) while deriving: ignore the stale result.
            return Ok(false);
        }
        let opened = kek.ok().and_then(|kek| {
            let meta = store.vault_meta().ok().flatten()?;
            open(&kek, &meta.master?.wrapped_key, KEY_AAD).ok()
        });
        let Some(key) = opened.and_then(|b| key_from(&b)) else {
            self.transition(LockEvent::Failed)?;
            return Ok(false);
        };
        if let Err(err) = self.load_items(store, &key) {
            _ = self.transition(LockEvent::Failed);
            return Err(err);
        }
        self.key = Some(key);
        self.transition(LockEvent::Unlocked)?;
        Ok(true)
    }

    /// Opens a linked vault with the account key (fast: HKDF + one AEAD,
    /// no Argon2). Locked only; a vault linked to another account, or a key
    /// that doesn't open it, leaves everything as it was.
    pub fn unlock_with_account(
        &mut self,
        store: &Store,
        email: &str,
        account_key: &SecretKey,
    ) -> Result<AccountUnlock> {
        if self.state != LockState::Locked || self.opener != Some(Opener::Account) {
            return Err(self.state_error());
        }
        let meta = store.vault_meta()?.context("No vault is set up")?;
        let wrap = meta.account.as_ref().context("This vault isn't linked")?;
        if wrap.email != email {
            return Ok(AccountUnlock::OtherAccount(wrap.email.clone()));
        }
        let Ok(key) = vault_link::unwrap_vault_key(account_key, &wrap.wrapped_key) else {
            return Ok(AccountUnlock::WrongKey);
        };
        self.transition(LockEvent::StartUnlock)?;
        if let Err(err) = self.load_items(store, &key) {
            self.transition(LockEvent::Failed)?;
            return Err(err);
        }
        self.key = Some(key);
        self.transition(LockEvent::Unlocked)?;
        Ok(AccountUnlock::Opened)
    }

    /// Starts linking a master-password vault to an account: returns what
    /// to derive the master password's KEK from, and the token the result
    /// must bring back. A locked vault moves to `Unlocking`.
    pub fn start_link(&mut self, store: &Store) -> Result<(KdfInput, u64)> {
        self.opener_allowed(OpenerEvent::Link)?;
        if self.linking.is_some() {
            bail!("Already linking.");
        }
        let meta = store.vault_meta()?.context("No vault is set up")?;
        let input = kdf_input(meta.master.as_ref().context("No master password is set")?)?;
        let from_locked = self.state == LockState::Locked;
        if from_locked {
            self.transition(LockEvent::StartUnlock)?;
        }
        self.linking = Some(PendingLink {
            from_locked,
            epoch: self.epoch,
        });
        Ok((input, self.epoch))
    }

    /// Abandons a link that won't finish (the account changed meanwhile).
    pub fn cancel_link(&mut self, epoch: u64) {
        if let Some(pending) = self.linking.filter(|p| p.epoch == epoch) {
            self.linking = None;
            if pending.from_locked && self.state == LockState::Unlocking {
                _ = self.transition(LockEvent::Failed);
            }
        }
    }

    /// Finishes linking: the master password's KEK must open the vault key;
    /// the vault key is then wrapped for `account_key`, the wrapping is
    /// opened again to prove it, and one transaction stores it and retires
    /// the master password (`new_recovery` first, when the vault has no
    /// recovery key). No item is re-encrypted. A locked vault ends up
    /// unlocked. Anything short of the commit leaves the master password
    /// working.
    pub fn finish_link(
        &mut self,
        store: &mut Store,
        epoch: u64,
        kek: Result<SecretKey>,
        account: LinkAccount<'_>,
        new_recovery: Option<(KdfInput, &SecretKey)>,
        now: Timestamp,
    ) -> Result<LinkResult> {
        let LinkAccount {
            email,
            key: account_key,
        } = account;
        let Some(pending) = self
            .linking
            .filter(|p| p.epoch == epoch && self.epoch == epoch)
        else {
            return Ok(LinkResult::Stale);
        };
        let expected = if pending.from_locked {
            LockState::Unlocking
        } else {
            LockState::Unlocked
        };
        if self.state != expected {
            self.linking = None;
            return Ok(LinkResult::Stale);
        }
        let outcome = self.write_link(store, kek, email, account_key, new_recovery, now);
        self.linking = None;
        match outcome {
            Ok(Some(key)) => {
                self.opener = Some(Opener::Account);
                self.linked_email = Some(email.to_owned());
                if pending.from_locked {
                    if let Err(err) = self.load_items(store, &key) {
                        self.transition(LockEvent::Failed)?;
                        return Err(err);
                    }
                    self.key = Some(key);
                    self.transition(LockEvent::Unlocked)?;
                }
                Ok(LinkResult::Linked)
            }
            Ok(None) => {
                if pending.from_locked {
                    self.transition(LockEvent::Failed)?;
                }
                Ok(LinkResult::WrongPassword)
            }
            Err(err) => {
                if pending.from_locked {
                    self.transition(LockEvent::Failed)?;
                }
                Err(err)
            }
        }
    }

    /// The link's crypto and its one write. `Ok(None)`: wrong password.
    fn write_link(
        &self,
        store: &mut Store,
        kek: Result<SecretKey>,
        email: &str,
        account_key: &SecretKey,
        new_recovery: Option<(KdfInput, &SecretKey)>,
        now: Timestamp,
    ) -> Result<Option<SecretKey>> {
        let meta = store.vault_meta()?.context("No vault is set up")?;
        let master = meta.master.as_ref().context("No master password is set")?;
        let opened = kek
            .ok()
            .and_then(|kek| open(&kek, &master.wrapped_key, KEY_AAD).ok());
        let Some(vault_key) = opened.and_then(|b| key_from(&b)) else {
            return Ok(None);
        };
        if let Some(live) = &self.key
            && live.expose() != vault_key.expose()
        {
            bail!("The vault changed while linking; nothing was changed.");
        }
        let account = account_wrap(email, account_key, &vault_key)?;
        let recovery = match (&meta.recovery_wrapped, new_recovery) {
            (Some(_), _) if meta.recovery_salt.is_some() => None,
            (_, Some((input, rkek))) => Some((
                input.salt.0.to_vec(),
                seal(rkek, vault_key.expose(), RECOVERY_AAD)
                    .context("Couldn't encrypt the vault key")?,
            )),
            _ => bail!("This vault needs a recovery key before it can be linked."),
        };
        let outcome = store.link_vault_to_account(
            &master.wrapped_key,
            &account,
            recovery.as_ref().map(|(s, w)| (s.as_slice(), w.as_slice())),
            now.as_millisecond(),
        )?;
        if outcome != LinkOutcome::Linked {
            bail!("The vault changed while linking; nothing was changed.");
        }
        Ok(Some(vault_key))
    }

    /// Re-reads and decrypts items from `store` when unlocked (e.g. after sync).
    pub fn reload_if_unlocked(&mut self, store: &Store) -> Result<()> {
        if self.state != LockState::Unlocked {
            return Ok(());
        }
        let Some(ref key) = self.key else {
            return Ok(());
        };
        let mut raw = Zeroizing::new([0u8; SecretKey::LEN]);
        raw.copy_from_slice(key.expose());
        let copy = SecretKey::from_bytes(&mut raw);
        self.load_items(store, &copy)
    }

    fn load_items(&mut self, store: &Store, key: &SecretKey) -> Result<()> {
        let sealed = store.vault_items()?;
        let mut items = Vec::with_capacity(sealed.len());
        let mut unreadable = 0;
        for row in sealed {
            let item = open(key, &row.blob, row.id.to_string().as_bytes())
                .map_err(|_| DomainError::Corrupt)
                .and_then(|json| VaultItem::from_json(&json, row.id));
            match item {
                Ok(item) => items.push(item),
                Err(_) => unreadable += 1,
            }
        }
        sort(&mut items);
        self.items = items;
        self.unreadable = unreadable;
        self.strength.clear();
        Ok(())
    }

    /// Drops the key and every decrypted item. Pending unlocks and links
    /// that started before this can no longer land.
    pub fn lock(&mut self) {
        if self.transition(LockEvent::Lock).is_ok() {
            self.epoch = self.epoch.wrapping_add(1);
            self.linking = None;
            self.key = None;
            self.items.clear();
            self.strength.clear();
            self.unreadable = 0;
        }
    }

    /// Checks the current password (the KEK derived from it) and re-wraps
    /// the vault key under `new_kek`. Items are untouched.
    pub fn change_password(
        &mut self,
        store: &mut Store,
        current_kek: &SecretKey,
        new_input: KdfInput,
        new_kek: &SecretKey,
        now: Timestamp,
    ) -> Result<bool> {
        self.master_only()?;
        let key = self.key()?;
        let mut meta = store.vault_meta()?.context("No vault is set up")?;
        let master = meta.master.as_ref().context("No master password is set")?;
        if open(current_kek, &master.wrapped_key, KEY_AAD).is_err() {
            return Ok(false);
        }
        meta.master = Some(master_wrap(new_input, new_kek, key)?);
        meta.updated_at = now.as_millisecond();
        store.save_vault_meta(&meta)?;
        Ok(true)
    }

    /// The header's KDF input (to verify the current password).
    pub fn current_kdf(store: &Store) -> Result<KdfInput> {
        let meta = store.vault_meta()?.context("No vault is set up")?;
        kdf_input(meta.master.as_ref().context("No master password is set")?)
    }

    // ---- Items ------------------------------------------------------------

    #[must_use]
    pub fn items(&self) -> &[VaultItem] {
        &self.items
    }

    #[must_use]
    pub fn item(&self, id: VaultItemId) -> Option<&VaultItem> {
        self.items.iter().find(|i| i.id() == id)
    }

    fn seal_item(&self, item: &VaultItem) -> Result<SealedItem> {
        let json = item.to_json()?;
        let blob = seal(self.key()?, &json, item.id().to_string().as_bytes())
            .map_err(|_: CryptoError| anyhow::anyhow!("Couldn't encrypt the item"))?;
        Ok(SealedItem {
            id: item.id(),
            blob,
            updated_at: item.updated_at().as_millisecond(),
        })
    }

    /// Saves new or changed items in one transaction and updates memory.
    pub fn put(&mut self, store: &mut Store, items: Vec<VaultItem>) -> Result<()> {
        let sealed = items
            .iter()
            .map(|i| self.seal_item(i))
            .collect::<Result<Vec<_>>>()?;
        store
            .save_vault_items(&sealed)
            .context("Couldn't save to the vault")?;
        for item in items {
            self.strength.remove(&item.id());
            match self.items.iter_mut().find(|i| i.id() == item.id()) {
                Some(slot) => *slot = item,
                None => self.items.push(item),
            }
        }
        sort(&mut self.items);
        Ok(())
    }

    pub fn create(
        &mut self,
        store: &mut Store,
        fields: &ItemFields,
        now: Timestamp,
    ) -> Result<VaultItemId> {
        self.key()?;
        let item = VaultItem::new(fields, now)?;
        let id = item.id();
        self.put(store, vec![item])?;
        Ok(id)
    }

    /// Applies `change` to one item and saves it if it changed.
    pub fn update(
        &mut self,
        store: &mut Store,
        id: VaultItemId,
        change: impl FnOnce(&mut VaultItem) -> ottero_core::DomainResult<bool>,
    ) -> Result<()> {
        let mut item = self.item(id).context("That item no longer exists")?.clone();
        if change(&mut item)? {
            self.put(store, vec![item])?;
        }
        Ok(())
    }

    /// Deletes items for good (Trash → Delete, Empty trash).
    pub fn delete(&mut self, store: &mut Store, ids: &[VaultItemId]) -> Result<()> {
        self.key()?;
        store.delete_vault_items(ids)?;
        self.items.retain(|i| !ids.contains(&i.id()));
        Ok(())
    }

    /// Items for `filter` (and optional `tag`) matching `query` (search
    /// language in `core::vault::search`). With text, best matches first;
    /// otherwise in `sort` order.
    pub fn list(
        &mut self,
        filter: VaultFilter,
        tag: Option<&str>,
        query: &str,
        sort: VaultSort,
        now: Timestamp,
    ) -> Vec<VaultItemId> {
        let exact = self.list_with(filter, tag, Query::parse(query), sort, now);
        let q = Query::parse(query);
        if !exact.is_empty() || q.words.is_empty() {
            return exact;
        }
        // Nothing matched exactly: allow one typo per word.
        self.list_with(filter, tag, q.with_typos(), sort, now)
    }

    fn list_with(
        &mut self,
        filter: VaultFilter,
        tag: Option<&str>,
        q: Query,
        sort: VaultSort,
        now: Timestamp,
    ) -> Vec<VaultItemId> {
        let needs_health = filter == VaultFilter::Health || q.weak || q.reused || q.old;
        let report = needs_health.then(|| self.health(now));
        let in_report = |id: VaultItemId, weak: bool, reused: bool, old: bool| {
            report.as_ref().is_some_and(|r| {
                (weak && r.weak.contains(&id))
                    || (reused && r.reused.iter().any(|g| g.contains(&id)))
                    || (old && r.old.contains(&id))
            })
        };
        let mut scored: Vec<(u8, usize, VaultItemId)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| match filter {
                VaultFilter::Trash => i.is_trashed(),
                _ if i.is_trashed() => false,
                VaultFilter::All => true,
                VaultFilter::Favorites => i.is_favorite(),
                VaultFilter::Kind(k) => i.kind() == k,
                VaultFilter::Codes => i.otp_setup().is_some(),
                VaultFilter::Health => in_report(i.id(), true, true, true),
            })
            .filter(|(_, i)| tag.is_none_or(|t| i.fields().tags.iter().any(|have| have == t)))
            .filter(|(_, i)| {
                let (w, r, o) = (q.weak, q.reused, q.old);
                !(w || r || o) || in_report(i.id(), w, r, o)
            })
            .filter_map(|(ix, i)| q.score(i).map(|s| (s, ix, i.id())))
            .collect();
        let key = |ix: usize| -> i64 {
            let i = &self.items[ix];
            match sort {
                VaultSort::Name => 0,
                VaultSort::RecentlyUsed => -i.used_at().unwrap_or(i.created_at()).as_millisecond(),
                VaultSort::RecentlyEdited => -i.updated_at().as_millisecond(),
                VaultSort::Created => -i.created_at().as_millisecond(),
            }
        };
        let by_text = !q.words.is_empty();
        // `items` is kept in title order, so the index breaks ties by name.
        scored.sort_by(|a, b| {
            let rank = if by_text {
                b.0.cmp(&a.0)
            } else {
                std::cmp::Ordering::Equal
            };
            rank.then(key(a.1).cmp(&key(b.1))).then(a.1.cmp(&b.1))
        });
        scored.into_iter().map(|(_, _, id)| id).collect()
    }

    /// Up to `n` live items most recently copied from (for "Recent").
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<VaultItemId> {
        let mut used: Vec<&VaultItem> = self
            .items
            .iter()
            .filter(|i| !i.is_trashed() && i.used_at().is_some())
            .collect();
        used.sort_by_key(|i| std::cmp::Reverse(i.used_at()));
        used.into_iter().take(n).map(VaultItem::id).collect()
    }

    /// Tags on live items with their counts, most used first.
    #[must_use]
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for i in self.items.iter().filter(|i| !i.is_trashed()) {
            for t in &i.fields().tags {
                *counts.entry(t.as_str()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, usize)> =
            counts.into_iter().map(|(t, n)| (t.to_owned(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Counts per filter, for sidebar badges.
    #[must_use]
    pub fn count(&self, filter: VaultFilter) -> usize {
        self.items
            .iter()
            .filter(|i| match filter {
                VaultFilter::Trash => i.is_trashed(),
                _ if i.is_trashed() => false,
                VaultFilter::All | VaultFilter::Health => true,
                VaultFilter::Favorites => i.is_favorite(),
                VaultFilter::Kind(k) => i.kind() == k,
                VaultFilter::Codes => i.otp_setup().is_some(),
            })
            .count()
    }

    /// Strength of an item's password (cached).
    pub fn strength_of(&mut self, id: VaultItemId) -> Option<Strength> {
        let item = self.items.iter().find(|i| i.id() == id)?;
        let f = item.fields();
        if f.password.is_empty() {
            return None;
        }
        let fingerprint = fingerprint(&f.password);
        if let Some((fp, s)) = self.strength.get(&id)
            && *fp == fingerprint
        {
            return Some(*s);
        }
        let context: Vec<&str> = [f.title.as_str(), f.username.as_str()]
            .into_iter()
            .chain(f.urls.iter().map(String::as_str))
            .collect();
        let s = health::estimate(&f.password, &context).strength;
        self.strength.insert(id, (fingerprint, s));
        Some(s)
    }

    /// Password health across live logins.
    pub fn health(&mut self, now: Timestamp) -> Report {
        let ids: Vec<VaultItemId> = self.items.iter().map(VaultItem::id).collect();
        let mut strengths: HashMap<VaultItemId, Strength> = HashMap::new();
        for id in ids {
            if let Some(s) = self.strength_of(id) {
                strengths.insert(id, s);
            }
        }
        let refs: Vec<&VaultItem> = self.items.iter().collect();
        health::report(&refs, now, &mut |i| {
            strengths
                .get(&i.id())
                .copied()
                .unwrap_or(Strength::VeryStrong)
        })
    }
}

/// An archived vault on disk (`vault-archives/*.otvault`; Calmfold wrote
/// `*.cfvault` and Keepstone `*.ksvault`, same format): the header and
/// sealed items exactly as they were, as hex.
///
/// * **Version 1** (a master-password vault): opens with the master password
///   the vault had then. Byte-for-byte the format older versions write.
/// * **Version 2** (a vault linked to an Ottero Cloud account, ADR 0014):
///   no master password; it holds the account wrapping and the recovery
///   wrapping, and opens with that account or the vault's recovery key.
///   Older versions refuse it ("isn't an Ottero vault archive") without
///   touching it.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ArchiveFile {
    format: String,
    version: u32,
    archived_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    salt: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    kdf_memory_kib: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    kdf_iterations: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    kdf_parallelism: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    wrapped_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_wrapped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_salt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_wrapped: Option<String>,
    items: Vec<ArchivedItem>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ArchivedItem {
    id: String,
    blob: String,
    updated_at: i64,
}

// The format name predates both renames; changing it would make every
// existing backup and archive unreadable.
const ARCHIVE_FORMAT: &str = "keepstone-vault-archive";

/// File extensions of backups and archives, newest first: `otvault` (Ottero),
/// `cfvault` (Calmfold), `ksvault` (Keepstone). All hold the same format.
pub const ARCHIVE_EXTENSIONS: [&str; 3] = ["otvault", "cfvault", "ksvault"];

/// What opens an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveOpener {
    MasterPassword,
    /// The Ottero Cloud account with this email, or the recovery key.
    Account(String),
}

/// Writes the vault's header and sealed items to `path` atomically (temp
/// file, fsync, rename) and proves the file reads back. Nothing is
/// decrypted. Returns the number of sealed items written.
fn write_archive(store: &Store, path: &std::path::Path, now: Timestamp) -> Result<usize> {
    let meta = store.vault_meta()?.context("No vault is set up")?;
    let items = store.vault_items()?;
    let file = ArchiveFile::new(&meta, &items, now)?;
    let json = serde_json::to_vec_pretty(&file).context("Couldn't write the archive")?;
    crate::private_file::write_private(path, &json).context("Couldn't write the archive")?;
    let back = ArchiveFile::read(path)?;
    if back.items.len() != items.len() {
        bail!("The archive didn't save correctly; nothing was changed.");
    }
    Ok(items.len())
}

impl ArchiveFile {
    fn new(meta: &VaultMeta, items: &[SealedItem], now: Timestamp) -> Result<Self> {
        let mut file = Self {
            format: ARCHIVE_FORMAT.into(),
            version: 1,
            archived_at: now.as_millisecond(),
            salt: String::new(),
            kdf_memory_kib: 0,
            kdf_iterations: 0,
            kdf_parallelism: 0,
            wrapped_key: String::new(),
            account_email: None,
            account_wrapped: None,
            recovery_salt: None,
            recovery_wrapped: None,
            items: items
                .iter()
                .map(|i| ArchivedItem {
                    id: i.id.to_string(),
                    blob: hex(&i.blob),
                    updated_at: i.updated_at,
                })
                .collect(),
        };
        match (&meta.master, &meta.account) {
            (Some(m), _) => {
                // Version 1 exactly as before: the master password opens it.
                file.salt = hex(&m.salt);
                file.kdf_memory_kib = m.kdf_memory_kib;
                file.kdf_iterations = m.kdf_iterations;
                file.kdf_parallelism = m.kdf_parallelism;
                file.wrapped_key = hex(&m.wrapped_key);
            }
            (None, Some(a)) => {
                let (Some(rs), Some(rw)) = (&meta.recovery_salt, &meta.recovery_wrapped) else {
                    bail!("This vault has no recovery key, so it can't be archived.");
                };
                file.version = 2;
                file.account_email = Some(a.email.clone());
                file.account_wrapped = Some(hex(&a.wrapped_key));
                file.recovery_salt = Some(hex(rs));
                file.recovery_wrapped = Some(hex(rw));
            }
            (None, None) => bail!("This vault has no key to archive."),
        }
        Ok(file)
    }

    pub fn read(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read(path).context("Couldn't read the archive")?;
        let file: Self =
            serde_json::from_slice(&text).context("That isn't an Ottero vault archive")?;
        let complete = match file.version {
            1 => !file.wrapped_key.is_empty() && !file.salt.is_empty(),
            2 => {
                file.account_email.is_some()
                    && file.account_wrapped.is_some()
                    && file.recovery_salt.is_some()
                    && file.recovery_wrapped.is_some()
            }
            _ => false,
        };
        if file.format != ARCHIVE_FORMAT || !complete {
            bail!("That isn't an Ottero vault archive");
        }
        Ok(file)
    }

    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn archived_at(&self) -> Timestamp {
        Timestamp::from_millisecond(self.archived_at).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    #[must_use]
    pub fn opener(&self) -> ArchiveOpener {
        match &self.account_email {
            Some(email) if self.version == 2 => ArchiveOpener::Account(email.clone()),
            _ => ArchiveOpener::MasterPassword,
        }
    }

    /// The master password's KDF input (version 1).
    pub fn kdf(&self) -> Result<KdfInput> {
        kdf_input(&self.master()?)
    }

    /// The recovery key's KDF input (version 2).
    pub fn recovery_kdf(&self) -> Result<KdfInput> {
        let salt = unhex(self.recovery_salt.as_deref().context("No recovery key")?)?;
        recovery_input(Some(&salt))
    }

    fn master(&self) -> Result<MasterWrap> {
        if self.version != 1 {
            bail!("This archive opens with an Ottero account or its recovery key.");
        }
        Ok(MasterWrap {
            salt: unhex(&self.salt)?,
            kdf_memory_kib: self.kdf_memory_kib,
            kdf_iterations: self.kdf_iterations,
            kdf_parallelism: self.kdf_parallelism,
            wrapped_key: unhex(&self.wrapped_key)?,
        })
    }

    fn sealed(&self) -> Result<Vec<SealedItem>> {
        self.items
            .iter()
            .map(|i| {
                Ok(SealedItem {
                    id: i.id.parse().map_err(|_| DomainError::Corrupt)?,
                    blob: unhex(&i.blob)?,
                    updated_at: i.updated_at,
                })
            })
            .collect()
    }

    /// The archive's live items, decrypted with its vault key.
    fn items_with(&self, key: &SecretKey) -> Result<Vec<VaultItem>> {
        let mut out = Vec::new();
        for row in self.sealed()? {
            if let Ok(json) = open(key, &row.blob, row.id.to_string().as_bytes())
                && let Ok(item) = VaultItem::from_json(&json, row.id)
                && !item.is_trashed()
            {
                out.push(item);
            }
        }
        Ok(out)
    }
}

impl VaultSession {
    /// Decrypts an archived vault with its old master password's KEK and
    /// returns its live items (to import into the current vault).
    pub fn open_archive(archive: &ArchiveFile, kek: &SecretKey) -> Result<Option<Vec<VaultItem>>> {
        let wrapped = archive.master()?.wrapped_key;
        let Some(key) = open(kek, &wrapped, KEY_AAD).ok().and_then(|b| key_from(&b)) else {
            return Ok(None);
        };
        archive.items_with(&key).map(Some)
    }

    /// Opens a version-2 archive with the account key it was linked to.
    pub fn open_archive_with_account(
        archive: &ArchiveFile,
        account_key: &SecretKey,
    ) -> Result<Option<Vec<VaultItem>>> {
        let wrapped = unhex(
            archive
                .account_wrapped
                .as_deref()
                .context("No account key")?,
        )?;
        let Ok(key) = vault_link::unwrap_vault_key(account_key, &wrapped) else {
            return Ok(None);
        };
        archive.items_with(&key).map(Some)
    }

    /// Opens a version-2 archive with the recovery key's KEK.
    pub fn open_archive_with_recovery(
        archive: &ArchiveFile,
        rkek: &SecretKey,
    ) -> Result<Option<Vec<VaultItem>>> {
        let wrapped = unhex(
            archive
                .recovery_wrapped
                .as_deref()
                .context("No recovery key")?,
        )?;
        let Some(key) = open(rkek, &wrapped, RECOVERY_AAD)
            .ok()
            .and_then(|b| key_from(&b))
        else {
            return Ok(None);
        };
        archive.items_with(&key).map(Some)
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(DomainError::Corrupt.into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            s.get(i..i + 2)
                .and_then(|p| u8::from_str_radix(p, 16).ok())
                .ok_or_else(|| DomainError::Corrupt.into())
        })
        .collect()
}

fn kdf_input(master: &MasterWrap) -> Result<KdfInput> {
    let salt: [u8; Salt::LEN] = master
        .salt
        .as_slice()
        .try_into()
        .map_err(|_| DomainError::Corrupt)?;
    let params = KdfParams {
        memory_kib: master.kdf_memory_kib,
        iterations: master.kdf_iterations,
        parallelism: master.kdf_parallelism,
    };
    params.validate().map_err(|_| DomainError::Corrupt)?;
    Ok(KdfInput {
        salt: Salt(salt),
        params,
    })
}

fn recovery_input(salt: Option<&[u8]>) -> Result<KdfInput> {
    let salt: [u8; Salt::LEN] = salt
        .context("This vault has no recovery key")?
        .try_into()
        .map_err(|_| DomainError::Corrupt)?;
    Ok(KdfInput {
        salt: Salt(salt),
        params: RECOVERY_KDF,
    })
}

/// A vault key from opened bytes (anything but 32 bytes is no key).
fn key_from(bytes: &[u8]) -> Option<SecretKey> {
    if bytes.len() != SecretKey::LEN {
        return None;
    }
    let mut raw = Zeroizing::new([0u8; SecretKey::LEN]);
    raw.copy_from_slice(bytes);
    Some(SecretKey::from_bytes(&mut raw))
}

fn master_wrap(input: KdfInput, kek: &SecretKey, vault_key: &SecretKey) -> Result<MasterWrap> {
    Ok(MasterWrap {
        salt: input.salt.0.to_vec(),
        kdf_memory_kib: input.params.memory_kib,
        kdf_iterations: input.params.iterations,
        kdf_parallelism: input.params.parallelism,
        wrapped_key: seal(kek, vault_key.expose(), KEY_AAD)
            .context("Couldn't encrypt the vault key")?,
    })
}

type RecoveryColumns = (Option<Vec<u8>>, Option<Vec<u8>>);

fn recovery_wrap(
    recovery: Option<(KdfInput, &SecretKey)>,
    vault_key: &SecretKey,
) -> Result<RecoveryColumns> {
    Ok(match recovery {
        Some((input, rkek)) => (
            Some(input.salt.0.to_vec()),
            Some(
                seal(rkek, vault_key.expose(), RECOVERY_AAD)
                    .context("Couldn't encrypt the vault key")?,
            ),
        ),
        None => (None, None),
    })
}

/// Wraps the vault key for an account and opens the wrapping again before
/// anything is stored, so a broken wrapping can never replace a working one.
fn account_wrap(
    email: &str,
    account_key: &SecretKey,
    vault_key: &SecretKey,
) -> Result<AccountWrap> {
    let wrapped = vault_link::wrap_vault_key(account_key, vault_key)
        .context("Couldn't encrypt the vault key")?;
    let back = vault_link::unwrap_vault_key(account_key, &wrapped)
        .context("Couldn't encrypt the vault key")?;
    if back.expose() != vault_key.expose() {
        bail!("Couldn't encrypt the vault key");
    }
    Ok(AccountWrap {
        email: email.to_owned(),
        wrapped_key: wrapped,
    })
}

fn sort(items: &mut [VaultItem]) {
    items.sort_by_cached_key(|i| i.title().to_lowercase());
}

/// A non-reversible tag for "has this password changed" (cache key only;
/// never stored or shown).
fn fingerprint(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ottero_crypto::SecretString;

    fn now() -> Timestamp {
        Timestamp::from_second(1_800_000_000).expect("ts")
    }

    /// The minimum KDF cost keeps tests fast; the app uses RECOMMENDED.
    fn input() -> KdfInput {
        KdfInput::fresh(KdfParams::MINIMUM).expect("salt")
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn unlocked() -> (Store, VaultSession) {
        let mut store = Store::open_in_memory().expect("store");
        let mut v = VaultSession::load(&store).expect("load");
        assert_eq!(v.state(), LockState::Uninitialized);
        let i = input();
        let kek = derive(&pw("correct horse"), &i.salt, i.params).expect("kek");
        v.finish_setup(&mut store, i, &kek, None, now())
            .expect("setup");
        (store, v)
    }

    fn login(title: &str, password: &str) -> ItemFields {
        ItemFields {
            title: title.into(),
            password: password.into(),
            ..ItemFields::new(ItemKind::Login)
        }
    }

    fn unlock(store: &Store, v: &mut VaultSession, password: &str) -> bool {
        let i = v.start_unlock(store).expect("start");
        v.finish_unlock(store, derive(&pw(password), &i.salt, i.params))
            .expect("finish")
    }

    #[test]
    fn setup_lock_unlock_round_trip() {
        let (mut store, mut v) = unlocked();
        let id = v
            .create(&mut store, &login("GitHub", "s3cret!"), now())
            .expect("create");
        v.lock();
        assert_eq!(v.state(), LockState::Locked);
        assert!(v.items().is_empty(), "nothing decrypted while locked");
        assert!(v.create(&mut store, &login("x", ""), now()).is_err());

        assert!(!unlock(&store, &mut v, "wrong"), "wrong password");
        assert_eq!(v.state(), LockState::Locked);
        assert!(unlock(&store, &mut v, "correct horse"));
        assert_eq!(
            v.item(id).map(|i| i.fields().password.clone()).as_deref(),
            Some("s3cret!")
        );

        // A reopened app starts locked.
        let fresh = VaultSession::load(&store).expect("load");
        assert_eq!(fresh.state(), LockState::Locked);
    }

    #[test]
    fn storage_only_ever_sees_ciphertext() {
        let (mut store, mut v) = unlocked();
        v.create(&mut store, &login("GitHub", "hunter2-unique"), now())
            .expect("create");
        for row in store.vault_items().expect("rows") {
            let text = String::from_utf8_lossy(&row.blob);
            assert!(!text.contains("hunter2") && !text.contains("GitHub"));
        }
    }

    #[test]
    fn environment_secrets_survive_encryption_reload_and_backup() {
        use ottero_core::vault::environment::EnvironmentSecrets;
        let (mut store, mut v) = unlocked();
        let source = (0..200)
            .map(|i| format!("K{i}=secret-value-{i}\n"))
            .collect::<String>();
        let fields = ItemFields {
            title: "Project development".into(),
            environment: Some(EnvironmentSecrets::parse(&source).expect("parse")),
            ..ItemFields::new(ItemKind::Note)
        };
        let id = v.create(&mut store, &fields, now()).expect("create");
        for row in store.vault_items().expect("rows") {
            let text = String::from_utf8_lossy(&row.blob);
            assert!(!text.contains("secret-value") && !text.contains("Project development"));
        }
        v.lock();
        assert!(unlock(&store, &mut v, "correct horse"));
        assert_eq!(
            v.item(id).expect("item").fields().environment,
            fields.environment
        );
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("backup.otvault");
        write_archive(&store, &path, now()).expect("backup");
        let archive = ArchiveFile::read(&path).expect("read");
        let k = archive.kdf().expect("kdf");
        let key = derive(&pw("correct horse"), &k.salt, k.params).expect("derive");
        let items = VaultSession::open_archive(&archive, &key)
            .expect("open")
            .expect("items");
        assert_eq!(items[0].fields().environment, fields.environment);
    }

    #[test]
    fn a_blob_moved_to_another_id_is_unreadable_not_fatal() {
        let (mut store, mut v) = unlocked();
        v.create(&mut store, &login("A", "a"), now()).expect("a");
        let rows = store.vault_items().expect("rows");
        // Copy A's ciphertext under a new id: the AAD no longer matches.
        let moved = SealedItem {
            id: VaultItemId::new(),
            ..rows[0].clone()
        };
        store.save_vault_items(&[moved]).expect("save");
        v.lock();
        assert!(unlock(&store, &mut v, "correct horse"));
        assert_eq!(v.items().len(), 1);
        assert_eq!(v.unreadable(), 1);
    }

    #[test]
    fn stale_unlock_results_are_ignored() {
        let (store, mut v) = unlocked();
        v.lock();
        let i = v.start_unlock(&store).expect("start");
        v.lock(); // user locked again while deriving
        let late = v
            .finish_unlock(&store, derive(&pw("correct horse"), &i.salt, i.params))
            .expect("finish");
        assert!(!late);
        assert_eq!(v.state(), LockState::Locked);
    }

    #[test]
    fn changing_the_master_password_keeps_items() {
        let (mut store, mut v) = unlocked();
        let id = v.create(&mut store, &login("A", "pw"), now()).expect("a");
        let current = VaultSession::current_kdf(&store).expect("kdf");
        let wrong = derive(&pw("nope"), &current.salt, current.params).expect("kek");
        let new_input = input();
        let new_kek = derive(&pw("new master"), &new_input.salt, new_input.params).expect("kek");
        assert!(
            !v.change_password(&mut store, &wrong, new_input, &new_kek, now())
                .expect("try")
        );
        let right = derive(&pw("correct horse"), &current.salt, current.params).expect("kek");
        assert!(
            v.change_password(&mut store, &right, new_input, &new_kek, now())
                .expect("change")
        );
        v.lock();
        assert!(
            !unlock(&store, &mut v, "correct horse"),
            "old password no longer works"
        );
        assert!(unlock(&store, &mut v, "new master"));
        assert!(v.item(id).is_some());
    }

    #[test]
    fn filters_search_trash_and_health() {
        let (mut store, mut v) = unlocked();
        let gh = v
            .create(&mut store, &login("GitHub", "same-Pass-word-81!"), now())
            .expect("gh");
        let gl = v
            .create(&mut store, &login("GitLab", "same-Pass-word-81!"), now())
            .expect("gl");
        let weak = v
            .create(&mut store, &login("Router", "123456"), now())
            .expect("weak");
        v.update(&mut store, gh, |i| {
            i.set_favorite(true, now()).map(|()| true)
        })
        .expect("fav");
        v.update(&mut store, weak, |i| i.trash(now()).map(|()| true))
            .expect("trash");

        assert_eq!(
            v.list(VaultFilter::All, None, "", VaultSort::Name, now()),
            [gh, gl]
        );
        assert_eq!(
            v.list(VaultFilter::Favorites, None, "", VaultSort::Name, now()),
            [gh]
        );
        assert_eq!(
            v.list(VaultFilter::Trash, None, "", VaultSort::Name, now()),
            [weak]
        );
        assert_eq!(
            v.list(VaultFilter::All, None, "lab", VaultSort::Name, now()),
            [gl]
        );
        assert_eq!(v.count(VaultFilter::All), 2);
        let report = v.health(now());
        assert_eq!(report.reused.len(), 1, "GitHub and GitLab share one");
        assert!(report.weak.is_empty(), "trashed items aren't checked");
        v.delete(&mut store, &[weak]).expect("delete");
        assert_eq!(store.vault_items().expect("rows").len(), 2);
    }

    #[test]
    fn a_recovery_key_reopens_the_vault_with_a_new_password() {
        let (mut store, mut v) = unlocked();
        let id = v.create(&mut store, &login("A", "pw"), now()).expect("a");
        let rinput = KdfInput::fresh(RECOVERY_KDF).expect("salt");
        let rkey = pw("ABCDABCDABCDABCDABCDABCDABCDABCD");
        let rkek = derive(&rkey, &rinput.salt, rinput.params).expect("kek");
        v.set_recovery(&mut store, rinput, &rkek, now())
            .expect("recovery");
        assert!(VaultSession::has_recovery(&store));
        v.lock();

        let new_input = input();
        let new_kek =
            derive(&pw("brand new master"), &new_input.salt, new_input.params).expect("kek");
        let ri = VaultSession::recovery_kdf(&store).expect("kdf");
        let wrong = derive(&pw("ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ"), &ri.salt, ri.params);
        assert!(
            !v.recover(
                &mut store,
                wrong,
                NewOpener::MasterPassword(new_input, &new_kek),
                now()
            )
            .expect("try")
        );
        assert_eq!(v.state(), LockState::Locked);
        let right = derive(&rkey, &ri.salt, ri.params);
        assert!(
            v.recover(
                &mut store,
                right,
                NewOpener::MasterPassword(new_input, &new_kek),
                now()
            )
            .expect("recover")
        );
        assert!(v.item(id).is_some(), "items back");
        v.lock();
        assert!(
            !unlock(&store, &mut v, "correct horse"),
            "forgotten password retired"
        );
        assert!(unlock(&store, &mut v, "brand new master"));
    }

    #[test]
    fn starting_over_archives_first_and_the_archive_opens_later() {
        let (mut store, mut v) = unlocked();
        v.create(&mut store, &login("Old bank", "pw-1"), now())
            .expect("a");
        v.lock();
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("vault-archives/old.cfvault");
        v.archive_and_reset(&mut store, &path, now())
            .expect("reset");
        assert_eq!(v.state(), LockState::Uninitialized);
        assert!(store.vault_meta().expect("meta").is_none());
        let text = std::fs::read_to_string(&path).expect("archive");
        assert!(
            !text.contains("Old bank") && !text.contains("pw-1"),
            "still encrypted"
        );

        let archive = ArchiveFile::read(&path).expect("read");
        let k = archive.kdf().expect("kdf");
        let wrong = derive(&pw("nope"), &k.salt, k.params).expect("kek");
        assert!(
            VaultSession::open_archive(&archive, &wrong)
                .expect("open")
                .is_none()
        );
        let right = derive(&pw("correct horse"), &k.salt, k.params).expect("kek");
        let items = VaultSession::open_archive(&archive, &right)
            .expect("open")
            .expect("items");
        assert_eq!(items[0].title(), "Old bank");
        assert!(unhex("abc").is_err());
    }

    #[test]
    fn sort_tags_recent_and_codes() {
        let (mut store, mut v) = unlocked();
        let mut a = login("Alpha", "x");
        a.tags = vec!["work".into()];
        let a = v.create(&mut store, &a, now()).expect("a");
        let mut b = login("Beta", "y");
        b.totp = "JBSWY3DPEHPK3PXP".into();
        let b = v.create(&mut store, &b, now()).expect("b");
        v.update(&mut store, b, |i| {
            i.mark_used(Timestamp::from_second(1_900_000_000).expect("ts"));
            Ok(true)
        })
        .expect("used");
        assert_eq!(
            v.list(VaultFilter::All, Some("work"), "", VaultSort::Name, now()),
            [a]
        );
        assert_eq!(
            v.list(VaultFilter::Codes, None, "", VaultSort::Name, now()),
            [b]
        );
        assert_eq!(
            v.list(VaultFilter::All, None, "", VaultSort::RecentlyUsed, now())[0],
            b
        );
        assert_eq!(v.recent(5), [b]);
        assert_eq!(v.tags(), [("work".to_owned(), 1)]);
        assert_eq!(
            v.list(VaultFilter::All, None, "has:code", VaultSort::Name, now()),
            [b]
        );
    }

    #[test]
    fn large_vaults_load_and_search_quickly() {
        let (mut store, mut v) = unlocked();
        let items: Vec<VaultItem> = (0..2_000)
            .map(|n| {
                VaultItem::new(&login(&format!("Site {n:04}"), &format!("pw-{n}")), now())
                    .expect("item")
            })
            .collect();
        v.put(&mut store, items).expect("bulk");
        v.lock();
        let started = std::time::Instant::now();
        assert!(unlock(&store, &mut v, "correct horse"));
        assert_eq!(v.items().len(), 2_000);
        assert_eq!(
            v.list(VaultFilter::All, None, "site 1999", VaultSort::Name, now())
                .len(),
            1
        );
        assert!(
            started.elapsed().as_secs() < 5,
            "unlock + decrypt 2,000 items"
        );
    }

    // ---- Linked to an Ottero Cloud account (ADR 0014) ---------------------

    const EMAIL: &str = "sam@example.com";

    fn account_key(b: u8) -> SecretKey {
        let mut raw = [b; SecretKey::LEN];
        SecretKey::from_bytes(&mut raw)
    }

    fn recovery_key() -> (KdfInput, SecretKey, SecretString) {
        let rinput = KdfInput::fresh(RECOVERY_KDF).expect("salt");
        let rkey = pw("ABCDABCDABCDABCDABCDABCDABCDABCD");
        let rkek = derive(&rkey, &rinput.salt, rinput.params).expect("kek");
        (rinput, rkek, rkey)
    }

    /// Links a locked master-password vault; returns the result.
    fn link(
        store: &mut Store,
        v: &mut VaultSession,
        password: &str,
        key: &SecretKey,
        recovery: Option<(KdfInput, &SecretKey)>,
    ) -> LinkResult {
        let (i, epoch) = v.start_link(store).expect("start");
        let kek = derive(&pw(password), &i.salt, i.params);
        v.finish_link(
            store,
            epoch,
            kek,
            LinkAccount { email: EMAIL, key },
            recovery,
            now(),
        )
        .expect("finish")
    }

    #[test]
    fn linking_rewraps_the_key_without_touching_items_and_retires_the_master_password() {
        let (mut store, mut v) = unlocked();
        let id = v
            .create(&mut store, &login("GitHub", "s3cret!"), now())
            .expect("a");
        let blobs = store.vault_items().expect("rows");
        v.lock();
        let ak = account_key(7);
        let (ri, rkek, _) = recovery_key();

        assert_eq!(
            link(&mut store, &mut v, "wrong", &ak, Some((ri, &rkek))),
            LinkResult::WrongPassword
        );
        assert_eq!(v.state(), LockState::Locked);
        assert_eq!(v.opener(), Some(Opener::MasterPassword), "nothing changed");
        assert!(
            store
                .vault_meta()
                .expect("meta")
                .expect("m")
                .master
                .is_some()
        );

        assert_eq!(
            link(&mut store, &mut v, "correct horse", &ak, Some((ri, &rkek))),
            LinkResult::Linked
        );
        assert_eq!(
            v.state(),
            LockState::Unlocked,
            "linking from locked also unlocks"
        );
        assert_eq!(v.opener(), Some(Opener::Account));
        assert_eq!(v.linked_email(), Some(EMAIL));
        assert!(v.item(id).is_some());
        assert_eq!(
            store.vault_items().expect("rows"),
            blobs,
            "no item re-encrypted"
        );
        let meta = store.vault_meta().expect("meta").expect("m");
        assert!(meta.master.is_none(), "master password retired");
        assert!(meta.recovery_wrapped.is_some(), "a recovery key was added");
        assert_eq!(meta.account.as_ref().map(|a| a.email.as_str()), Some(EMAIL));

        // Restart: the master password no longer opens it; the account does.
        let mut fresh = VaultSession::load(&store).expect("load");
        assert_eq!(fresh.opener(), Some(Opener::Account));
        assert!(fresh.start_unlock(&store).is_err(), "no master password");
        assert_eq!(
            fresh
                .unlock_with_account(&store, EMAIL, &account_key(8))
                .expect("other key"),
            AccountUnlock::WrongKey
        );
        assert_eq!(
            fresh
                .unlock_with_account(&store, "else@example.com", &ak)
                .expect("other"),
            AccountUnlock::OtherAccount(EMAIL.into())
        );
        assert_eq!(fresh.state(), LockState::Locked);
        assert_eq!(
            fresh.unlock_with_account(&store, EMAIL, &ak).expect("open"),
            AccountUnlock::Opened
        );
        assert_eq!(
            fresh
                .item(id)
                .map(|i| i.fields().password.clone())
                .as_deref(),
            Some("s3cret!")
        );
    }

    #[test]
    fn linking_while_unlocked_needs_the_master_password_too() {
        let (mut store, mut v) = unlocked();
        let (ri, rkek, _) = recovery_key();
        v.set_recovery(&mut store, ri, &rkek, now())
            .expect("recovery");
        let ak = account_key(7);
        assert_eq!(
            link(&mut store, &mut v, "nope", &ak, None),
            LinkResult::WrongPassword
        );
        assert_eq!(v.state(), LockState::Unlocked, "still unlocked");
        assert_eq!(v.opener(), Some(Opener::MasterPassword));
        assert_eq!(
            link(&mut store, &mut v, "correct horse", &ak, None),
            LinkResult::Linked
        );
        assert_eq!(v.opener(), Some(Opener::Account));
        assert!(v.start_link(&store).is_err(), "can't link twice");
    }

    #[test]
    fn a_vault_without_a_recovery_key_needs_one_to_link() {
        let (mut store, mut v) = unlocked();
        v.lock();
        let ak = account_key(7);
        let (i, epoch) = v.start_link(&store).expect("start");
        let kek = derive(&pw("correct horse"), &i.salt, i.params);
        assert!(
            v.finish_link(
                &mut store,
                epoch,
                kek,
                LinkAccount {
                    email: EMAIL,
                    key: &ak
                },
                None,
                now()
            )
            .is_err()
        );
        assert_eq!(v.state(), LockState::Locked);
        assert!(
            unlock(&store, &mut v, "correct horse"),
            "master password still works"
        );
    }

    #[test]
    fn a_link_finishing_after_a_lock_is_dropped() {
        let (mut store, mut v) = unlocked();
        let (ri, rkek, _) = recovery_key();
        v.set_recovery(&mut store, ri, &rkek, now())
            .expect("recovery");
        v.lock();
        let (i, epoch) = v.start_link(&store).expect("start");
        v.lock(); // auto-lock while Argon2 runs
        let kek = derive(&pw("correct horse"), &i.salt, i.params);
        assert_eq!(
            v.finish_link(
                &mut store,
                epoch,
                kek,
                LinkAccount {
                    email: EMAIL,
                    key: &account_key(7)
                },
                None,
                now()
            )
            .expect("finish"),
            LinkResult::Stale
        );
        assert_eq!(v.opener(), Some(Opener::MasterPassword));
        assert!(store.vault_meta().expect("m").expect("m").account.is_none());
        // A cancelled link (account changed) also leaves it as it was.
        let (_, epoch) = v.start_link(&store).expect("start");
        assert_eq!(v.state(), LockState::Unlocking);
        v.cancel_link(epoch);
        assert_eq!(v.state(), LockState::Locked);
        assert!(unlock(&store, &mut v, "correct horse"));
    }

    /// The master password changed while the link's Argon2 ran: the old
    /// password's key no longer opens the header, so nothing is written.
    /// (A crash inside the commit itself is covered in ottero-storage:
    /// `an_interrupted_link_leaves_the_old_header_on_disk`.)
    #[test]
    fn a_link_racing_a_password_change_writes_nothing() {
        let (mut store, mut v) = unlocked();
        let (ri, rkek, _) = recovery_key();
        v.set_recovery(&mut store, ri, &rkek, now())
            .expect("recovery");
        let (i, epoch) = v.start_link(&store).expect("start");
        let kek = derive(&pw("correct horse"), &i.salt, i.params);
        let current = VaultSession::current_kdf(&store).expect("kdf");
        let old = derive(&pw("correct horse"), &current.salt, current.params).expect("k");
        let ni = input();
        let new = derive(&pw("other master"), &ni.salt, ni.params).expect("k");
        v.change_password(&mut store, &old, ni, &new, now())
            .expect("change");
        let result = v
            .finish_link(
                &mut store,
                epoch,
                kek,
                LinkAccount {
                    email: EMAIL,
                    key: &account_key(7),
                },
                None,
                now(),
            )
            .expect("finish");
        assert_eq!(result, LinkResult::WrongPassword);
        assert_eq!(v.opener(), Some(Opener::MasterPassword));
        assert!(store.vault_meta().expect("m").expect("m").account.is_none());
        v.lock();
        assert!(unlock(&store, &mut v, "other master"));
    }

    #[test]
    fn setup_under_the_account_has_no_master_password_and_a_recovery_key() {
        let mut store = Store::open_in_memory().expect("store");
        let mut v = VaultSession::load(&store).expect("load");
        let ak = account_key(3);
        let (ri, rkek, rkey) = recovery_key();
        v.finish_setup_with_account(&mut store, EMAIL, &ak, (ri, &rkek), now())
            .expect("setup");
        assert_eq!(v.opener(), Some(Opener::Account));
        let id = v
            .create(&mut store, &login("Bank", "pw-1"), now())
            .expect("a");
        v.lock();
        assert!(VaultSession::current_kdf(&store).is_err());
        assert_eq!(
            v.unlock_with_account(&store, EMAIL, &ak).expect("open"),
            AccountUnlock::Opened
        );
        v.lock();

        // The recovery key opens it with no account: a new master password.
        let ri = VaultSession::recovery_kdf(&store).expect("kdf");
        let rk = derive(&rkey, &ri.salt, ri.params);
        let ni = input();
        let nk = derive(&pw("fresh master"), &ni.salt, ni.params).expect("k");
        assert!(
            v.recover(&mut store, rk, NewOpener::MasterPassword(ni, &nk), now())
                .expect("recover")
        );
        assert_eq!(v.opener(), Some(Opener::MasterPassword), "unlinked");
        assert!(v.item(id).is_some());
        assert!(store.vault_meta().expect("m").expect("m").account.is_none());
        v.lock();
        assert!(unlock(&store, &mut v, "fresh master"));
    }

    #[test]
    fn recovery_can_move_a_linked_vault_to_the_open_account() {
        let mut store = Store::open_in_memory().expect("store");
        let mut v = VaultSession::load(&store).expect("load");
        let (ri, rkek, rkey) = recovery_key();
        v.finish_setup_with_account(&mut store, EMAIL, &account_key(3), (ri, &rkek), now())
            .expect("setup");
        v.lock();
        let other = account_key(9);
        let ri = VaultSession::recovery_kdf(&store).expect("kdf");
        let wrong = derive(&pw("ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ"), &ri.salt, ri.params);
        let new = NewOpener::Account {
            email: "new@example.com",
            account_key: &other,
        };
        assert!(!v.recover(&mut store, wrong, new, now()).expect("try"));
        assert_eq!(v.linked_email(), Some(EMAIL), "wrong key: nothing changed");
        let right = derive(&rkey, &ri.salt, ri.params);
        let new = NewOpener::Account {
            email: "new@example.com",
            account_key: &other,
        };
        assert!(v.recover(&mut store, right, new, now()).expect("recover"));
        assert_eq!(v.linked_email(), Some("new@example.com"));
        v.lock();
        assert_eq!(
            v.unlock_with_account(&store, "new@example.com", &other)
                .expect("open"),
            AccountUnlock::Opened
        );
    }

    #[test]
    fn a_tampered_account_wrapping_opens_nothing_and_changes_nothing() {
        let mut store = Store::open_in_memory().expect("store");
        let mut v = VaultSession::load(&store).expect("load");
        let ak = account_key(3);
        let (ri, rkek, _) = recovery_key();
        v.finish_setup_with_account(&mut store, EMAIL, &ak, (ri, &rkek), now())
            .expect("setup");
        v.lock();
        let mut meta = store.vault_meta().expect("m").expect("m");
        let before = meta.clone();
        if let Some(a) = meta.account.as_mut() {
            let last = a.wrapped_key.len() - 1;
            a.wrapped_key[last] ^= 1;
        }
        store.save_vault_meta(&meta).expect("tamper");
        assert_eq!(
            v.unlock_with_account(&store, EMAIL, &ak).expect("try"),
            AccountUnlock::WrongKey
        );
        assert_eq!(v.state(), LockState::Locked);
        let after = store.vault_meta().expect("m").expect("m");
        assert_eq!(after.recovery_wrapped, before.recovery_wrapped);
    }

    #[test]
    fn a_linked_backup_opens_with_its_account_or_recovery_key_only() {
        let mut store = Store::open_in_memory().expect("store");
        let mut v = VaultSession::load(&store).expect("load");
        let ak = account_key(3);
        let (ri, rkek, rkey) = recovery_key();
        v.finish_setup_with_account(&mut store, EMAIL, &ak, (ri, &rkek), now())
            .expect("setup");
        v.create(&mut store, &login("Bank", "pw-1"), now())
            .expect("a");
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("linked.otvault");
        v.export(&store, &path, now()).expect("export");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("\"version\": 2") && !text.contains("pw-1"));
        let archive = ArchiveFile::read(&path).expect("archive");
        assert_eq!(archive.opener(), ArchiveOpener::Account(EMAIL.into()));
        assert!(archive.kdf().is_err(), "no master password in it");
        assert_eq!(
            VaultSession::open_archive_with_account(&archive, &ak)
                .expect("open")
                .expect("items")
                .len(),
            1
        );
        assert!(
            VaultSession::open_archive_with_account(&archive, &account_key(4))
                .expect("open")
                .is_none()
        );
        let k = archive.recovery_kdf().expect("kdf");
        let rk = derive(&rkey, &k.salt, k.params).expect("k");
        assert_eq!(
            VaultSession::open_archive_with_recovery(&archive, &rk)
                .expect("open")
                .expect("items")[0]
                .title(),
            "Bank"
        );
    }

    #[test]
    fn master_password_backups_are_still_version_1() {
        let (mut store, v) = unlocked();
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("plain.otvault");
        v.export(&store, &path, now()).expect("export");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("\"version\": 1"));
        for absent in [
            "account_email",
            "account_wrapped",
            "recovery_salt",
            "recovery_wrapped",
        ] {
            assert!(!text.contains(absent), "{absent}");
        }
        let _ = &mut store;
    }
}
