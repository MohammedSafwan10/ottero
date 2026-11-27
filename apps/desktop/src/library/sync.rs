//! Cloud sync state and cycle orchestration on [`Library`].

use std::time::Duration;

use anyhow::Result;
use gpui_kit::Context;

use ottero_core::account::AccountState;
use ottero_core::clock;
use ottero_core::vault::lock::{LockState, Opener};

use crate::vault::VaultSession;

use super::{Library, LibraryEvent};

const AUTO_SYNC_DEBOUNCE: Duration = Duration::from_millis(350);
const AUTO_SYNC_POLL_INTERVAL: Duration = Duration::from_secs(60);
const FOCUS_SYNC_MIN_GAP_MS: i64 = 10_000;

impl Library {
    // ---- Sync ------------------------------------------------------------

    /// Enables automatic cloud sync (sign-in/unlock trigger, debounced local
    /// writes, 60 s background fallback timer, and live WebSocket push notices).
    /// Called only from real desktop startup so headless tests never hit the network.
    pub fn enable_auto_sync(&mut self, cx: &mut Context<Self>) {
        if self.auto_sync_enabled {
            return;
        }
        self.auto_sync_enabled = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(AUTO_SYNC_POLL_INTERVAL)
                    .await;
                let alive = this.update(cx, |lib, cx| {
                    if lib.can_auto_sync() {
                        lib.ensure_sync_ws(cx);
                        if !lib.sync_in_flight {
                            lib.sync_now(cx);
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    #[must_use]
    pub(crate) fn can_auto_sync(&self) -> bool {
        if !self.auto_sync_enabled {
            return false;
        }
        if !self.account.is_unlocked() || self.account.current_token().is_none() {
            return false;
        }
        matches!(
            self.account.state(),
            AccountState::SignedIn { plan, .. } if plan.has_cloud()
        )
    }

    /// Called when the account key opens (sign-in or unlock): starts the live
    /// WebSocket listener and runs an immediate sync if the account has Cloud.
    pub(crate) fn on_account_unlocked_for_sync(&mut self, cx: &mut Context<Self>) {
        if !self.can_auto_sync() {
            return;
        }
        self.ensure_sync_ws(cx);
        self.sync_now(cx);
    }

    /// Stops any active WebSocket listener (on lock or sign-out).
    pub(crate) fn stop_sync_ws(&mut self) {
        self.sync_ws_handle = None;
        self.sync_ws_epoch = None;
        self.sync_needed_again = false;
    }

    /// Starts the WebSocket push listener if not already connected for this epoch.
    pub(crate) fn ensure_sync_ws(&mut self, cx: &mut Context<Self>) {
        if !self.can_auto_sync() {
            return;
        }
        let epoch = self.account.epoch();
        if self.sync_ws_epoch == Some(epoch) && self.sync_ws_handle.is_some() {
            return;
        }
        let Some(token) = self.account.current_token() else {
            return;
        };
        let sync_url = crate::sync::client::resolve_sync_url();
        let (tx, rx) = async_channel::bounded::<crate::sync::ws::WsNotice>(16);
        let Some(handle) = crate::sync::ws::spawn_ws_listener(
            &sync_url,
            ottero_crypto::Zeroizing::new(token.expose().to_owned()),
            tx,
        ) else {
            return;
        };
        self.sync_ws_handle = Some(handle);
        self.sync_ws_epoch = Some(epoch);
        cx.spawn(async move |this, cx| {
            while let Ok(notice) = rx.recv().await {
                let keep = this.update(cx, |lib, cx| {
                    if lib.account.epoch() != epoch {
                        return false;
                    }
                    match notice {
                        crate::sync::ws::WsNotice::SeqAdvanced {
                            latest_seq,
                            changes,
                        } => {
                            if !lib.can_auto_sync() {
                                lib.stop_sync_ws();
                                return false;
                            }
                            let last = lib.store.sync_state().map(|s| s.last_pull_seq).unwrap_or(0);
                            if latest_seq > last {
                                // Fast path: decrypt and apply inline WS changes
                                // directly (~2 ms, zero HTTP round-trip).
                                let applied_inline = !lib.sync_in_flight
                                    && !changes.is_empty()
                                    && lib.try_apply_ws_inline_changes(latest_seq, &changes, cx);
                                if !applied_inline {
                                    if lib.sync_in_flight {
                                        lib.sync_needed_again = true;
                                    } else {
                                        lib.sync_now(cx);
                                    }
                                }
                            }
                            true
                        }
                        crate::sync::ws::WsNotice::DeviceRevoked
                        | crate::sync::ws::WsNotice::AuthRejected => {
                            lib.stop_sync_ws();
                            lib.refresh_account(cx);
                            false
                        }
                        crate::sync::ws::WsNotice::EntitlementChanged(active) => {
                            if !active {
                                lib.stop_sync_ws();
                            }
                            lib.refresh_account(cx);
                            active
                        }
                    }
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Attempts to decrypt and apply inline WebSocket changes directly to
    /// SQLite without an HTTP `/pull` round-trip. Returns `true` when handled.
    fn try_apply_ws_inline_changes(
        &mut self,
        latest_seq: i64,
        changes: &[ottero_sync_protocol::RemoteChange],
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open_key) = self.account.key() else {
            return false;
        };
        let Ok(state) = self.store.sync_state() else {
            return false;
        };
        if state.account_id.is_empty() || state.needs_full_resync {
            return false;
        }
        let Ok(device_id) = self.sync_device_id() else {
            return false;
        };
        let account_key = {
            let mut raw = ottero_crypto::Zeroizing::new([0u8; ottero_crypto::SecretKey::LEN]);
            raw.copy_from_slice(open_key.expose());
            ottero_crypto::SecretKey::from_bytes(&mut raw)
        };
        let meta_before = self
            .store
            .vault_meta()
            .ok()
            .flatten()
            .and_then(|m| m.account.map(|a| a.wrapped_key));
        let Ok(Some(touched)) = ottero_sync_client::apply_inline_remote_changes(
            &mut self.store,
            &state.account_id,
            &device_id,
            &account_key,
            clock::now(),
            latest_seq,
            changes,
        ) else {
            return false;
        };

        let mut notes_changed = false;
        let mut tasks_changed = false;
        let mut events_changed = false;
        let mut habits_changed = false;
        let mut focus_changed = false;
        let mut vault_changed = false;
        for c in touched {
            match c {
                ottero_sync_protocol::Collection::Notes => notes_changed = true,
                ottero_sync_protocol::Collection::Tasks
                | ottero_sync_protocol::Collection::Projects => tasks_changed = true,
                ottero_sync_protocol::Collection::Events => events_changed = true,
                ottero_sync_protocol::Collection::Habits => habits_changed = true,
                ottero_sync_protocol::Collection::Focus => focus_changed = true,
                ottero_sync_protocol::Collection::Vault => vault_changed = true,
                _ => {}
            }
        }
        let meta_after = self
            .store
            .vault_meta()
            .ok()
            .flatten()
            .and_then(|m| m.account.map(|a| a.wrapped_key));
        let vault_meta_changed = meta_before != meta_after;
        self.sync_last_error = None;
        if vault_changed || vault_meta_changed {
            self.refresh_vault_after_sync(vault_meta_changed);
        }
        if notes_changed {
            cx.emit(LibraryEvent::NotesChanged);
        }
        if tasks_changed {
            cx.emit(LibraryEvent::TasksChanged);
        }
        if events_changed {
            cx.emit(LibraryEvent::EventsChanged);
        }
        if habits_changed {
            cx.emit(LibraryEvent::HabitsChanged);
        }
        if focus_changed {
            cx.emit(LibraryEvent::FocusChanged);
        }
        if vault_changed || vault_meta_changed {
            cx.emit(LibraryEvent::VaultChanged);
        }
        cx.emit(LibraryEvent::AccountChanged);
        true
    }

    /// Triggers an immediate sync for discrete user actions (task toggle,
    /// habit tap, event save) without waiting for the typing debounce window.
    pub fn schedule_immediate_sync(&mut self, cx: &mut Context<Self>) {
        if !self.can_auto_sync() {
            return;
        }
        self.ensure_sync_ws(cx);
        if self.sync_in_flight {
            self.sync_needed_again = true;
            return;
        }
        self.sync_debounce_gen = self.sync_debounce_gen.wrapping_add(1);
        self.sync_now(cx);
    }

    /// Coalesces rapid local edits into a single sync cycle after a short delay.
    pub fn schedule_debounced_sync(&mut self, cx: &mut Context<Self>) {
        if !self.can_auto_sync() {
            return;
        }
        self.ensure_sync_ws(cx);
        if self.sync_in_flight {
            self.sync_needed_again = true;
            return;
        }
        self.sync_debounce_gen = self.sync_debounce_gen.wrapping_add(1);
        let generation = self.sync_debounce_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTO_SYNC_DEBOUNCE).await;
            _ = this.update(cx, |lib, cx| {
                if lib.sync_debounce_gen == generation && lib.can_auto_sync() {
                    if lib.sync_in_flight {
                        lib.sync_needed_again = true;
                    } else {
                        lib.sync_now(cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Runs a quick sync when the window returns to the foreground, unless one
    /// already finished within the last 10 seconds.
    pub fn sync_on_focus(&mut self, cx: &mut Context<Self>) {
        if !self.can_auto_sync() || self.sync_in_flight {
            return;
        }
        self.ensure_sync_ws(cx);
        let now_ms = clock::now().as_millisecond();
        let recently_synced = self
            .store
            .sync_state()
            .ok()
            .and_then(|s| s.last_synced_at)
            .is_some_and(|t| now_ms.saturating_sub(t.as_millisecond()) < FOCUS_SYNC_MIN_GAP_MS);
        if !recently_synced || self.pending_sync_count() > 0 {
            self.sync_now(cx);
        }
    }

    /// Enqueues the account-wrapped `vault_meta` header so other devices signed
    /// into this Ottero Cloud account can decrypt synced Passwords automatically.
    pub(super) fn record_vault_meta_sync(&mut self, cx: &mut Context<Self>) {
        if let Ok(Some(meta)) = self.store.vault_meta()
            && let Some(payload) = crate::sync::SyncVaultMetaPayload::from_meta(&meta)
        {
            self.record_sync(
                ottero_sync_protocol::Collection::Vault,
                crate::sync::VAULT_META_SYNC_ID,
                &["record"],
                false,
                &payload,
            );
            self.schedule_debounced_sync(cx);
        }
    }

    /// Enqueues the account-wrapped `vault_meta` header and all existing vault
    /// items so another device receives the entire linked vault cleanly.
    pub(super) fn record_vault_meta_and_items_sync(&mut self, cx: &mut Context<Self>) {
        self.record_vault_meta_sync(cx);
        if let Ok(items) = self.store.vault_items() {
            for item in items {
                self.record_sync(
                    ottero_sync_protocol::Collection::Vault,
                    &item.id.to_string(),
                    &["record"],
                    false,
                    &crate::sync::SyncVaultPayload::from(&item),
                );
            }
        }
        self.schedule_debounced_sync(cx);
    }

    /// Refreshes local `VaultSession` after a sync cycle touched `Collection::Vault`.
    /// When `vault_meta_changed` is true (e.g., a fresh or empty device adopted a
    /// synced account-wrapped vault key), reloads `VaultSession` from disk and
    /// unlocks with the open account key so the in-memory key is never stale.
    fn refresh_vault_after_sync(&mut self, vault_meta_changed: bool) {
        if (vault_meta_changed || self.vault.state() == LockState::Uninitialized)
            && let Ok(loaded) = VaultSession::load(&self.store)
        {
            self.vault = loaded;
        }
        if self.vault.is_unlocked() {
            if let Err(err) = self.vault.reload_if_unlocked(&self.store) {
                eprintln!("ottero: couldn't reload vault after sync: {err:#}");
            }
        } else if self.vault.state() == LockState::Locked
            && self.vault.opener() == Some(Opener::Account)
            && let (Some(key), Some(email)) = (self.account.key(), self.account.signed_in_email())
        {
            match self.vault.unlock_with_account(&self.store, &email, key) {
                Ok(crate::vault::AccountUnlock::Opened) => {
                    self.vault_account_problem = None;
                    self.touch_vault();
                }
                Ok(problem) => self.vault_account_problem = Some(problem),
                Err(err) => eprintln!("ottero: couldn't unlock synced vault: {err:#}"),
            }
        }
    }

    /// Pending encrypted changes waiting in the local outbox.
    #[must_use]
    pub fn pending_sync_count(&self) -> u64 {
        self.store.pending_sync_count().unwrap_or(0)
    }

    #[must_use]
    pub fn sync_in_flight(&self) -> bool {
        self.sync_in_flight
    }

    #[must_use]
    pub fn sync_last_error(&self) -> Option<&str> {
        self.sync_last_error.as_deref()
    }

    /// Triggers a non-blocking encrypted sync cycle on the background executor.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.sync_in_flight {
            return;
        }
        let Some(email) = self.account.signed_in_email() else {
            return;
        };
        let Some(token) = self.account.current_token() else {
            self.sync_last_error = Some("Sign in again to sync.".into());
            cx.emit(LibraryEvent::AccountChanged);
            return;
        };
        let Some(open_key) = self.account.key() else {
            self.sync_last_error = Some("Unlock your account key to sync.".into());
            cx.emit(LibraryEvent::AccountChanged);
            return;
        };
        let account_key = {
            let mut raw = ottero_crypto::Zeroizing::new([0u8; ottero_crypto::SecretKey::LEN]);
            raw.copy_from_slice(open_key.expose());
            ottero_crypto::SecretKey::from_bytes(&mut raw)
        };
        let Ok(device_id) = self.sync_device_id() else {
            self.sync_last_error = Some("Couldn't initialize sync device id.".into());
            cx.emit(LibraryEvent::AccountChanged);
            return;
        };
        let now = clock::now();
        let Ok(mut state) = self.store.sync_state() else {
            self.sync_last_error = Some("Couldn't read local sync state.".into());
            cx.emit(LibraryEvent::AccountChanged);
            return;
        };
        // Use a stable separator-safe account identifier derived from email if
        // server account_id isn't stored yet, and persist it so AAD stays stable.
        // A brand-new sync_state (empty account_id) always triggers one initial
        // full snapshot so pre-existing local data is uploaded even if edits
        // happened before the first sync.
        let first_sync = state.account_id.is_empty();
        let account_id = if first_sync {
            let derived = ottero_sync_client::derive_sync_account_id(&email);
            state.account_id = derived.clone();
            state.needs_full_resync = true;
            let _ = self.store.save_sync_state(&state);
            derived
        } else {
            state.account_id.clone()
        };
        if state.needs_full_resync
            && crate::sync::enqueue_full_snapshot(&self.store, &device_id, now).is_ok()
        {
            // Re-read sync_state so we keep the fresh `last_hlc` written by
            // `enqueue_full_snapshot` instead of clobbering it with stale state.
            if let Ok(mut refreshed) = self.store.sync_state() {
                refreshed.account_id = account_id.clone();
                refreshed.needs_full_resync = false;
                refreshed.last_pull_seq = 0;
                let _ = self.store.save_sync_state(&refreshed);
                state = refreshed;
            }
        }
        let local_keys = self.store.sync_collection_keys().unwrap_or_default();
        let pending = self.store.pending_sync_changes(1_000).unwrap_or_default();
        let sync_url = crate::sync::client::resolve_sync_url();
        let input = crate::sync::engine::BackgroundSyncInput {
            sync_url,
            bearer_token: ottero_crypto::Zeroizing::new(token.expose().to_owned()),
            account_id,
            device_id,
            account_key,
            local_keys,
            pending,
            last_pull_seq: state.last_pull_seq,
            last_hlc: state.last_hlc.clone(),
            now,
        };
        let epoch = self.account.epoch();
        self.sync_in_flight = true;
        self.sync_last_error = None;
        cx.emit(LibraryEvent::AccountChanged);

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::sync::engine::run_background_sync(input) })
                .await;
            _ = this.update(cx, |lib, cx| {
                lib.sync_in_flight = false;
                if lib.account.epoch() != epoch {
                    cx.emit(LibraryEvent::AccountChanged);
                    return;
                }
                match result {
                    Ok(outcome) => {
                        for k in &outcome.authoritative_keys {
                            let _ = lib.store.save_sync_collection_key(k);
                        }
                        if !outcome.acked_ids.is_empty() {
                            let _ = lib.store.ack_sync_changes(&outcome.acked_ids);
                        }
                        let mut notes_changed = false;
                        let mut tasks_changed = false;
                        let mut events_changed = false;
                        let mut habits_changed = false;
                        let mut focus_changed = false;
                        let mut vault_changed = false;
                        let mut apply_err: Option<String> = None;
                        let mut applied_seq =
                            lib.store.sync_state().map(|s| s.last_pull_seq).unwrap_or(0);
                        let meta_before = lib
                            .store
                            .vault_meta()
                            .ok()
                            .flatten()
                            .and_then(|m| m.account.map(|a| a.wrapped_key));
                        for (next_seq, page) in &outcome.pages {
                            match crate::sync::apply_decrypted_page(&mut lib.store, page) {
                                Ok(touched) => {
                                    for c in touched {
                                        match c {
                                            ottero_sync_protocol::Collection::Notes => {
                                                notes_changed = true;
                                            }
                                            ottero_sync_protocol::Collection::Tasks
                                            | ottero_sync_protocol::Collection::Projects => {
                                                tasks_changed = true;
                                            }
                                            ottero_sync_protocol::Collection::Events => {
                                                events_changed = true;
                                            }
                                            ottero_sync_protocol::Collection::Habits => {
                                                habits_changed = true;
                                            }
                                            ottero_sync_protocol::Collection::Focus => {
                                                focus_changed = true;
                                            }
                                            ottero_sync_protocol::Collection::Vault => {
                                                vault_changed = true;
                                            }
                                            _ => {}
                                        }
                                    }
                                    applied_seq = applied_seq.max(*next_seq);
                                }
                                Err(err) => {
                                    apply_err = Some(err.to_string());
                                    break;
                                }
                            }
                        }
                        let meta_after = lib
                            .store
                            .vault_meta()
                            .ok()
                            .flatten()
                            .and_then(|m| m.account.map(|a| a.wrapped_key));
                        let vault_meta_changed = meta_before != meta_after;
                        if let Ok(mut st) = lib.store.sync_state() {
                            st.last_pull_seq = st.last_pull_seq.max(applied_seq);
                            if let Some(hlc) = outcome.last_hlc {
                                st.last_hlc = Some(match st.last_hlc {
                                    Some(existing) => existing.max(hlc),
                                    None => hlc,
                                });
                            }
                            if apply_err.is_none() {
                                st.last_synced_at = Some(clock::now());
                                st.needs_full_resync = false;
                            }
                            let _ = lib.store.save_sync_state(&st);
                        }
                        lib.sync_last_error = apply_err;
                        if vault_changed || vault_meta_changed {
                            lib.refresh_vault_after_sync(vault_meta_changed);
                        }
                        if notes_changed {
                            cx.emit(LibraryEvent::NotesChanged);
                        }
                        if tasks_changed {
                            cx.emit(LibraryEvent::TasksChanged);
                        }
                        if events_changed {
                            cx.emit(LibraryEvent::EventsChanged);
                        }
                        if habits_changed {
                            cx.emit(LibraryEvent::HabitsChanged);
                        }
                        if focus_changed {
                            cx.emit(LibraryEvent::FocusChanged);
                        }
                        if vault_changed || vault_meta_changed {
                            cx.emit(LibraryEvent::VaultChanged);
                        }
                        let made_progress = !outcome.acked_ids.is_empty();
                        let should_resync = lib.sync_needed_again
                            || (lib.can_auto_sync()
                                && made_progress
                                && lib.pending_sync_count() > 0
                                && lib.sync_last_error.is_none());
                        lib.sync_needed_again = false;
                        if should_resync {
                            lib.schedule_debounced_sync(cx);
                        }
                    }
                    Err(crate::sync::SyncClientError::CursorExpired) => {
                        if let Ok(mut st) = lib.store.sync_state() {
                            st.needs_full_resync = true;
                            st.last_pull_seq = 0;
                            let _ = lib.store.save_sync_state(&st);
                        }
                        lib.sync_last_error = Some(
                            "Sync cursor expired; click Sync now to re-upload your data.".into(),
                        );
                    }
                    Err(crate::sync::SyncClientError::EntitlementRequired) => {
                        lib.stop_sync_ws();
                        lib.sync_last_error =
                            Some("An active Ottero Cloud plan is required to sync.".into());
                    }
                    Err(crate::sync::SyncClientError::Unauthorized) => {
                        lib.stop_sync_ws();
                        lib.sync_last_error = Some("Sign in again to sync.".into());
                    }
                    Err(err) => {
                        lib.sync_last_error = Some(err.to_string());
                    }
                }
                cx.emit(LibraryEvent::AccountChanged);
            });
        })
        .detach();
    }

    /// Current sync cursor and metadata.
    pub fn sync_state(&self) -> Result<ottero_storage::SyncState> {
        Ok(self.store.sync_state()?)
    }

    pub(crate) fn clear_sync_state(&mut self) {
        if let Err(err) = self.store.clear_sync_state() {
            eprintln!("ottero: couldn't clear sync state: {err:#}");
        }
    }

    fn sync_device_id(&self) -> Result<String> {
        let mut state = self.store.sync_state()?;
        if state.device_id.is_empty() {
            state.device_id = format!("dev-{}", ottero_sync_protocol::ChangeId::new().0);
            self.store.save_sync_state(&state)?;
        }
        Ok(state.device_id)
    }

    /// Enqueues a local mutation into `sync_outbox` when signed in with an
    /// active Ottero Cloud plan. Local-only and Free users never accumulate
    /// unbounded outbox rows.
    pub(super) fn record_sync(
        &self,
        collection: ottero_sync_protocol::Collection,
        item_id: &str,
        fields: &[&str],
        deleted: bool,
        payload: &impl serde::Serialize,
    ) {
        if !matches!(
            self.account.state(),
            AccountState::SignedIn { plan, .. } if plan.has_cloud()
        ) {
            return;
        }
        let Ok(device_id) = self.sync_device_id() else {
            return;
        };
        if let Err(err) = crate::sync::enqueue_local_change(
            &self.store,
            &device_id,
            collection,
            item_id,
            fields,
            deleted,
            payload,
            clock::now(),
        ) {
            eprintln!("ottero: couldn't enqueue sync change: {err:#}");
        }
    }

    /// Runs one encrypted push + pull sync cycle and applies incoming remote
    /// changes to SQLite, emitting the relevant [`LibraryEvent`]s.
    pub fn run_sync_once(
        &mut self,
        client: &crate::sync::SyncClient,
        bearer_token: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) -> std::result::Result<crate::sync::SyncCycleReport, crate::sync::SyncClientError> {
        let device_id = self.sync_device_id().map_err(|_| {
            crate::sync::SyncClientError::Decode("couldn't resolve sync device_id".into())
        })?;
        let key = {
            let open_key = self
                .account
                .key()
                .ok_or(crate::sync::SyncClientError::Unauthorized)?;
            let mut raw = ottero_crypto::Zeroizing::new([0u8; ottero_crypto::SecretKey::LEN]);
            raw.copy_from_slice(open_key.expose());
            ottero_crypto::SecretKey::from_bytes(&mut raw)
        };
        let mut notes_changed = false;
        let mut tasks_changed = false;
        let mut events_changed = false;
        let mut habits_changed = false;
        let mut focus_changed = false;
        let mut vault_changed = false;
        let meta_before = self
            .store
            .vault_meta()
            .ok()
            .flatten()
            .and_then(|m| m.account.map(|a| a.wrapped_key));
        let report = crate::sync::sync_once_with_apply(
            &mut self.store,
            client,
            bearer_token,
            account_id,
            &device_id,
            &key,
            clock::now(),
            |store, page| {
                for collection in crate::sync::apply_decrypted_page(store, page)? {
                    match collection {
                        ottero_sync_protocol::Collection::Notes => notes_changed = true,
                        ottero_sync_protocol::Collection::Tasks
                        | ottero_sync_protocol::Collection::Projects => tasks_changed = true,
                        ottero_sync_protocol::Collection::Events => events_changed = true,
                        ottero_sync_protocol::Collection::Habits => habits_changed = true,
                        ottero_sync_protocol::Collection::Focus => focus_changed = true,
                        ottero_sync_protocol::Collection::Vault => vault_changed = true,
                        _ => {}
                    }
                }
                Ok(())
            },
        )?;
        let meta_after = self
            .store
            .vault_meta()
            .ok()
            .flatten()
            .and_then(|m| m.account.map(|a| a.wrapped_key));
        let vault_meta_changed = meta_before != meta_after;
        if vault_changed || vault_meta_changed {
            self.refresh_vault_after_sync(vault_meta_changed);
        }
        if notes_changed {
            cx.emit(LibraryEvent::NotesChanged);
        }
        if tasks_changed {
            cx.emit(LibraryEvent::TasksChanged);
        }
        if events_changed {
            cx.emit(LibraryEvent::EventsChanged);
        }
        if habits_changed {
            cx.emit(LibraryEvent::HabitsChanged);
        }
        if focus_changed {
            cx.emit(LibraryEvent::FocusChanged);
        }
        if vault_changed {
            cx.emit(LibraryEvent::VaultChanged);
        }
        Ok(report)
    }
}
