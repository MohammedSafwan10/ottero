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
    pub today_events: Vec<CalendarEventItem>,
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
    pub location: String,
    pub date_iso: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HolidayItem {
    pub date_iso: String,
    pub name: String,
    pub short_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnifiedTrashSnapshot {
    pub notes: Vec<NoteListItem>,
    pub tasks: Vec<TaskItem>,
    pub events: Vec<CalendarEventItem>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MobileSignInStatus {
    SignedIn,
    NeedsTwoStep,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountSessionBundle {
    pub email: String,
    pub account_id: String,
    pub bearer_token: String,
    pub account_key_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MobileSignInResult {
    pub status: MobileSignInStatus,
    pub session: Option<AccountSessionBundle>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MobileAccountStatus {
    pub signed_in: bool,
    pub email: Option<String>,
    pub last_synced_at_ms: Option<i64>,
    pub pending_outbox_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MobileSyncReport {
    pub pushed: u32,
    pub pulled: u32,
    pub latest_seq: i64,
}

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncWsCredentials {
    pub ws_url: String,
    pub bearer_token: String,
    pub last_pull_seq: i64,
}
