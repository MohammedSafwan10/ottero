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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TagCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarEventItem {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub date_iso: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MobileVaultStatus {
    NotSetUp,
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultEntrySummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub has_totp: bool,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultEntryDetail {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub totp_secret: String,
    pub notes: String,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TotpCodeSnapshot {
    pub code: String,
    pub grouped: String,
    pub seconds_left: u32,
    pub period: u32,
}
