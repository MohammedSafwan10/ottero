//! UniFFI bridge exposing Ottero's shared Rust core and SQLite storage
//! to native Android (Kotlin/Compose) and iOS (SwiftUI) apps.

mod dto;
mod error;
mod habits;
mod notes;
mod tasks;
mod today;

use std::path::Path;
use std::sync::Mutex;

use ottero_storage::Store;

pub use dto::{
    AppInfo, HabitItem, MobileTaskFilter, NoteDetail, NoteListItem, TaskItem, TodaySnapshot,
};
pub use error::MobileError;

uniffi::setup_scaffolding!();

/// Thread-safe handle to a local Ottero SQLite store.
#[derive(uniffi::Object)]
pub struct OtteroMobile {
    store: Mutex<Store>,
}

#[uniffi::export]
impl OtteroMobile {
    /// Opens (or creates and migrates) an Ottero SQLite database at `db_path`.
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<Self, MobileError> {
        let store = Store::open(Path::new(&db_path))?;
        Ok(Self {
            store: Mutex::new(store),
        })
    }

    /// In-memory database for previews and instrumentation tests.
    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Self, MobileError> {
        let store = Store::open_in_memory()?;
        Ok(Self {
            store: Mutex::new(store),
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
}

impl OtteroMobile {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, MobileError> {
        self.store.lock().map_err(|_| MobileError::LockPoisoned)
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
    }
}
