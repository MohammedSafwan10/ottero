//! Flat, immutable UniFFI records exposed to Kotlin and Swift.

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppInfo {
    pub version: String,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteListItem {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub pinned: bool,
    pub trashed: bool,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteDetail {
    pub id: String,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub trashed: bool,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MobileTaskFilter {
    Today,
    Upcoming,
    All,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub completed: bool,
    pub priority: u8,
    pub due_date: Option<String>,
    pub overdue: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitItem {
    pub id: String,
    pub name: String,
    pub target_per_day: u16,
    pub today_count: u16,
    pub due_today: bool,
    pub done_today: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TodaySnapshot {
    pub date_iso: String,
    pub due_tasks: Vec<TaskItem>,
    pub habits: Vec<HabitItem>,
    pub recent_notes: Vec<NoteListItem>,
    pub completed_today_count: u32,
    pub overdue_count: u32,
}
