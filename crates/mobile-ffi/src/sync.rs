//! Ottero Cloud account sign-in (with 2FA), automatic account-linked vault
//! unlock, and end-to-end encrypted push/pull sync via `ottero-sync-client`.

use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ottero_core::clock;
use ottero_crypto::account::{
    AccountKdf, AccountSecretKey, decode_salt, derive_account_keys, normalize_email,
    unwrap_account_key,
};
use ottero_crypto::{SecretKey, SecretString, Zeroize, Zeroizing, vault_link};
use ottero_storage::{AccountWrap, Store, VaultMeta};
use ottero_sync_client::{
    DEFAULT_SYNC_URL, SyncClient, SyncVaultMetaPayload, VAULT_META_SYNC_ID, apply_decrypted_page,
    apply_inline_remote_changes, derive_sync_account_id as shared_derive_sync_account_id,
    enqueue_full_snapshot, enqueue_local_change, sync_once_with_apply,
};
use ottero_sync_protocol::{Collection, WsServerMessage};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::dto::{
    AccountSessionBundle, MobileAccountStatus, MobileSignInResult, MobileSignInStatus,
    MobileSyncReport, SyncWsCredentials,
};
use crate::error::MobileError;
use crate::vault::VaultKeyState;

const ACCOUNT_BASE_URL: &str = "https://account.nexdark.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const META_VERSION: u32 = 1;

pub(crate) struct ActiveAccount {
    pub(crate) email: String,
    pub(crate) account_id: String,
    pub(crate) bearer_token: Zeroizing<String>,
    pub(crate) account_key: SecretKey,
}

pub(crate) struct PendingTwoStep {
    pub(crate) cookie: Zeroizing<String>,
    pub(crate) wrap_key: SecretKey,
}

pub(crate) struct AccountSessionState {
    pub(crate) active: Option<ActiveAccount>,
    pub(crate) pending_two_step: Option<PendingTwoStep>,
}

impl AccountSessionState {
    pub(crate) fn new() -> Self {
        Self {
            active: None,
            pending_two_step: None,
        }
    }
}

fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(format!("Ottero/{} (Android)", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

struct ApiReply {
    status: u16,
    body: Zeroizing<Vec<u8>>,
    auth_token: Option<Zeroizing<String>>,
    two_step_cookie: Option<Zeroizing<String>>,
}

fn read_reply(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ApiReply, MobileError> {
    let mut response = result.map_err(|_| MobileError::Storage {
        reason: "Could not reach Ottero Cloud. Check your connection.".into(),
    })?;
    let status = response.status().as_u16();
    let auth_token = response
        .headers()
        .get("set-auth-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(|v| Zeroizing::new(v.to_owned()));
    let two_step_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(extract_two_step_cookie);
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|_| MobileError::Storage {
            reason: "Could not read response from Ottero Cloud.".into(),
        })?;
    Ok(ApiReply {
        status,
        body: Zeroizing::new(body),
        auth_token,
        two_step_cookie,
    })
}

fn extract_two_step_cookie(line: &str) -> Option<Zeroizing<String>> {
    let pair = line.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let ours = name
        .trim_start_matches("__Secure-")
        .trim_start_matches("__Host-")
        == "better-auth.two_factor";
    (ours && !value.is_empty()).then(|| Zeroizing::new(pair.to_owned()))
}

fn post_json(
    agent: &ureq::Agent,
    path: &str,
    payload: &serde_json::Value,
    bearer: Option<&str>,
    cookie: Option<&str>,
) -> Result<ApiReply, MobileError> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(payload).map_err(|_| MobileError::Storage {
            reason: "Could not encode request".into(),
        })?,
    );
    let mut req = agent
        .post(format!("{ACCOUNT_BASE_URL}{path}"))
        .header("content-type", "application/json")
        .header("origin", ACCOUNT_BASE_URL);
    if let Some(tok) = bearer {
        req = req.header("authorization", format!("Bearer {tok}"));
    }
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    read_reply(req.send(bytes.as_slice()))
}

fn get_authed(agent: &ureq::Agent, path: &str, bearer: &str) -> Result<ApiReply, MobileError> {
    read_reply(
        agent
            .get(format!("{ACCOUNT_BASE_URL}{path}"))
            .header("authorization", format!("Bearer {bearer}"))
            .call(),
    )
}

/// Derives the separator-safe sync `account_id` used in AEAD AAD, delegating
/// to `ottero_sync_client::derive_sync_account_id` shared with Desktop.
pub(crate) fn derive_sync_account_id(email: &str) -> String {
    shared_derive_sync_account_id(email)
}

fn fetch_me_and_unwrap_key(
    agent: &ureq::Agent,
    bearer: &str,
    wrap_key: &SecretKey,
) -> Result<(String, String, SecretKey), MobileError> {
    #[derive(Deserialize)]
    struct MeBody {
        email: String,
    }
    #[derive(Deserialize)]
    struct KeysBody {
        wrapped_by_password: String,
    }

    let me_reply = get_authed(agent, "/api/me", bearer)?;
    if !(200..300).contains(&me_reply.status) {
        return Err(MobileError::InvalidInput {
            reason: "Session expired. Please sign in again.".into(),
        });
    }
    let me: MeBody = serde_json::from_slice(&me_reply.body).map_err(|_| MobileError::Storage {
        reason: "Unexpected account response".into(),
    })?;

    let keys_reply = get_authed(agent, "/api/keys", bearer)?;
    if !(200..300).contains(&keys_reply.status) {
        return Err(MobileError::InvalidInput {
            reason: "Account encryption keys are not set up yet. Finish setup on Desktop first."
                .into(),
        });
    }
    let keys: KeysBody =
        serde_json::from_slice(&keys_reply.body).map_err(|_| MobileError::Storage {
            reason: "Unexpected keys response".into(),
        })?;
    let account_key = unwrap_account_key(wrap_key, &keys.wrapped_by_password).map_err(|_| {
        MobileError::InvalidInput {
            reason: "Wrong email, password, or Secret Key.".into(),
        }
    })?;

    let canonical_email = normalize_email(&me.email);
    let sync_account_id = derive_sync_account_id(&canonical_email);
    Ok((sync_account_id, canonical_email, account_key))
}

fn ensure_device_id(store: &Store, account_id: &str) -> Result<String, MobileError> {
    let mut state = store.sync_state()?;
    let mut changed = false;
    if state.device_id.is_empty() {
        state.device_id = Uuid::now_v7().to_string();
        changed = true;
    }
    if state.account_id != account_id {
        state.account_id = account_id.to_owned();
        state.last_pull_seq = 0;
        state.needs_full_resync = true;
        changed = true;
    }
    if changed {
        store.save_sync_state(&state)?;
    }
    Ok(state.device_id)
}

/// Automatically unlocks the local Passwords Vault using `account_key` if the
/// vault is linked to this account. When `create_if_missing` is true (after an
/// initial sync confirms no remote vault exists), initializes a new
/// account-linked vault so Passwords works without a separate master password.
pub(crate) fn try_unlock_vault_with_account(
    store: &mut Store,
    vault_state: &mut VaultKeyState,
    email: &str,
    account_key: &SecretKey,
    device_id: &str,
    create_if_missing: bool,
) -> Result<bool, MobileError> {
    let normalized = normalize_email(email);
    if let Some(meta) = store.vault_meta()? {
        if let Some(account_wrap) = &meta.account
            && normalize_email(&account_wrap.email) == normalized
            && let Ok(vault_key) =
                vault_link::unwrap_vault_key(account_key, &account_wrap.wrapped_key)
        {
            vault_state.key = Some(vault_key);
            vault_state.epoch = vault_state.epoch.wrapping_add(1);
            return Ok(true);
        }
        return Ok(false);
    }

    if !create_if_missing {
        return Ok(false);
    }

    // No local or remote vault header yet — create an account-linked vault so
    // Passwords works immediately with zero extra password prompt.
    let vault_key = SecretKey::generate().map_err(|_| MobileError::Storage {
        reason: "Could not generate vault key".into(),
    })?;
    let wrapped_key =
        vault_link::wrap_vault_key(account_key, &vault_key).map_err(|_| MobileError::Storage {
            reason: "Could not wrap vault key".into(),
        })?;
    let now = clock::now();
    let now_ms = now.as_millisecond();
    let meta = VaultMeta {
        version: META_VERSION,
        master: None,
        created_at: now_ms,
        updated_at: now_ms,
        recovery_salt: None,
        recovery_wrapped: None,
        account: Some(AccountWrap {
            email: normalized,
            wrapped_key,
        }),
    };
    store.save_vault_meta(&meta)?;
    if let Some(payload) = SyncVaultMetaPayload::from_meta(&meta) {
        let _ = enqueue_local_change(
            store,
            device_id,
            Collection::Vault,
            VAULT_META_SYNC_ID,
            &[],
            false,
            &payload,
            now,
        );
    }
    vault_state.key = Some(vault_key);
    vault_state.epoch = vault_state.epoch.wrapping_add(1);
    Ok(true)
}

/// Offline-capable re-unlock of an account-linked vault using the in-memory
/// `account_key` (e.g. when returning to the foreground without network).
pub(crate) fn unlock_vault_with_account(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
) -> Result<bool, MobileError> {
    let (email, account_key) = {
        let acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        let Some(active) = acct.active.as_ref() else {
            return Ok(false);
        };
        let mut key_copy = *active.account_key.expose();
        (active.email.clone(), SecretKey::from_bytes(&mut key_copy))
    };
    let mut store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let device_id = store.sync_state()?.device_id;
    let mut vault_state = vault_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    try_unlock_vault_with_account(
        &mut store,
        &mut vault_state,
        &email,
        &account_key,
        &device_id,
        false,
    )
}

fn finish_session_install(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
    account_id: String,
    email: String,
    bearer_token: Zeroizing<String>,
    account_key: SecretKey,
) -> Result<AccountSessionBundle, MobileError> {
    let key_b64 = BASE64.encode(account_key.expose());
    let bundle = AccountSessionBundle {
        email: email.clone(),
        account_id: account_id.clone(),
        bearer_token: (*bearer_token).clone(),
        account_key_b64: key_b64,
    };

    let mut store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let device_id = ensure_device_id(&store, &account_id)?;
    let mut vault_state = vault_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let _ = try_unlock_vault_with_account(
        &mut store,
        &mut vault_state,
        &email,
        &account_key,
        &device_id,
        false,
    );
    drop(vault_state);
    drop(store);

    let mut acct = account_mutex
        .lock()
        .map_err(|_| MobileError::LockPoisoned)?;
    acct.pending_two_step = None;
    acct.active = Some(ActiveAccount {
        email,
        account_id,
        bearer_token,
        account_key,
    });
    Ok(bundle)
}

/// Signs in with email, password, and Secret Key (Argon2id + HTTPS run with NO locks held).
pub(crate) fn sign_in(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
    email: String,
    password: String,
    secret_key_raw: String,
) -> Result<MobileSignInResult, MobileError> {
    #[derive(Deserialize)]
    struct KdfBody {
        salt: String,
        kdf: String,
    }
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    struct SignInBody {
        #[serde(default)]
        two_factor_redirect: bool,
    }

    let email = normalize_email(&email);
    if email.is_empty() || password.is_empty() || secret_key_raw.trim().is_empty() {
        return Err(MobileError::InvalidInput {
            reason: "Enter your email, password, and Secret Key.".into(),
        });
    }
    let parsed_secret =
        AccountSecretKey::parse(&secret_key_raw).map_err(|_| MobileError::InvalidInput {
            reason: "Secret Key format is invalid.".into(),
        })?;
    let agent = http_agent();

    let kdf_reply = post_json(&agent, "/api/kdf", &json!({ "email": email }), None, None)?;
    if !(200..300).contains(&kdf_reply.status) {
        return Err(MobileError::InvalidInput {
            reason: "Wrong email, password, or Secret Key.".into(),
        });
    }
    let kdf_body: KdfBody =
        serde_json::from_slice(&kdf_reply.body).map_err(|_| MobileError::Storage {
            reason: "Unexpected KDF response".into(),
        })?;
    let salt = decode_salt(&kdf_body.salt).map_err(|_| MobileError::Storage {
        reason: "Invalid KDF salt".into(),
    })?;
    let kdf = AccountKdf::parse(&kdf_body.kdf).map_err(|_| MobileError::Storage {
        reason: "Unsupported KDF parameters".into(),
    })?;

    let secret_pw = SecretString::from(password);
    let derived =
        derive_account_keys(&secret_pw, &email, &salt, &parsed_secret, kdf).map_err(|_| {
            MobileError::Storage {
                reason: "Could not derive account keys".into(),
            }
        })?;
    let auth_hex = derived.auth_key().to_hex();

    let sign_in_reply = post_json(
        &agent,
        "/api/auth/sign-in/email",
        &json!({ "email": email, "password": auth_hex.as_str() }),
        None,
        None,
    )?;
    if !(200..300).contains(&sign_in_reply.status) {
        return Err(MobileError::InvalidInput {
            reason: "Wrong email, password, or Secret Key.".into(),
        });
    }
    let body: SignInBody = serde_json::from_slice(&sign_in_reply.body).unwrap_or_default();
    if body.two_factor_redirect {
        let cookie = sign_in_reply
            .two_step_cookie
            .ok_or_else(|| MobileError::Storage {
                reason: "Missing two-step verification cookie".into(),
            })?;
        let mut wrap_bytes = *derived.wrap_key().expose();
        let wrap_key = SecretKey::from_bytes(&mut wrap_bytes);
        let mut acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        acct.pending_two_step = Some(PendingTwoStep { cookie, wrap_key });
        return Ok(MobileSignInResult {
            status: MobileSignInStatus::NeedsTwoStep,
            session: None,
        });
    }

    let token = sign_in_reply
        .auth_token
        .ok_or_else(|| MobileError::Storage {
            reason: "Missing session token".into(),
        })?;
    let (account_id, canonical_email, account_key) =
        fetch_me_and_unwrap_key(&agent, &token, derived.wrap_key())?;
    let bundle = finish_session_install(
        store_mutex,
        vault_mutex,
        account_mutex,
        account_id,
        canonical_email,
        token,
        account_key,
    )?;
    Ok(MobileSignInResult {
        status: MobileSignInStatus::SignedIn,
        session: Some(bundle),
    })
}

/// Completes two-step verification (authenticator TOTP or backup code).
pub(crate) fn verify_two_step(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
    code: String,
    is_backup_code: bool,
) -> Result<MobileSignInResult, MobileError> {
    let (cookie, wrap_key) = {
        let acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        let pending = acct
            .pending_two_step
            .as_ref()
            .ok_or_else(|| MobileError::InvalidInput {
                reason: "Sign-in timed out. Enter your password again.".into(),
            })?;
        let mut wrap_bytes = *pending.wrap_key.expose();
        (
            pending.cookie.clone(),
            SecretKey::from_bytes(&mut wrap_bytes),
        )
    };

    let path = if is_backup_code {
        "/api/auth/two-factor/verify-backup-code"
    } else {
        "/api/auth/two-factor/verify-totp"
    };
    let agent = http_agent();
    let reply = post_json(
        &agent,
        path,
        &json!({ "code": code.trim() }),
        None,
        Some(&cookie),
    )?;
    if !(200..300).contains(&reply.status) {
        return Err(MobileError::InvalidInput {
            reason: "That code is not valid. Check and try again.".into(),
        });
    }
    let token = reply.auth_token.ok_or_else(|| MobileError::Storage {
        reason: "Missing session token".into(),
    })?;
    let (account_id, canonical_email, account_key) =
        fetch_me_and_unwrap_key(&agent, &token, &wrap_key)?;
    let bundle = finish_session_install(
        store_mutex,
        vault_mutex,
        account_mutex,
        account_id,
        canonical_email,
        token,
        account_key,
    )?;
    Ok(MobileSignInResult {
        status: MobileSignInStatus::SignedIn,
        session: Some(bundle),
    })
}

/// Restores a previously signed-in session decrypted from Android Keystore.
pub(crate) fn restore_session(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
    email: String,
    account_id: String,
    bearer_token: String,
    mut account_key_b64: String,
) -> Result<(), MobileError> {
    let raw = Zeroizing::new(BASE64.decode(account_key_b64.trim()).map_err(|_| {
        MobileError::InvalidInput {
            reason: "Invalid stored session key".into(),
        }
    })?);
    account_key_b64.zeroize();
    let mut key_bytes: [u8; SecretKey::LEN] =
        raw.as_slice()
            .try_into()
            .map_err(|_| MobileError::InvalidInput {
                reason: "Invalid stored session key length".into(),
            })?;
    let account_key = SecretKey::from_bytes(&mut key_bytes);

    let canonical_email = normalize_email(&email);
    let derived_account_id = derive_sync_account_id(&canonical_email);
    let _ = account_id;

    let _ = finish_session_install(
        store_mutex,
        vault_mutex,
        account_mutex,
        derived_account_id,
        canonical_email,
        Zeroizing::new(bearer_token),
        account_key,
    )?;
    Ok(())
}

/// Signs out on the server (best-effort) and clears in-memory session + vault key.
pub(crate) fn sign_out(
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
) -> Result<(), MobileError> {
    let token = {
        let mut acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        acct.pending_two_step = None;
        acct.active.take().map(|a| a.bearer_token)
    };
    {
        let mut vault = vault_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
        crate::vault::lock(&mut vault);
    }
    if let Some(tok) = token {
        let agent = http_agent();
        let _ = post_json(&agent, "/api/auth/sign-out", &json!({}), Some(&tok), None);
    }
    Ok(())
}

pub(crate) fn account_status(
    store: &Store,
    account_state: &AccountSessionState,
) -> Result<MobileAccountStatus, MobileError> {
    let sync = store.sync_state()?;
    let pending = store.pending_sync_count()?;
    Ok(MobileAccountStatus {
        signed_in: account_state.active.is_some(),
        email: account_state.active.as_ref().map(|a| a.email.clone()),
        last_synced_at_ms: sync.last_synced_at.map(|t| t.as_millisecond()),
        pending_outbox_count: u32::try_from(pending).unwrap_or(u32::MAX),
    })
}

/// Returns the WebSocket URL (`wss://.../v1/sync/ws`), bearer token, and current
/// pull cursor for the active signed-in session so Android's foreground
/// realtime listener can receive instant `seq_advanced` notices.
pub(crate) fn ws_credentials(
    store: &Store,
    account_state: &AccountSessionState,
) -> Result<Option<SyncWsCredentials>, MobileError> {
    let Some(active) = account_state.active.as_ref() else {
        return Ok(None);
    };
    let sync = store.sync_state()?;
    let ws_base = DEFAULT_SYNC_URL
        .trim_end_matches('/')
        .replacen("https://", "wss://", 1);
    Ok(Some(SyncWsCredentials {
        ws_url: format!("{ws_base}/v1/sync/ws"),
        bearer_token: (*active.bearer_token).clone(),
        last_pull_seq: sync.last_pull_seq,
    }))
}

/// Runs one full end-to-end encrypted push + pull sync cycle with Ottero Cloud.
pub(crate) fn sync_now(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
) -> Result<MobileSyncReport, MobileError> {
    let (email, account_id, bearer, account_key) = {
        let acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        let active = acct
            .active
            .as_ref()
            .ok_or_else(|| MobileError::InvalidInput {
                reason: "Sign in to Ottero Cloud to sync.".into(),
            })?;
        let mut key_copy = *active.account_key.expose();
        (
            active.email.clone(),
            active.account_id.clone(),
            active.bearer_token.clone(),
            SecretKey::from_bytes(&mut key_copy),
        )
    };

    let client = SyncClient::shared(DEFAULT_SYNC_URL).map_err(|_| MobileError::Storage {
        reason: "Invalid sync server URL".into(),
    })?;
    let now = clock::now();

    let mut store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let device_id = ensure_device_id(&store, &account_id)?;
    let state = store.sync_state()?;
    if state.needs_full_resync {
        let _ = enqueue_full_snapshot(&store, &device_id, now);
    }

    let report = sync_once_with_apply(
        &mut store,
        &client,
        &bearer,
        &account_id,
        &device_id,
        &account_key,
        now,
        |s, page| {
            apply_decrypted_page(s, page)?;
            Ok(())
        },
    )
    .map_err(|e| MobileError::Storage {
        reason: e.to_string(),
    })?;

    // If remote sync delivered or updated `vault-meta`, unlock Passwords automatically.
    // If no vault exists on any device yet, initialize an account-linked vault now.
    let mut vault_state = vault_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let _ = try_unlock_vault_with_account(
        &mut store,
        &mut vault_state,
        &email,
        &account_key,
        &device_id,
        true,
    );

    Ok(MobileSyncReport {
        pushed: u32::try_from(report.pushed).unwrap_or(u32::MAX),
        pulled: u32::try_from(report.pulled).unwrap_or(u32::MAX),
        latest_seq: report.latest_seq,
    })
}

/// Fast-path for inline encrypted WebSocket push (`SeqAdvanced` with `changes`).
/// Parses the raw JSON frame, decrypts and applies contiguous changes directly
/// to SQLite with zero HTTP round-trips, and returns `true` if handled.
pub(crate) fn apply_ws_frame(
    store_mutex: &Mutex<Store>,
    vault_mutex: &Mutex<VaultKeyState>,
    account_mutex: &Mutex<AccountSessionState>,
    raw_frame: &str,
) -> Result<bool, MobileError> {
    let Ok(WsServerMessage::SeqAdvanced {
        latest_seq,
        changes,
    }) = serde_json::from_str::<WsServerMessage>(raw_frame)
    else {
        return Ok(false);
    };
    if changes.is_empty() {
        return Ok(false);
    }

    let (email, account_id, account_key) = {
        let acct = account_mutex
            .lock()
            .map_err(|_| MobileError::LockPoisoned)?;
        let Some(active) = acct.active.as_ref() else {
            return Ok(false);
        };
        let mut key_copy = *active.account_key.expose();
        (
            active.email.clone(),
            active.account_id.clone(),
            SecretKey::from_bytes(&mut key_copy),
        )
    };

    let mut store = store_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
    let device_id = ensure_device_id(&store, &account_id)?;
    let Ok(Some(touched)) = apply_inline_remote_changes(
        &mut store,
        &account_id,
        &device_id,
        &account_key,
        clock::now(),
        latest_seq,
        &changes,
    ) else {
        return Ok(false);
    };

    if touched.contains(&Collection::Vault) {
        let mut vault_state = vault_mutex.lock().map_err(|_| MobileError::LockPoisoned)?;
        let _ = try_unlock_vault_with_account(
            &mut store,
            &mut vault_state,
            &email,
            &account_key,
            &device_id,
            false,
        );
    }
    Ok(true)
}

/// Returns the active sync `device_id` if this device is signed in to Ottero Cloud.
pub(crate) fn active_device_id(store: &Store) -> Option<String> {
    let state = store.sync_state().ok()?;
    if state.account_id.is_empty() || state.device_id.is_empty() {
        return None;
    }
    Some(state.device_id)
}
