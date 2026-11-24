//! Global actions, their default shortcuts, and the macOS menu bar.
//!
//! Every user-facing command is an action so it is reachable by keyboard,
//! menu, and command palette alike.

use gpui_kit::{Action, App, KeyBinding, Menu, MenuItem, actions};

actions!(
    ottero,
    [
        OpenCommandPalette,
        NewNote,
        NewTask,
        GoToday,
        GoNotes,
        GoTasks,
        GoCalendar,
        NewEvent,
        ShowDeletedEvents,
        ImportCalendar,
        GoHabits,
        NewHabit,
        GoFocus,
        GoDeveloper,
        NewDeveloperProject,
        ToggleArchivedDeveloperProjects,
        NewProjectSecrets,
        ImportProjectEnvironment,
        PasteProjectEnvironment,
        CopyProjectEnvironment,
        ExportProjectEnvironment,
        FocusToggle,
        FocusReset,
        FocusFinish,
        GoPasswords,
        LockVault,
        NewLogin,
        GeneratePassword,
        ChangeMasterPassword,
        UseAccountForPasswords,
        CreateRecoveryKey,
        ShowVaultArchives,
        ExportVaultBackup,
        RestoreVaultBackup,
        GoSettings,
        OpenAccountSettings,
        SyncNow,
        CreateAccount,
        SignInToCloud,
        SignOutOfCloud,
        CopySecretKey,
        SaveEmergencyKit,
        TurnOnTwoStep,
        TurnOffTwoStep,
        NewBackupCodes,
        CustomizeSidebar,
        CheckForUpdates,
        ShowChangelog,
        RestartToUpdate,
        ToggleSidebar,
        ToggleListPanel,
        ToggleMarkdownSource,
        FindInNote,
        FindNext,
        FindPrevious,
        OpenDailyNote,
        NewProject,
        ImportNotes,
        ExportNotes,
        UseLightTheme,
        UseDarkTheme,
        UseSystemTheme,
        Quit,
    ]
);

// Formatting in the note editor (bound only inside it, context "NoteEditor").
actions!(
    ottero_format,
    [
        FormatBold,
        FormatItalic,
        FormatStrikethrough,
        FormatCode,
        FormatHighlight,
        FormatLink,
        FormatHeading1,
        FormatHeading2,
        FormatHeading3,
        FormatParagraph,
        FormatBullets,
        FormatNumbers,
        FormatChecklist,
        FormatQuote,
        ToggleCheckbox,
    ]
);

// Calendar navigation (bound only on the calendar grid, context "CalendarGrid",
// so typing in quick add never triggers them).
actions!(
    ottero_calendar,
    [
        CalendarToday,
        CalendarPreviousPeriod,
        CalendarNextPeriod,
        CalendarPreviousDay,
        CalendarNextDay,
        CalendarPreviousWeek,
        CalendarNextWeek,
        CalendarMonthView,
        CalendarWeekView,
        CalendarDayView,
        CalendarAgendaView,
        CalendarOpenDay,
        CalendarQuickAdd,
    ]
);

// Habit list keys (context "HabitList", never in the quick-add field).
actions!(
    ottero_habits,
    [
        HabitSelectPrevious,
        HabitSelectNext,
        HabitToggleSelected,
        HabitOpenSelected,
        HabitQuickAdd,
    ]
);

// Vault list keys (context "VaultList", never in a text field).
actions!(
    ottero_vault,
    [
        VaultPreviousItem,
        VaultNextItem,
        VaultCopyPassword,
        VaultCopyUsername,
        VaultCopyCode,
        VaultEditSelected,
        VaultTrashSelected,
    ]
);

/// Key context set on the vault item list.
pub const VAULT_CONTEXT: &str = "VaultList";

/// Key context set on the habit list.
pub const HABITS_CONTEXT: &str = "HabitList";

/// Key context set on the focus screen.
pub const FOCUS_CONTEXT: &str = "FocusView";

/// Key context set on the note editor; format shortcuts only work there.
pub const NOTE_EDITOR_CONTEXT: &str = "NoteEditor";

/// Key context set on the calendar grid (not its quick-add field).
pub const CALENDAR_CONTEXT: &str = "CalendarGrid";

/// `cmd-<keys>` on macOS, `ctrl-<keys>` elsewhere, as a `&'static str`.
macro_rules! shortcut {
    ($keys:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $keys)
        } else {
            concat!("ctrl-", $keys)
        }
    };
}

/// Groups on the Keyboard settings page, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutGroup {
    General,
    Create,
    Navigate,
    View,
}

impl ShortcutGroup {
    pub const ALL: [Self; 4] = [Self::General, Self::Create, Self::Navigate, Self::View];

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Create => "Create",
            Self::Navigate => "Go to",
            Self::View => "View",
        }
    }
}

/// One app-wide shortcut. This table is the single source of truth: key
/// bindings, the Keyboard settings page and command-palette hints all read it.
pub struct Shortcut {
    /// Platform keystroke (`cmd-…` on macOS, `ctrl-…` elsewhere).
    pub keys: &'static str,
    pub label: &'static str,
    pub group: ShortcutGroup,
    action: fn() -> Box<dyn Action>,
}

impl Shortcut {
    #[must_use]
    pub fn action(&self) -> Box<dyn Action> {
        (self.action)()
    }
}

macro_rules! shortcuts {
    ($( $group:ident, $keys:literal, $label:literal, $action:expr; )*) => {
        &[ $( Shortcut {
            keys: shortcut!($keys),
            label: $label,
            group: ShortcutGroup::$group,
            action: || Box::new($action),
        }, )* ]
    };
}

pub const SHORTCUTS: &[Shortcut] = shortcuts![
    General, "k", "Search and commands", OpenCommandPalette;
    General, ",", "Settings", GoSettings;
    Create, "n", "New note", NewNote;
    Create, "shift-n", "New task", NewTask;
    Create, "shift-d", "Today's note", OpenDailyNote;
    Create, "shift-e", "New event", NewEvent;
    Navigate, "1", "Today", GoToday;
    Navigate, "2", "Notes", GoNotes;
    Navigate, "3", "Tasks", GoTasks;
    Navigate, "4", "Calendar", GoCalendar;
    Navigate, "5", "Habits", GoHabits;
    Navigate, "6", "Passwords", GoPasswords;
    Navigate, "7", "Focus", GoFocus;
    Navigate, "8", "Developer", GoDeveloper;
    General, "ctrl-l", "Lock vault", LockVault;
    View, "\\", "Toggle sidebar", ToggleSidebar;
    View, "shift-l", "Toggle note list", ToggleListPanel;
    View, "f", "Find in note", FindInNote;
    View, "g", "Next match", FindNext;
    View, "shift-g", "Previous match", FindPrevious;
];

/// Formatting shortcuts, active only while typing in a note (Bear/Notion keys).
pub const FORMAT_SHORTCUTS: &[Shortcut] = shortcuts![
    General, "b", "Bold", FormatBold;
    General, "i", "Italic", FormatItalic;
    General, "shift-x", "Strikethrough", FormatStrikethrough;
    General, "e", "Inline code", FormatCode;
    General, "shift-h", "Highlight", FormatHighlight;
    General, "shift-k", "Link", FormatLink;
    General, "alt-1", "Heading 1", FormatHeading1;
    General, "alt-2", "Heading 2", FormatHeading2;
    General, "alt-3", "Heading 3", FormatHeading3;
    General, "alt-0", "Body text", FormatParagraph;
    General, "shift-8", "Bulleted list", FormatBullets;
    General, "shift-7", "Numbered list", FormatNumbers;
    General, "shift-9", "Checklist", FormatChecklist;
    General, "enter", "Check / uncheck item", ToggleCheckbox;
];

/// Calendar keys, active while the calendar grid has focus (Apple Calendar /
/// Fantastical conventions). Plain keys, so they never reach a text field.
pub const CALENDAR_SHORTCUTS: &[Shortcut] = &[
    plain(shortcut!("left"), "Previous period", || {
        Box::new(CalendarPreviousPeriod)
    }),
    plain(shortcut!("right"), "Next period", || {
        Box::new(CalendarNextPeriod)
    }),
    plain("t", "Go to today", || Box::new(CalendarToday)),
    plain("left", "Previous day", || Box::new(CalendarPreviousDay)),
    plain("right", "Next day", || Box::new(CalendarNextDay)),
    plain("up", "Previous week", || Box::new(CalendarPreviousWeek)),
    plain("down", "Next week", || Box::new(CalendarNextWeek)),
    plain("enter", "Open selected day", || Box::new(CalendarOpenDay)),
    plain("m", "Month", || Box::new(CalendarMonthView)),
    plain("w", "Week", || Box::new(CalendarWeekView)),
    plain("d", "Day", || Box::new(CalendarDayView)),
    plain("a", "Agenda", || Box::new(CalendarAgendaView)),
    plain("n", "New event", || Box::new(CalendarQuickAdd)),
];

const fn plain(
    keys: &'static str,
    label: &'static str,
    action: fn() -> Box<dyn Action>,
) -> Shortcut {
    Shortcut {
        keys,
        label,
        group: ShortcutGroup::General,
        action,
    }
}

/// Habit list keys, active while the list has focus.
pub const HABIT_SHORTCUTS: &[Shortcut] = &[
    plain("up", "Previous habit", || Box::new(HabitSelectPrevious)),
    plain("down", "Next habit", || Box::new(HabitSelectNext)),
    plain("space", "Check in today", || Box::new(HabitToggleSelected)),
    plain("enter", "Open habit", || Box::new(HabitOpenSelected)),
    plain("n", "New habit", || Box::new(HabitQuickAdd)),
];

/// Vault list keys, active while the item list has focus (1Password keys).
pub const VAULT_SHORTCUTS: &[Shortcut] = &[
    plain("up", "Previous item", || Box::new(VaultPreviousItem)),
    plain("down", "Next item", || Box::new(VaultNextItem)),
    plain(shortcut!("c"), "Copy password or card number", || {
        Box::new(VaultCopyPassword)
    }),
    plain(shortcut!("shift-c"), "Copy username", || {
        Box::new(VaultCopyUsername)
    }),
    plain(shortcut!("alt-c"), "Copy one-time code", || {
        Box::new(VaultCopyCode)
    }),
    plain("enter", "Edit item", || Box::new(VaultEditSelected)),
    plain(shortcut!("backspace"), "Move to trash", || {
        Box::new(VaultTrashSelected)
    }),
];

pub const FOCUS_SHORTCUTS: &[Shortcut] = &[
    plain("space", "Start / pause", || Box::new(FocusToggle)),
    plain("r", "Reset timer", || Box::new(FocusReset)),
];

/// Built-in editing keys of the note editor (zorite-editor), shown for reference only.
pub const EDITING_SHORTCUTS: &[(&str, &str)] = &[
    (shortcut!("z"), "Undo"),
    (shortcut!("shift-z"), "Redo"),
    (shortcut!("a"), "Select all"),
];

/// The shortcut bound to `action`, for showing a hint next to it.
#[must_use]
pub fn shortcut_for(action: &dyn Action) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|s| s.action().name() == action.name())
        .map(|s| s.keys)
}

pub fn modifier_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

pub fn bind(cx: &mut App) {
    let mut bindings: Vec<KeyBinding> = SHORTCUTS
        .iter()
        .map(|s| KeyBinding::load(s.keys, s.action(), None, false, None, cx))
        .filter_map(Result::ok)
        .collect();
    let editor = gpui_kit::KeyBindingContextPredicate::parse(NOTE_EDITOR_CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    bindings.extend(FORMAT_SHORTCUTS.iter().filter_map(|s| {
        KeyBinding::load(s.keys, s.action(), editor.clone(), false, None, cx).ok()
    }));
    let calendar = gpui_kit::KeyBindingContextPredicate::parse(CALENDAR_CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    bindings.extend(CALENDAR_SHORTCUTS.iter().filter_map(|s| {
        KeyBinding::load(s.keys, s.action(), calendar.clone(), false, None, cx).ok()
    }));
    let habits = gpui_kit::KeyBindingContextPredicate::parse(HABITS_CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    bindings.extend(HABIT_SHORTCUTS.iter().filter_map(|s| {
        KeyBinding::load(s.keys, s.action(), habits.clone(), false, None, cx).ok()
    }));
    let vault = gpui_kit::KeyBindingContextPredicate::parse(VAULT_CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    bindings.extend(
        VAULT_SHORTCUTS.iter().filter_map(|s| {
            KeyBinding::load(s.keys, s.action(), vault.clone(), false, None, cx).ok()
        }),
    );
    let focus = gpui_kit::KeyBindingContextPredicate::parse(FOCUS_CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    bindings.extend(
        FOCUS_SHORTCUTS.iter().filter_map(|s| {
            KeyBinding::load(s.keys, s.action(), focus.clone(), false, None, cx).ok()
        }),
    );
    // Quitting by shortcut is a macOS convention; Windows and Linux close the window.
    if cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("cmd-q", Quit, None));
    }
    cx.bind_keys(bindings);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub fn set_menus(cx: &mut App) {
    cx.set_menus([
        Menu::new("Ottero").items([
            MenuItem::action("Settings…", GoSettings),
            MenuItem::action("Account…", OpenAccountSettings),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Quit Ottero", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Note", NewNote),
            MenuItem::action("New Task", NewTask),
            MenuItem::action("New Event", NewEvent),
            MenuItem::action("New Habit", NewHabit),
            MenuItem::action("New Login", NewLogin),
            MenuItem::action("New Project…", NewProject),
            MenuItem::action("Today's Note", OpenDailyNote),
            MenuItem::separator(),
            MenuItem::action("Import Markdown Folder…", ImportNotes),
            MenuItem::action("Export Notes as Markdown…", ExportNotes),
            MenuItem::action("Import Calendar (.ics)…", ImportCalendar),
            MenuItem::separator(),
            MenuItem::action("Export Vault Backup…", ExportVaultBackup),
            MenuItem::action("Restore Vault Backup…", RestoreVaultBackup),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette", OpenCommandPalette),
            MenuItem::separator(),
            MenuItem::action("Today", GoToday),
            MenuItem::action("Notes", GoNotes),
            MenuItem::action("Tasks", GoTasks),
            MenuItem::action("Calendar", GoCalendar),
            MenuItem::action("Habits", GoHabits),
            MenuItem::action("Focus", GoFocus),
            MenuItem::action("Passwords", GoPasswords),
            MenuItem::action("Lock Vault", LockVault),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Note List", ToggleListPanel),
        ]),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_parses_and_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for s in SHORTCUTS {
            assert!(
                gpui_kit::Keystroke::parse(s.keys).is_ok(),
                "{} does not parse",
                s.keys
            );
            assert!(seen.insert(s.keys), "{} is bound twice", s.keys);
            assert_eq!(shortcut_for(&*s.action()), Some(s.keys));
        }
        let mut format_keys = std::collections::HashSet::new();
        for s in FORMAT_SHORTCUTS {
            assert!(
                gpui_kit::Keystroke::parse(s.keys).is_ok(),
                "{} does not parse",
                s.keys
            );
            assert!(format_keys.insert(s.keys), "{} is bound twice", s.keys);
            assert!(
                !seen.contains(s.keys),
                "{} clashes with an app shortcut",
                s.keys
            );
        }
        for table in [
            CALENDAR_SHORTCUTS,
            HABIT_SHORTCUTS,
            VAULT_SHORTCUTS,
            FOCUS_SHORTCUTS,
        ] {
            let mut calendar_keys = std::collections::HashSet::new();
            for s in table {
                assert!(
                    gpui_kit::Keystroke::parse(s.keys).is_ok(),
                    "{} does not parse",
                    s.keys
                );
                assert!(calendar_keys.insert(s.keys), "{} is bound twice", s.keys);
                assert!(
                    !seen.contains(s.keys),
                    "{} clashes with an app shortcut",
                    s.keys
                );
            }
        }
        for (keys, _) in EDITING_SHORTCUTS {
            assert!(gpui_kit::Keystroke::parse(keys).is_ok());
            assert!(!seen.contains(keys), "{keys} clashes with an editor key");
        }
    }

    #[test]
    fn uses_the_platform_modifier() {
        let expected = if cfg!(target_os = "macos") {
            "cmd-"
        } else {
            "ctrl-"
        };
        assert!(SHORTCUTS.iter().all(|s| s.keys.starts_with(expected)));
    }
}
