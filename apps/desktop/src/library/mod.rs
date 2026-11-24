//! The app-wide data model: one `Entity<Library>` wraps the [`Store`].
//!
//! Views never touch SQLite directly. They call methods here, which apply
//! domain rules, persist, and emit a [`LibraryEvent`] so every interested
//! view can refresh. Writes are synchronous: each is a single small SQLite
//! transaction (well under a millisecond in WAL mode). Move to a background
//! writer if profiling ever shows frame drops.

mod calendar;
mod developer;
mod focus;
mod habits;
mod notes;
mod sync;
mod tasks;
mod vault;

pub use calendar::{CalendarSpan, IcsReport};
pub use habits::HabitSummary;
pub use notes::daily_note_title;

use std::time::SystemTime;

use gpui_kit::EventEmitter;
use ottero_crypto::{KdfParams, SecretString, Zeroizing};
use ottero_storage::Store;

use crate::vault::{AccountUnlock, VaultSession};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryEvent {
    NotesChanged,
    TasksChanged,
    EventsChanged,
    HabitsChanged,
    FocusChanged,
    DeveloperChanged,
    /// Locked, unlocked, or an item changed.
    VaultChanged,
    /// Ottero Cloud sign-in state, plan, devices or a flow step changed.
    AccountChanged,
}

pub struct Library {
    store: Store,
    vault: VaultSession,
    /// Argon2 cost for new vaults and password changes (tests lower it).
    kdf: KdfParams,
    /// Last time the user did anything while the vault was unlocked. Wall
    /// clock on purpose: `Instant` stops while macOS sleeps, so time asleep
    /// would not count as idle.
    vault_activity: SystemTime,
    /// Where "Start a new vault" keeps the old encrypted vault.
    archive_dir: std::path::PathBuf,
    /// Ottero Cloud sign-in (`crate::account`).
    pub(crate) account: crate::account::AccountSession,
    /// Why the open account couldn't open a linked vault (shown on the
    /// locked screen); cleared when the account or the vault changes.
    vault_account_problem: Option<AccountUnlock>,
    /// True while a background cloud sync cycle is running.
    sync_in_flight: bool,
    /// Last user-facing sync error, cleared on the next sync attempt.
    sync_last_error: Option<String>,
    /// True in the real desktop app (off in headless unit/UI tests unless enabled).
    auto_sync_enabled: bool,
    /// Set when a local edit happens while a sync cycle is already in-flight.
    sync_needed_again: bool,
    /// Monotonic counter used to coalesce rapid local edits into one debounced sync.
    sync_debounce_gen: u64,
    /// Active WebSocket change-notice listener handle, if connected.
    sync_ws_handle: Option<crate::sync::ws::SyncWsHandle>,
    /// Account epoch that owns `sync_ws_handle`.
    sync_ws_epoch: Option<u64>,
}

/// What opens an archive or backup the user picked.
pub enum ArchiveSecret {
    /// Its master password (a version-1 file).
    Password(SecretString),
    /// Its recovery key (a version-2 file, linked to an account).
    RecoveryKey(Zeroizing<String>),
    /// The Ottero Cloud account key open now (a version-2 file).
    Account,
}

impl EventEmitter<LibraryEvent> for Library {}

impl Library {
    pub fn backup_for_update(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.store.backup_for_update(path)?;
        Ok(())
    }
    #[must_use]
    pub fn new(store: Store) -> Self {
        let vault = VaultSession::load(&store).unwrap_or_else(|err| {
            eprintln!("ottero: couldn't read the vault header: {err:#}");
            // Locked, never "set up": setup must not overwrite a header it couldn't read.
            VaultSession::locked()
        });
        Self {
            store,
            vault,
            kdf: KdfParams::RECOMMENDED,
            vault_activity: SystemTime::now(),
            archive_dir: std::env::temp_dir().join("ottero-vault-archives"),
            account: crate::account::AccountSession::detached(),
            vault_account_problem: None,
            sync_in_flight: false,
            sync_last_error: None,
            auto_sync_enabled: false,
            sync_needed_again: false,
            sync_debounce_gen: 0,
            sync_ws_handle: None,
            sync_ws_epoch: None,
        }
    }
}
