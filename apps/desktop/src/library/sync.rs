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
