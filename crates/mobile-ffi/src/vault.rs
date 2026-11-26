//! Passwords Vault bridge over `ottero-crypto` and `ottero-storage`.
//!
//! Uses the exact same KDF, envelope, and AAD format as the desktop app
//! (`keepstone-vault-key-v1` + per-item UUID AAD) so synced vaults open
//! identically across macOS, Windows, Linux, and Android.
//!
//! Argon2id key derivation runs WITHOUT holding any mutex, guarded by a
//! monotonic `epoch` counter so a concurrent `lock()` always wins over an
//! in-flight `unlock()`.

use std::sync::Mutex;

use ottero_core::vault::{ItemFields, ItemKind, VaultItem, generator, totp};
use ottero_core::{VaultItemId, clock};
use ottero_crypto::{
    KdfParams, Salt, SecretKey, SecretString, Zeroize, Zeroizing, derive_key, open, random_below,
    seal,
};
use ottero_storage::{MasterWrap, SealedItem, Store, VaultMeta};
use ottero_sync_client::enqueue_vault_item_change;

use crate::dto::{MobileVaultStatus, TotpCodeSnapshot, VaultEntryDetail, VaultEntrySummary};
use crate::error::MobileError;
use crate::sync;

fn enqueue_vault_item(store: &Store, sealed: &SealedItem) {
    if let Some(device_id) = sync::active_device_id(store) {
        let _ = enqueue_vault_item_change(store, &device_id, sealed, clock::now());
    }
}

const KEY_AAD: &[u8] = b"keepstone-vault-key-v1";
const META_VERSION: u32 = 1;

pub(crate) struct VaultKeyState {
    pub(crate) key: Option<SecretKey>,
    pub(crate) epoch: u64,
}

impl VaultKeyState {
    pub(crate) fn new() -> Self {
        Self {
            key: None,
            epoch: 0,
        }
    }
}

fn parse_vault_item_id(id: &str) -> Result<VaultItemId, MobileError> {
    id.parse::<VaultItemId>()
        .map_err(|_| MobileError::InvalidInput {
            reason: "Invalid vault item ID".into(),
        })
}

fn kind_code(kind: ItemKind) -> String {
    match kind {
        ItemKind::Login => "login".into(),
        ItemKind::Card => "card".into(),
        ItemKind::Note => "note".into(),
    }
}

fn seal_item(key: &SecretKey, item: &VaultItem) -> Result<SealedItem, MobileError> {
    let json = item.to_json()?;
    let blob =
        seal(key, &json, item.id().to_string().as_bytes()).map_err(|_| MobileError::Storage {
            reason: "Could not encrypt vault item".into(),
        })?;
    Ok(SealedItem {
        id: item.id(),
        blob,
        updated_at: item.updated_at().as_millisecond(),
    })
}

fn open_item(key: &SecretKey, sealed: &SealedItem) -> Result<VaultItem, MobileError> {
    let json = open(key, &sealed.blob, sealed.id.to_string().as_bytes()).map_err(|_| {
        MobileError::Storage {
            reason: "Could not decrypt vault item".into(),
        }
    })?;
    Ok(VaultItem::from_json(&json, sealed.id)?)
}

fn to_detail(item: &VaultItem) -> VaultEntryDetail {
    let f = item.fields();
    VaultEntryDetail {
        id: item.id().to_string(),
        kind: kind_code(item.kind()),
        title: item.title().to_owned(),
        username: f.username.clone(),
        password: f.password.clone(),
        url: f.urls.first().cloned().unwrap_or_default(),
        totp_secret: f.totp.clone(),
        cardholder: f.cardholder.clone(),
        card_number: f.number.clone(),
        card_expiry: f.expiry.clone(),
        card_cvv: f.cvv.clone(),
        notes: f.notes.clone(),
        favorite: item.is_favorite(),
    }
}

pub(crate) fn status(
    store: &Store,
    state: &VaultKeyState,
) -> Result<MobileVaultStatus, MobileError> {
    if state.key.is_some() {
        return Ok(MobileVaultStatus::Unlocked);
    }
    if store.vault_meta()?.is_some() {
        Ok(MobileVaultStatus::Locked)
    } else {
        Ok(MobileVaultStatus::NotSetUp)
    }
}

/// Runs Argon2id outside both locks, then saves `VaultMeta` and installs the key.
pub(crate) fn setup(
    store_mutex: &Mutex<Store>,
    key_mutex: &Mutex<VaultKeyState>,
    master_password: String,
) -> Result<(), MobileError> {
    if master_password.chars().count() < 8 {
        return Err(MobileError::InvalidInput {
            reason: "Master password must be at least 8 characters".into(),
        });
    }
    {
        let store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
        if store.vault_meta()?.is_some() {
            return Err(MobileError::InvalidInput {
                reason: "Vault is already set up".into(),
            });
        }
    }

    let secret = SecretString::from(master_password);
    let salt = Salt::generate().map_err(|_| MobileError::Storage {
        reason: "Could not generate salt".into(),
    })?;
    let params = KdfParams::RECOMMENDED;
    // Slow Argon2id derivation runs with NO locks held.
    let kek = derive_key(&secret, &salt, params).map_err(|_| MobileError::Storage {
        reason: "Could not derive vault key".into(),
    })?;
    let vault_key = SecretKey::generate().map_err(|_| MobileError::Storage {
        reason: "Could not generate vault key".into(),
    })?;
    let wrapped_key =
        seal(&kek, vault_key.expose(), KEY_AAD).map_err(|_| MobileError::Storage {
            reason: "Could not seal vault key".into(),
        })?;

    let now_ms = clock::now().as_millisecond();
    let meta = VaultMeta {
        version: META_VERSION,
        master: Some(MasterWrap {
            salt: salt.0.to_vec(),
            kdf_memory_kib: params.memory_kib,
            kdf_iterations: params.iterations,
            kdf_parallelism: params.parallelism,
            wrapped_key,
        }),
        created_at: now_ms,
        updated_at: now_ms,
        recovery_salt: None,
        recovery_wrapped: None,
        account: None,
    };

    let mut store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    if store.vault_meta()?.is_some() {
        return Err(MobileError::InvalidInput {
            reason: "Vault is already set up".into(),
        });
    }
    store.save_vault_meta(&meta)?;
    drop(store);

    let mut state = key_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    state.key = Some(vault_key);
    state.epoch = state.epoch.wrapping_add(1);
    Ok(())
}

/// Runs Argon2id outside both locks, guarded by `epoch` so a concurrent `lock()`
/// never gets overwritten by a stale unlock.
pub(crate) fn unlock(
    store_mutex: &Mutex<Store>,
    key_mutex: &Mutex<VaultKeyState>,
    master_password: String,
) -> Result<(), MobileError> {
    let captured_epoch = {
        let state = key_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
        state.epoch
    };
    let meta = {
        let store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
        store.vault_meta()?.ok_or_else(|| MobileError::NotFound {
            reason: "Vault is not set up".into(),
        })?
    };
    let master = meta.master.ok_or_else(|| MobileError::InvalidInput {
        reason: "Vault requires account sign-in to unlock".into(),
    })?;
    let salt_bytes: [u8; Salt::LEN] =
        master
            .salt
            .as_slice()
            .try_into()
            .map_err(|_| MobileError::Storage {
                reason: "Stored vault header is invalid".into(),
            })?;
    let salt = Salt(salt_bytes);
    let params = KdfParams {
        memory_kib: master.kdf_memory_kib,
        iterations: master.kdf_iterations,
        parallelism: master.kdf_parallelism,
    };
    let secret = SecretString::from(master_password);
    // Slow Argon2id derivation runs with NO locks held.
    let kek = derive_key(&secret, &salt, params).map_err(|_| MobileError::Storage {
        reason: "Could not derive vault key".into(),
    })?;
    let raw = open(&kek, &master.wrapped_key, KEY_AAD).map_err(|_| MobileError::InvalidInput {
        reason: "Incorrect master password".into(),
    })?;
    let mut key_bytes: [u8; SecretKey::LEN] =
        raw.as_slice()
            .try_into()
            .map_err(|_| MobileError::Storage {
                reason: "Stored vault key is invalid".into(),
            })?;
    let vault_key = SecretKey::from_bytes(&mut key_bytes);

    let mut state = key_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    if state.epoch == captured_epoch {
        state.key = Some(vault_key);
        state.epoch = state.epoch.wrapping_add(1);
    }
    Ok(())
}

pub(crate) fn lock(state: &mut VaultKeyState) {
    state.key = None;
    state.epoch = state.epoch.wrapping_add(1);
}

pub(crate) fn list_items(
    store: &Store,
    state: &VaultKeyState,
    query: Option<String>,
) -> Result<Vec<VaultEntrySummary>, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let q = query.unwrap_or_default();
    let q = q.trim();

    let mut out = Vec::new();
    for sealed in store.vault_items()? {
        // Skip single corrupt or newer-format items so one bad row never bricks the list.
        let Ok(item) = open_item(key, &sealed) else {
            continue;
        };
        if item.is_trashed() {
            continue;
        }
        if !q.is_empty() && item.matches(q).is_none() {
            continue;
        }
        out.push(VaultEntrySummary {
            id: item.id().to_string(),
            kind: kind_code(item.kind()),
            title: item.title().to_owned(),
            subtitle: item.subtitle(),
            has_totp: item.otp_setup().is_some(),
            favorite: item.is_favorite(),
        });
    }
    out.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    Ok(out)
}

pub(crate) fn get_item(
    store: &Store,
    state: &VaultKeyState,
    id: &str,
) -> Result<VaultEntryDetail, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let item_id = parse_vault_item_id(id)?;
    let sealed = store
        .vault_item(item_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Vault item not found".into(),
        })?;
    let item = open_item(key, &sealed)?;
    Ok(to_detail(&item))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_login(
    store: &mut Store,
    state: &VaultKeyState,
    id: Option<String>,
    title: String,
    username: String,
    mut password: String,
    url: String,
    mut totp_secret: String,
    mut notes: String,
) -> Result<VaultEntryDetail, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let now = clock::now();

    let item = if let Some(existing_id) = id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let item_id = parse_vault_item_id(existing_id)?;
        let sealed = store
            .vault_item(item_id)?
            .ok_or_else(|| MobileError::NotFound {
                reason: "Vault item not found".into(),
            })?;
        let mut existing = open_item(key, &sealed)?;
        if existing.kind() != ItemKind::Login {
            password.zeroize();
            totp_secret.zeroize();
            notes.zeroize();
            return Err(MobileError::InvalidInput {
                reason: "Only login items can be edited here".into(),
            });
        }
        // Clone existing fields so extra URLs, tags, custom fields, and templates are preserved.
        let mut fields = Zeroizing::new(existing.fields().clone());
        fields.title = title;
        fields.username = username;
        fields.password = std::mem::take(&mut password);
        let trimmed_url = url.trim();
        if trimmed_url.is_empty() {
            if !fields.urls.is_empty() {
                fields.urls.remove(0);
            }
        } else if fields.urls.is_empty() {
            fields.urls.push(trimmed_url.to_owned());
        } else {
            fields.urls[0] = trimmed_url.to_owned();
        }
        fields.totp = totp_secret.trim().to_owned();
        totp_secret.zeroize();
        fields.notes = std::mem::take(&mut notes);

        existing.update(&fields, now)?;
        existing
    } else {
        let mut fields = Zeroizing::new(ItemFields::new(ItemKind::Login));
        fields.title = title;
        fields.username = username;
        fields.password = std::mem::take(&mut password);
        if !url.trim().is_empty() {
            fields.urls = vec![url.trim().to_owned()];
        }
        fields.totp = totp_secret.trim().to_owned();
        totp_secret.zeroize();
        fields.notes = std::mem::take(&mut notes);

        VaultItem::new(&fields, now)?
    };

    let sealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&sealed))?;
    enqueue_vault_item(store, &sealed);
    Ok(to_detail(&item))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_card(
    store: &mut Store,
    state: &VaultKeyState,
    id: Option<String>,
    title: String,
    cardholder: String,
    mut number: String,
    expiry: String,
    mut cvv: String,
    mut notes: String,
) -> Result<VaultEntryDetail, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let now = clock::now();
    let item = if let Some(raw_id) = id {
        let item_id = parse_vault_item_id(&raw_id)?;
        let sealed = store
            .vault_item(item_id)?
            .ok_or_else(|| MobileError::NotFound {
                reason: "Vault item not found".into(),
            })?;
        let mut existing = open_item(key, &sealed)?;
        if existing.kind() != ItemKind::Card {
            number.zeroize();
            cvv.zeroize();
            notes.zeroize();
            return Err(MobileError::InvalidInput {
                reason: "Item is not a card".into(),
            });
        }
        let mut fields = Zeroizing::new(existing.fields().clone());
        fields.title = title;
        fields.cardholder = cardholder;
        fields.number = std::mem::take(&mut number);
        fields.expiry = expiry;
        fields.cvv = std::mem::take(&mut cvv);
        fields.notes = std::mem::take(&mut notes);
        existing.update(&fields, now)?;
        existing
    } else {
        let mut fields = Zeroizing::new(ItemFields::new(ItemKind::Card));
        fields.title = title;
        fields.cardholder = cardholder;
        fields.number = std::mem::take(&mut number);
        fields.expiry = expiry;
        fields.cvv = std::mem::take(&mut cvv);
        fields.notes = std::mem::take(&mut notes);
        VaultItem::new(&fields, now)?
    };
    let sealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&sealed))?;
    enqueue_vault_item(store, &sealed);
    Ok(to_detail(&item))
}

pub(crate) fn save_secure_note(
    store: &mut Store,
    state: &VaultKeyState,
    id: Option<String>,
    title: String,
    mut notes: String,
) -> Result<VaultEntryDetail, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let now = clock::now();
    let item = if let Some(raw_id) = id {
        let item_id = parse_vault_item_id(&raw_id)?;
        let sealed = store
            .vault_item(item_id)?
            .ok_or_else(|| MobileError::NotFound {
                reason: "Vault item not found".into(),
            })?;
        let mut existing = open_item(key, &sealed)?;
        if existing.kind() != ItemKind::Note {
            notes.zeroize();
            return Err(MobileError::InvalidInput {
                reason: "Item is not a secure note".into(),
            });
        }
        let mut fields = Zeroizing::new(existing.fields().clone());
        fields.title = title;
        fields.notes = std::mem::take(&mut notes);
        existing.update(&fields, now)?;
        existing
    } else {
        let mut fields = Zeroizing::new(ItemFields::new(ItemKind::Note));
        fields.title = title;
        fields.notes = std::mem::take(&mut notes);
        VaultItem::new(&fields, now)?
    };
    let sealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&sealed))?;
    enqueue_vault_item(store, &sealed);
    Ok(to_detail(&item))
}

pub(crate) fn toggle_favorite(
    store: &mut Store,
    state: &VaultKeyState,
    id: &str,
) -> Result<bool, MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let item_id = parse_vault_item_id(id)?;
    let sealed = store
        .vault_item(item_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Vault item not found".into(),
        })?;
    let mut item = open_item(key, &sealed)?;
    let next = !item.is_favorite();
    item.set_favorite(next, clock::now())?;
    let resealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&resealed))?;
    enqueue_vault_item(store, &resealed);
    Ok(next)
}

pub(crate) fn autofill_matches(
    store: &Store,
    state: &VaultKeyState,
    hint: &str,
) -> Result<Vec<VaultEntryDetail>, MobileError> {
    let Some(key) = state.key.as_ref() else {
        return Ok(Vec::new());
    };
    let needle = hint.trim().to_ascii_lowercase();
    let sealed = store.vault_items()?;
    let mut matches = Vec::new();
    for s in sealed {
        let Ok(item) = open_item(key, &s) else {
            continue;
        };
        if item.is_trashed() || item.kind() != ItemKind::Login {
            continue;
        }
        let f = item.fields();
        let matched = needle.is_empty()
            || item.title().to_ascii_lowercase().contains(&needle)
            || f.urls
                .iter()
                .any(|u| u.to_ascii_lowercase().contains(&needle));
        if matched {
            matches.push(to_detail(&item));
            if matches.len() >= 6 {
                break;
            }
        }
    }
    Ok(matches)
}

pub(crate) fn trash_item(
    store: &mut Store,
    state: &VaultKeyState,
    id: &str,
) -> Result<(), MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let item_id = parse_vault_item_id(id)?;
    let sealed = store
        .vault_item(item_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Vault item not found".into(),
        })?;
    let mut item = open_item(key, &sealed)?;
    item.trash(clock::now())?;
    let resealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&resealed))?;
    enqueue_vault_item(store, &resealed);
    Ok(())
}

pub(crate) fn restore_item(
    store: &mut Store,
    state: &VaultKeyState,
    id: &str,
) -> Result<(), MobileError> {
    let key = state
        .key
        .as_ref()
        .ok_or_else(|| MobileError::InvalidInput {
            reason: "Vault is locked".into(),
        })?;
    let item_id = parse_vault_item_id(id)?;
    let sealed = store
        .vault_item(item_id)?
        .ok_or_else(|| MobileError::NotFound {
            reason: "Vault item not found".into(),
        })?;
    let mut item = open_item(key, &sealed)?;
    item.restore(clock::now());
    let resealed = seal_item(key, &item)?;
    store.save_vault_items(std::slice::from_ref(&resealed))?;
    enqueue_vault_item(store, &resealed);
    Ok(())
}

pub(crate) fn generate_password(length: u8) -> Result<String, MobileError> {
    let options = generator::Options {
        length,
        ..generator::Options::default()
    };
    let generated =
        generator::generate(&options, &mut |n| random_below(n).ok()).ok_or_else(|| {
            MobileError::Storage {
                reason: "Could not generate password".into(),
            }
        })?;
    Ok((*generated).clone())
}

pub(crate) fn totp_now(secret: &str) -> Result<TotpCodeSnapshot, MobileError> {
    let parsed = totp::parse(secret)?;
    let now_secs = u64::try_from(clock::now().as_second().max(0)).unwrap_or(0);
    let code = parsed.code_at(now_secs);
    Ok(TotpCodeSnapshot {
        grouped: code.grouped(),
        code: code.code,
        seconds_left: u32::try_from(code.seconds_left).unwrap_or(30),
        period: u32::try_from(code.period).unwrap_or(30),
    })
}
