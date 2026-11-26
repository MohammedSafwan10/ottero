//! UniFFI bridge exposing Ottero's shared Rust core and SQLite storage
//! to native Android (Kotlin/Compose) and iOS (SwiftUI) apps.

mod calendar;
mod dto;
mod error;
mod habits;
mod notes;
mod sync;
mod tasks;
mod today;
mod vault;

use std::path::Path;
use std::sync::Mutex;

use ottero_storage::Store;
use sync::AccountSessionState;
use vault::VaultKeyState;

pub use dto::{
    AccountSessionBundle, AppInfo, CalendarEventItem, HabitItem, HolidayItem, MobileAccountStatus,
    MobileSignInResult, MobileSignInStatus, MobileSyncReport, MobileTaskFilter, MobileVaultStatus,
    NoteDetail, NoteListItem, SyncWsCredentials, TagCount, TaskItem, TodaySnapshot,
    TotpCodeSnapshot, UnifiedTrashSnapshot, VaultEntryDetail, VaultEntrySummary,
};
pub use error::MobileError;

uniffi::setup_scaffolding!();

/// Thread-safe handle to a local Ottero SQLite store, vault key, and cloud account session.
#[derive(uniffi::Object)]
pub struct OtteroMobile {
    store: Mutex<Store>,
    vault_key: Mutex<VaultKeyState>,
    account: Mutex<AccountSessionState>,
}

#[uniffi::export]
impl OtteroMobile {
    /// Opens (or creates and migrates) an Ottero SQLite database at `db_path`.
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<Self, MobileError> {
        let store = Store::open(Path::new(&db_path))?;
        Ok(Self {
            store: Mutex::new(store),
            vault_key: Mutex::new(VaultKeyState::new()),
            account: Mutex::new(AccountSessionState::new()),
        })
    }

    /// In-memory database for previews and instrumentation tests.
    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Self, MobileError> {
        let store = Store::open_in_memory()?;
        Ok(Self {
            store: Mutex::new(store),
            vault_key: Mutex::new(VaultKeyState::new()),
            account: Mutex::new(AccountSessionState::new()),
        })
    }

    pub fn app_info(&self) -> Result<AppInfo, MobileError> {
        let store = self.lock()?;
        Ok(AppInfo {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: store.schema_version()?,
        })
    }

    pub fn today_snapshot(&self) -> Result<TodaySnapshot, MobileError> {
        let store = self.lock()?;
        today::snapshot(&store)
    }

    pub fn list_notes(&self, query: Option<String>) -> Result<Vec<NoteListItem>, MobileError> {
        let store = self.lock()?;
        notes::list_notes(&store, query)
    }

    pub fn get_note(&self, id: String) -> Result<NoteDetail, MobileError> {
        let store = self.lock()?;
        notes::get_note(&store, &id)
    }

    pub fn create_note(&self, title: String, body: String) -> Result<NoteDetail, MobileError> {
        let mut store = self.lock()?;
        notes::create_note(&mut store, &title, &body)
    }

    pub fn update_note(
        &self,
        id: String,
        title: String,
        body: String,
    ) -> Result<NoteDetail, MobileError> {
        let mut store = self.lock()?;
        notes::update_note(&mut store, &id, &title, &body)
    }

    pub fn set_note_pinned(&self, id: String, pinned: bool) -> Result<NoteDetail, MobileError> {
        let mut store = self.lock()?;
        notes::set_note_pinned(&mut store, &id, pinned)
    }

    pub fn trash_note(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        notes::trash_note(&mut store, &id)
    }

    pub fn restore_note(&self, id: String) -> Result<NoteDetail, MobileError> {
        let mut store = self.lock()?;
        notes::restore_note(&mut store, &id)
    }

    pub fn list_trashed_notes(&self) -> Result<Vec<NoteListItem>, MobileError> {
        let store = self.lock()?;
        notes::list_trashed_notes(&store)
    }

    pub fn delete_note_permanently(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        notes::delete_note_permanently(&mut store, &id)
    }

    pub fn list_tags(&self) -> Result<Vec<TagCount>, MobileError> {
        let store = self.lock()?;
        notes::list_tags(&store)
    }

    pub fn list_notes_by_tag(&self, tag: String) -> Result<Vec<NoteListItem>, MobileError> {
        let store = self.lock()?;
        notes::list_notes_by_tag(&store, &tag)
    }

    pub fn list_tasks(&self, filter: MobileTaskFilter) -> Result<Vec<TaskItem>, MobileError> {
        let store = self.lock()?;
        tasks::list_tasks(&store, filter)
    }

    pub fn quick_add_task(
        &self,
        input: String,
        default_today: bool,
    ) -> Result<TaskItem, MobileError> {
        let mut store = self.lock()?;
        tasks::quick_add_task(&mut store, &input, default_today)
    }

    pub fn update_task(
        &self,
        id: String,
        title: String,
        notes: String,
        priority: u8,
        due_date: Option<String>,
    ) -> Result<TaskItem, MobileError> {
        let mut store = self.lock()?;
        tasks::update_task(&mut store, &id, &title, &notes, priority, due_date)
    }

    pub fn toggle_task_done(&self, id: String) -> Result<TaskItem, MobileError> {
        let mut store = self.lock()?;
        tasks::toggle_task_done(&mut store, &id)
    }

    pub fn trash_task(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        tasks::trash_task(&mut store, &id)
    }

    pub fn restore_task(&self, id: String) -> Result<TaskItem, MobileError> {
        let mut store = self.lock()?;
        tasks::restore_task(&mut store, &id)
    }

    pub fn list_habits(&self) -> Result<Vec<HabitItem>, MobileError> {
        let store = self.lock()?;
        habits::list_habits(&store)
    }

    pub fn quick_add_habit(&self, input: String) -> Result<HabitItem, MobileError> {
        let mut store = self.lock()?;
        habits::quick_add_habit(&mut store, &input)
    }

    pub fn tap_habit_today(&self, id: String) -> Result<HabitItem, MobileError> {
        let mut store = self.lock()?;
        habits::tap_habit_today(&mut store, &id)
    }

    pub fn list_events_in_range(
        &self,
        start_iso: String,
        end_iso: String,
    ) -> Result<Vec<CalendarEventItem>, MobileError> {
        let store = self.lock()?;
        calendar::list_events_in_range(&store, &start_iso, &end_iso)
    }

    pub fn quick_add_event(
        &self,
        input: String,
        default_date_iso: Option<String>,
    ) -> Result<CalendarEventItem, MobileError> {
        let mut store = self.lock()?;
        calendar::quick_add_event(&mut store, &input, default_date_iso)
    }

    pub fn trash_event(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        calendar::trash_event(&mut store, &id)
    }

    pub fn restore_event(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        calendar::restore_event(&mut store, &id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_event(
        &self,
        id: String,
        title: String,
        notes: String,
        location: String,
        date_iso: String,
        all_day: bool,
        start_time: Option<String>,
        end_time: Option<String>,
    ) -> Result<CalendarEventItem, MobileError> {
        let mut store = self.lock()?;
        calendar::update_event(
            &mut store, &id, &title, &notes, &location, &date_iso, all_day, start_time, end_time,
        )
    }

    pub fn list_holidays_in_range(
        &self,
        start_iso: String,
        end_iso: String,
        country_code: Option<String>,
    ) -> Result<Vec<HolidayItem>, MobileError> {
        calendar::list_holidays_in_range(&start_iso, &end_iso, country_code)
    }

    pub fn unified_trash_snapshot(&self) -> Result<UnifiedTrashSnapshot, MobileError> {
        let store = self.lock()?;
        Ok(UnifiedTrashSnapshot {
            notes: notes::list_trashed_notes(&store)?,
            tasks: tasks::list_trashed_tasks(&store)?,
            events: calendar::list_trashed_events(&store)?,
        })
    }

    pub fn delete_task_permanently(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        tasks::delete_task_permanently(&mut store, &id)
    }

    pub fn delete_event_permanently(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        calendar::delete_event_permanently(&mut store, &id)
    }

    pub fn empty_all_trash(&self) -> Result<u32, MobileError> {
        let mut store = self.lock()?;
        let trashed_notes = notes::list_trashed_notes(&store)?;
        let trashed_tasks = tasks::list_trashed_tasks(&store)?;
        let trashed_events = calendar::list_trashed_events(&store)?;
        let mut count: u32 = 0;
        for n in trashed_notes {
            notes::delete_note_permanently(&mut store, &n.id)?;
            count = count.saturating_add(1);
        }
        for t in trashed_tasks {
            tasks::delete_task_permanently(&mut store, &t.id)?;
            count = count.saturating_add(1);
        }
        for e in trashed_events {
            calendar::delete_event_permanently(&mut store, &e.id)?;
            count = count.saturating_add(1);
        }
        Ok(count)
    }

    pub fn vault_status(&self) -> Result<MobileVaultStatus, MobileError> {
        let store = self.lock()?;
        let key = self.lock_vault()?;
        vault::status(&store, &key)
    }

    pub fn vault_setup(&self, master_password: String) -> Result<(), MobileError> {
        vault::setup(&self.store, &self.vault_key, master_password)
    }

    pub fn vault_unlock(&self, master_password: String) -> Result<(), MobileError> {
        vault::unlock(&self.store, &self.vault_key, master_password)
    }

    pub fn vault_lock(&self) -> Result<(), MobileError> {
        let mut key = self.lock_vault()?;
        vault::lock(&mut key);
        Ok(())
    }

    pub fn vault_list_items(
        &self,
        query: Option<String>,
    ) -> Result<Vec<VaultEntrySummary>, MobileError> {
        let store = self.lock()?;
        let key = self.lock_vault()?;
        vault::list_items(&store, &key, query)
    }

    pub fn vault_get_item(&self, id: String) -> Result<VaultEntryDetail, MobileError> {
        let store = self.lock()?;
        let key = self.lock_vault()?;
        vault::get_item(&store, &key, &id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn vault_save_login(
        &self,
        id: Option<String>,
        title: String,
        username: String,
        password: String,
        url: String,
        totp_secret: String,
        notes: String,
    ) -> Result<VaultEntryDetail, MobileError> {
        let mut store = self.lock()?;
        let key = self.lock_vault()?;
        vault::save_login(
            &mut store,
            &key,
            id,
            title,
            username,
            password,
            url,
            totp_secret,
            notes,
        )
    }

    pub fn vault_trash_item(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        let key = self.lock_vault()?;
        vault::trash_item(&mut store, &key, &id)
    }

    pub fn vault_restore_item(&self, id: String) -> Result<(), MobileError> {
        let mut store = self.lock()?;
        let key = self.lock_vault()?;
        vault::restore_item(&mut store, &key, &id)
    }

    pub fn vault_generate_password(&self, length: u8) -> Result<String, MobileError> {
        vault::generate_password(length)
    }

    pub fn vault_totp_now(&self, secret: String) -> Result<TotpCodeSnapshot, MobileError> {
        vault::totp_now(&secret)
    }

    pub fn account_status(&self) -> Result<MobileAccountStatus, MobileError> {
        let store = self.lock()?;
        let acct = self.lock_account()?;
        sync::account_status(&store, &acct)
    }

    pub fn account_sign_in(
        &self,
        email: String,
        password: String,
        secret_key: String,
    ) -> Result<MobileSignInResult, MobileError> {
        sync::sign_in(
            &self.store,
            &self.vault_key,
            &self.account,
            email,
            password,
            secret_key,
        )
    }

    pub fn account_verify_two_step(
        &self,
        code: String,
        is_backup_code: bool,
    ) -> Result<MobileSignInResult, MobileError> {
        sync::verify_two_step(
            &self.store,
            &self.vault_key,
            &self.account,
            code,
            is_backup_code,
        )
    }

    pub fn account_restore_session(
        &self,
        email: String,
        account_id: String,
        bearer_token: String,
        account_key_b64: String,
    ) -> Result<(), MobileError> {
        sync::restore_session(
            &self.store,
            &self.vault_key,
            &self.account,
            email,
            account_id,
            bearer_token,
            account_key_b64,
        )
    }

    pub fn account_sign_out(&self) -> Result<(), MobileError> {
        sync::sign_out(&self.vault_key, &self.account)
    }

    pub fn sync_now(&self) -> Result<MobileSyncReport, MobileError> {
        sync::sync_now(&self.store, &self.vault_key, &self.account)
    }

    pub fn vault_unlock_with_account(&self) -> Result<bool, MobileError> {
        sync::unlock_vault_with_account(&self.store, &self.vault_key, &self.account)
    }

    pub fn sync_ws_credentials(&self) -> Result<Option<SyncWsCredentials>, MobileError> {
        let store = self.lock()?;
        let acct = self.lock_account()?;
        sync::ws_credentials(&store, &acct)
    }
}

impl OtteroMobile {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, MobileError> {
        self.store.lock().map_err(|_| MobileError::LockPoisoned)
    }

    fn lock_vault(&self) -> Result<std::sync::MutexGuard<'_, VaultKeyState>, MobileError> {
        self.vault_key.lock().map_err(|_| MobileError::LockPoisoned)
    }

    fn lock_account(&self) -> Result<std::sync::MutexGuard<'_, AccountSessionState>, MobileError> {
        self.account.lock().map_err(|_| MobileError::LockPoisoned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_ffi_end_to_end_notes_tasks_habits_today() {
        let app = OtteroMobile::open_in_memory().expect("open");
        let info = app.app_info().expect("info");
        assert!(info.schema_version >= 14);

        let note = app
            .create_note("Android Plan".into(), "Native Kotlin + UniFFI".into())
            .expect("create note");
        let updated = app
            .update_note(
                note.id.clone(),
                "Android Plan".into(),
                "Native Kotlin + UniFFI + 120Hz".into(),
            )
            .expect("update");
        assert!(updated.body.contains("120Hz"));
        assert_eq!(
            app.list_notes(Some("120Hz".into())).expect("search").len(),
            1
        );

        let task = app
            .quick_add_task("Ship Android MVP !high".into(), true)
            .expect("task");
        assert_eq!(task.priority, 3);
        let toggled = app.toggle_task_done(task.id.clone()).expect("toggle");
        assert!(toggled.completed);

        let habit = app.quick_add_habit("Read 20 pages".into()).expect("habit");
        let tapped = app.tap_habit_today(habit.id.clone()).expect("tap");
        assert!(tapped.done_today);

        let snap = app.today_snapshot().expect("today");
        assert_eq!(snap.completed_today_count, 1);
        assert_eq!(snap.habits.len(), 1);
        assert_eq!(snap.recent_notes.len(), 1);

        // Trash + restore note
        app.trash_note(note.id.clone()).expect("trash note");
        assert!(app.list_notes(None).expect("list").is_empty());
        let restored_note = app.restore_note(note.id.clone()).expect("restore note");
        assert!(!restored_note.trashed);
        assert_eq!(app.list_notes(None).expect("list").len(), 1);

        // Trash + restore task
        app.trash_task(task.id.clone()).expect("trash task");
        let restored_task = app.restore_task(task.id.clone()).expect("restore task");
        assert_eq!(restored_task.id, task.id);

        // Update task details
        let edited_task = app
            .update_task(
                task.id.clone(),
                "Ship Android Phase 4".into(),
                "Calendar + Focus + Markdown".into(),
                2,
                Some(snap.date_iso.clone()),
            )
            .expect("update task");
        assert_eq!(edited_task.title, "Ship Android Phase 4");
        assert_eq!(edited_task.priority, 2);

        // Tags + permanent delete
        let tagged = app
            .create_note("Tagged".into(), "Working on #android".into())
            .expect("tagged note");
        assert!(!app.list_tags().expect("tags").is_empty());
        assert_eq!(
            app.list_notes_by_tag("android".into())
                .expect("by tag")
                .len(),
            1
        );
        app.trash_note(tagged.id.clone()).expect("trash tagged");
        assert_eq!(app.list_trashed_notes().expect("trashed").len(), 1);
        app.delete_note_permanently(tagged.id)
            .expect("permanent delete");
        assert!(app.list_trashed_notes().expect("trashed").is_empty());

        // Calendar events + trash/restore
        let ev = app
            .quick_add_event("Standup 10am".into(), Some(snap.date_iso.clone()))
            .expect("event");
        assert_eq!(ev.title, "Standup");
        let events = app
            .list_events_in_range(snap.date_iso.clone(), snap.date_iso.clone())
            .expect("list events");
        assert_eq!(events.len(), 1);
        app.trash_event(ev.id.clone()).expect("trash event");
        assert!(
            app.list_events_in_range(snap.date_iso.clone(), snap.date_iso.clone())
                .expect("list events")
                .is_empty()
        );
        app.restore_event(ev.id).expect("restore event");
        assert_eq!(
            app.list_events_in_range(snap.date_iso.clone(), snap.date_iso)
                .expect("list events")
                .len(),
            1
        );

        // Passwords Vault end-to-end (setup, save, lock, wrong pw rejected, unlock, TOTP, trash/restore)
        assert_eq!(
            app.vault_status().expect("status"),
            MobileVaultStatus::NotSetUp
        );
        app.vault_setup("correct horse battery staple".into())
            .expect("setup");
        assert_eq!(
            app.vault_status().expect("status"),
            MobileVaultStatus::Unlocked
        );
        let generated = app.vault_generate_password(20).expect("generate");
        assert_eq!(generated.len(), 20);
        let saved = app
            .vault_save_login(
                None,
                "GitHub".into(),
                "safwan".into(),
                generated.clone(),
                "https://github.com".into(),
                "JBSWY3DPEHPK3PXP".into(),
                "".into(),
            )
            .expect("save login");
        assert_eq!(saved.title, "GitHub");
        assert_eq!(
            app.vault_list_items(Some("git".into()))
                .expect("list")
                .len(),
            1
        );
        let totp = app.vault_totp_now("JBSWY3DPEHPK3PXP".into()).expect("totp");
        assert_eq!(totp.code.len(), 6);

        app.vault_lock().expect("lock");
        assert_eq!(
            app.vault_status().expect("status"),
            MobileVaultStatus::Locked
        );
        assert!(app.vault_unlock("wrong-password".into()).is_err());
        app.vault_unlock("correct horse battery staple".into())
            .expect("unlock");
        let reopened = app.vault_get_item(saved.id.clone()).expect("get");
        assert_eq!(reopened.password, generated);
        app.vault_trash_item(saved.id.clone()).expect("trash");
        assert!(app.vault_list_items(None).expect("list").is_empty());
        app.vault_restore_item(saved.id).expect("restore");
        assert_eq!(app.vault_list_items(None).expect("list").len(), 1);
    }

    #[test]
    fn sync_account_id_matches_desktop_derivation() {
        assert_eq!(
            sync::derive_sync_account_id("Alice.Test+1@Example.COM "),
            "alice.test_1_example.com"
        );
    }
}
