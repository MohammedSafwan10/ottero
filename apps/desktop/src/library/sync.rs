//! Cloud sync state and cycle orchestration on [`Library`].

use anyhow::Result;
use gpui_kit::Context;

use ottero_core::clock;

use super::{Library, LibraryEvent};

impl Library {
    // ---- Sync ------------------------------------------------------------

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
        if state.needs_full_resync || (state.last_pull_seq == 0 && self.pending_sync_count() == 0) {
            let _ = crate::sync::enqueue_full_snapshot(&self.store, &device_id, now);
            state.last_pull_seq = 0;
        }
        let local_keys = self.store.sync_collection_keys().unwrap_or_default();
        let pending = self.store.pending_sync_changes(1_000).unwrap_or_default();
        let sync_url = std::env::var("OTTERO_SYNC_URL")
            .unwrap_or_else(|_| crate::sync::DEFAULT_SYNC_URL.to_owned());
        // Use a stable separator-safe account identifier derived from email if
        // server account_id isn't stored yet.
        let account_id = if state.account_id.is_empty() {
            email
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        } else {
            state.account_id.clone()
        };
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
                        if let Ok(mut st) = lib.store.sync_state() {
                            st.last_pull_seq = st.last_pull_seq.max(applied_seq);
                            if let Some(hlc) = outcome.last_hlc {
                                st.last_hlc = Some(hlc);
                            }
                            if apply_err.is_none() {
                                st.last_synced_at = Some(clock::now());
                                st.needs_full_resync = false;
                            }
                            let _ = lib.store.save_sync_state(&st);
                        }
                        lib.sync_last_error = apply_err;
                        if vault_changed && let Err(err) = lib.vault.reload_if_unlocked(&lib.store)
                        {
                            eprintln!("ottero: couldn't reload vault after sync: {err:#}");
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
                        lib.sync_last_error =
                            Some("An active Ottero Cloud plan is required to sync.".into());
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

    /// Enqueues a local mutation into `sync_outbox` when signed in to Ottero Cloud.
    /// Local-only users never accumulate outbox rows.
    pub(super) fn record_sync(
        &self,
        collection: ottero_sync_protocol::Collection,
        item_id: &str,
        fields: &[&str],
        deleted: bool,
        payload: &impl serde::Serialize,
    ) {
        if self.account.signed_in_email().is_none() {
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
        if vault_changed && let Err(err) = self.vault.reload_if_unlocked(&self.store) {
            eprintln!("ottero: couldn't reload vault after sync: {err:#}");
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
