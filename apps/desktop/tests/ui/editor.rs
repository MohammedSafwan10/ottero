use super::*;
use crate::core_ui::*;
use crate::notes_tasks::*;

// ---- Live-preview editor (zorite-editor, ADR 0004) ----

fn note_editor(
    app: &App,
    cx: &mut TestAppContext,
) -> Entity<ottero_desktop::views::note_editor::NoteEditor> {
    cx.read(|cx| app.shell.read(cx).notes().read(cx).note_editor().clone())
}

fn open_note_with(
    app: &App,
    cx: &mut TestAppContext,
    title: &str,
    body: &str,
) -> ottero_core::NoteId {
    let lib = library(app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_note(title, cx).expect("create");
            l.edit_note(id, title, body, cx).expect("edit");
            id
        })
    });
    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| s.open_note(id, window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    cx.run_until_parked();
    id
}

#[gpui_kit::test]
fn opening_and_saving_never_changes_a_notes_markdown(cx: &mut TestAppContext) {
    let app = launch(cx);
    // Things block editors are known to rewrite: blank lines, loose lists,
    // trailing spaces, tables, HTML, math, CRLF, no final newline.
    let samples = [
        "",
        "Plain line",
        "# Title\n\n\n\nThree blank lines above.\n",
        "- a\n\n- loose list\n  - nested\n\n1) other\n2) numbering\n",
        "Hard break with two spaces  \nnext line\t\ttabs",
        "| A | B |\n|:--|--:|\n| 1 | 2 |\n\n<!-- table:striped -->\n",
        "**bold** _it_ ~~s~~ ==hi== `code` <u>u</u> <span style=\"color:#f00\">red</span>",
        "```rust\nfn main() {}\n```\n~~~\nraw\n~~~",
        "> [!NOTE]\n> callout\n\n---\n\n[^1]: foot",
        "$$\nx^2 + y^2\n$$\n\nInline $a+b$ math.",
        "Windows\r\nline endings\r\n",
        "Emoji 😀, accents é, RTL שלום, [[Link|alias]] #tag/nested",
    ];
    let editor = note_editor(&app, cx);
    for sample in samples {
        cx.update(|cx| editor.update(cx, |e, cx| e.set_note(None, sample, cx)));
        cx.update_window(app.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        let back = cx.read(|cx| editor.read(cx).text(cx));
        assert_eq!(back, sample, "loaded text must come back byte-identical");
    }

    // And through the real open → type → save path.
    let id = open_note_with(&app, cx, "Round trip", samples[3]);
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.focus_body(window, cx));
    })
    .expect("window");
    press(&app, cx, &secondary("end"));
    press(&app, cx, "ctrl-end");
    type_text(&app, cx, "x");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let saved = cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .note(id)
            .expect("load")
            .expect("note")
            .body()
            .to_owned()
    });
    assert_eq!(saved, format!("{}x", samples[3]));
}

#[gpui_kit::test]
fn selecting_text_shows_a_format_bar_that_keeps_focus(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    open_note_with(&app, cx, "Bar", "make this bold");
    let editor = note_editor(&app, cx);
    cx.update_window(app.window, |_, window, cx| {
        editor.update(cx, |e, cx| e.focus(window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    let bar_open = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).is_selection_bar_open());
    assert!(!bar_open(cx), "no bar without a selection");

    press(&app, cx, &secondary("a"));
    assert!(bar_open(cx), "selecting shows the bar");

    click(&app, cx, "fmt-bold");
    let body = cx.read(|cx| editor.read(cx).text(cx));
    assert_eq!(body, "**make this bold**");
    let focused = cx
        .update_window(app.window, |_, window, cx| {
            editor.read(cx).is_focused(window, cx)
        })
        .expect("window");
    assert!(focused, "the bar never takes focus from the text");

    // The change is saved like typing.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(all_note_titles(&app, cx).contains(&"Bar".to_owned()));

    press(&app, cx, "escape");
    assert!(!bar_open(cx), "Escape hides the bar");
    press(&app, cx, &secondary("a"));
    assert!(bar_open(cx));
    type_text(&app, cx, "new");
    assert!(!bar_open(cx), "typing over the selection hides it");
}

#[gpui_kit::test]
fn markdown_source_mode_toggles_without_changing_text(cx: &mut TestAppContext) {
    use ottero_desktop::actions::ToggleMarkdownSource;
    let app = launch(cx);
    open_note_with(&app, cx, "Source", "# Heading\n**bold**");
    let notes = cx.read(|cx| app.shell.read(cx).notes().clone());
    let source = |cx: &mut TestAppContext| cx.read(|cx| notes.read(cx).is_source_mode(cx));
    assert!(!source(cx), "live preview is the default");
    dispatch(&app, cx, ToggleMarkdownSource);
    assert!(source(cx));
    assert_eq!(
        cx.read(|cx| notes.read(cx).body_text(cx)),
        "# Heading\n**bold**"
    );
    dispatch(&app, cx, ToggleMarkdownSource);
    assert!(!source(cx));
}

#[gpui_kit::test]
fn clicking_links_and_tags_in_the_editor_navigates(cx: &mut TestAppContext) {
    let app = launch(cx);
    let car = open_note_with(&app, cx, "Car", "Insurance");
    open_note_with(&app, cx, "Trip", "Drive the [[Car]] to #coast");
    let editor = note_editor(&app, cx);
    let engine = cx.read(|cx| editor.read(cx).engine().clone());
    let notes = cx.read(|cx| app.shell.read(cx).notes().clone());

    cx.update(|cx| {
        engine.update(cx, |_, cx| {
            cx.emit(zorite_editor::EditorEvent::OpenWikiLink("coast".into()))
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| notes.read(cx).tag_filter().map(str::to_owned)),
        Some("coast".to_owned()),
        "a #tag filters the list"
    );

    cx.update(|cx| {
        engine.update(cx, |_, cx| {
            cx.emit(zorite_editor::EditorEvent::OpenWikiLink("Car".into()))
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| notes.read(cx).open_note_id()),
        Some(car),
        "a [[link]] opens the note"
    );
}

#[gpui_kit::test]
fn arrow_keys_move_through_math_blocks(cx: &mut TestAppContext) {
    let app = launch(cx);
    open_note_with(&app, cx, "Math", "above\n$$\nx^2\n$$\nbelow");
    let editor = note_editor(&app, cx);
    let engine = cx.read(|cx| editor.read(cx).engine().clone());
    cx.update_window(app.window, |_, window, cx| {
        editor.update(cx, |e, cx| e.focus(window, cx));
        window.render_frame(cx);
    })
    .expect("window");
    let cursor = |cx: &mut TestAppContext| cx.read(|cx| engine.read(cx).cursor());
    assert_eq!(cursor(cx), 0);
    press(&app, cx, "down");
    let inside = cursor(cx);
    assert!(
        (6..=15).contains(&inside),
        "the caret enters the formula instead of stopping: {inside}"
    );
    for _ in 0..4 {
        press(&app, cx, "down");
    }
    assert!(
        cursor(cx) >= "above\n$$\nx^2\n$$\n".len(),
        "and leaves it again"
    );
}

#[gpui_kit::test]
fn find_in_note_counts_steps_wraps_and_closes(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{FindInNote, FindNext, FindPrevious};
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, GoNotes);
    open_note_with(
        &app,
        cx,
        "Groceries",
        "Milk, eggs.\nMore milk later.\nNo MILK at all",
    );
    let editor = note_editor(&app, cx);
    let status = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).find_status());

    assert_eq!(status(cx), None, "closed until asked for");
    dispatch(&app, cx, FindInNote);
    assert_eq!(status(cx), Some((0, 0)), "open with an empty query");
    type_text(&app, cx, "milk");
    assert_eq!(
        status(cx),
        Some((1, 3)),
        "case-insensitive, active on the first"
    );

    dispatch(&app, cx, FindNext);
    assert_eq!(status(cx), Some((2, 3)));
    press(&app, cx, "enter");
    assert_eq!(
        status(cx),
        Some((3, 3)),
        "Enter in the field goes to the next"
    );
    dispatch(&app, cx, FindNext);
    assert_eq!(status(cx), Some((1, 3)), "wraps to the first");
    dispatch(&app, cx, FindPrevious);
    assert_eq!(status(cx), Some((3, 3)), "and back to the last");
    press(&app, cx, "shift-enter");
    assert_eq!(status(cx), Some((2, 3)));

    // Edits keep the count current.
    cx.update(|cx| {
        let engine = editor.read(cx).engine().clone();
        engine.update(cx, |e, cx| e.replace_range(0..4, "Oat", cx));
        engine.update(cx, |_, cx| cx.emit(zorite_editor::EditorEvent::Changed));
    });
    cx.run_until_parked();
    assert_eq!(status(cx).map(|s| s.1), Some(2), "matches follow the text");

    press(&app, cx, "escape");
    assert_eq!(status(cx), None, "Escape closes the bar");
    let focused = cx
        .update_window(app.window, |_, window, cx| {
            editor.read(cx).is_focused(window, cx)
        })
        .expect("window");
    assert!(focused, "and puts you back in the text");

    // ⌘F with a selection searches for it.
    press(&app, cx, &secondary("a"));
    cx.update(|cx| {
        let engine = editor.read(cx).engine().clone();
        engine.update(cx, |e, cx| e.set_cursor(0, cx));
    });
    cx.run_until_parked();
    dispatch(&app, cx, FindInNote);
    type_text(&app, cx, "zzz");
    assert_eq!(status(cx), Some((0, 0)), "no match shows zero");
}

/// The OS checker exists on macOS and Windows; elsewhere spelling is a no-op.
#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn spelling_flags_prose_skips_code_and_can_be_turned_off(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, GoNotes);
    let body = "I recieved the packige.\n`teh code` and [[Grocerys]] #teh";
    open_note_with(&app, cx, "Spelling", body);
    let editor = note_editor(&app, cx);
    let flagged = |cx: &mut TestAppContext| cx.read(|cx| editor.read(cx).flagged_words(cx));
    let words = flagged(cx);
    assert!(words.contains(&"recieved".to_owned()), "{words:?}");
    assert!(words.contains(&"packige".to_owned()), "{words:?}");
    assert!(
        !words
            .iter()
            .any(|w| ["teh", "Grocerys"].contains(&w.as_str())),
        "code, links and tags are not prose: {words:?}"
    );

    cx.update(|cx| ottero_ui::appearance::update(cx, |s| s.spell_check = false));
    cx.run_until_parked();
    assert!(flagged(cx).is_empty(), "the setting turns underlines off");
}

#[gpui_kit::test]
fn picking_a_theme_card_applies_and_saves_it(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    dispatch(&app, cx, UseDarkTheme);
    dispatch(&app, cx, GoSettings);
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.within("settings-sidebar").click("0-3", cx);
    })
    .expect("Appearance navigation");
    cx.run_until_parked();
    let name = |cx: &mut TestAppContext| cx.read(|cx| Theme::global(cx).theme_name().to_string());
    assert_eq!(name(cx), ottero_ui::theme::DARK);

    // Clicking the light preview card switches cleanly to Ottero Light.
    click(&app, cx, "theme-ottero-light");
    assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Light);
    assert_eq!(name(cx), ottero_ui::theme::LIGHT);
    assert_eq!(
        cx.read(|cx| Theme::global(cx).highlight_theme.name.clone()),
        ottero_ui::theme::LIGHT,
        "code colors follow the theme"
    );

    let saved = ottero_ui::appearance::AppearanceSettings::load(&app.paths.appearance);
    assert_eq!(saved.dark_theme, ottero_ui::theme::DARK);
    assert_eq!(saved.light_theme, ottero_ui::theme::LIGHT);

    // Clicking the dark preview card switches back to Ottero Dark.
    click(&app, cx, "theme-ottero-dark");
    assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Dark);
    assert_eq!(name(cx), ottero_ui::theme::DARK);
}

#[gpui_kit::test]
fn calendar_quick_add_keyboard_views_and_moves(cx: &mut TestAppContext) {
    use ottero_desktop::actions::{GoCalendar, NewEvent};
    use ottero_desktop::views::calendar_layout::CalendarMode;

    let app = launch(cx);
    dispatch(&app, cx, GoCalendar);
    assert_eq!(route(&app, cx), Route::Calendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let today = ottero_core::clock::today();

    // Quick add from the "New event" command.
    dispatch(&app, cx, NewEvent);
    type_text(&app, cx, "Lunch with Sam tomorrow 1pm");
    // Calendar keys must not fire while typing ("w", "t", "m" are in the text).
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Month);
    press(&app, cx, "enter");
    let tomorrow = today.tomorrow().expect("date");
    let span = cx.read(|cx| calendar.read(cx).span().clone());
    let occ = span
        .on_day(tomorrow)
        .next()
        .copied()
        .expect("event shown on its day");
    let event = span.events.get(&occ.event).expect("event");
    assert_eq!(event.title(), "Lunch with Sam");
    assert_eq!(
        event.start(),
        tomorrow.to_datetime(ottero_core::Time::constant(13, 0, 0, 0))
    );
    assert_eq!(
        cx.read(|cx| calendar.read(cx).anchor()),
        tomorrow,
        "selects where it landed"
    );

    // Grid keys once the grid has focus.
    dispatch(&app, cx, GoCalendar);
    press(&app, cx, "w");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Week);
    press(&app, cx, "t");
    assert_eq!(cx.read(|cx| calendar.read(cx).anchor()), today);
    press(&app, cx, "right");
    assert_eq!(cx.read(|cx| calendar.read(cx).anchor()), tomorrow);
    press(&app, cx, "d");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Day);
    press(&app, cx, "a");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Agenda);
    press(&app, cx, "m");
    assert_eq!(cx.read(|cx| calendar.read(cx).mode()), CalendarMode::Month);

    // Moving it (what a drag does) refreshes the view.
    let id = event.id();
    let later = tomorrow.tomorrow().expect("date");
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.move_event(id, later, cx).expect("move");
        })
    });
    cx.run_until_parked();
    let span = cx.read(|cx| calendar.read(cx).span().clone());
    assert!(span.on_day(tomorrow).next().is_none());
    assert_eq!(span.on_day(later).next().map(|o| o.event), Some(id));
}

#[gpui_kit::test]
fn every_calendar_view_renders_events_and_tasks(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;
    use ottero_desktop::views::calendar_layout::CalendarMode;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            for text in [
                "Standup 9:30-9:45 every weekday",
                "Review 9-10am",
                "Overlap 9:30-11am",
                "Trip nov 20-22",
                "Flight 10pm-2am",
            ] {
                lib.quick_add_event(text, today, cx).expect("event");
            }
            lib.create_task("Pay rent", Some(today), cx).expect("task");
        })
    });
    dispatch(&app, cx, GoCalendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    for (mode, _) in CalendarMode::OPTIONS {
        cx.update(|cx| calendar.update(cx, |c, cx| c.set_mode(mode, cx)));
        cx.run_until_parked();
        cx.update_window(app.window, |_, window, _| window.refresh())
            .expect("window");
        cx.run_until_parked();
        let span = cx.read(|cx| calendar.read(cx).span().clone());
        assert!(
            span.on_day(today).count() >= 3,
            "{mode:?} shows today's events"
        );
        assert_eq!(span.tasks_on(today).count(), 1, "{mode:?} shows the task");
    }
}

#[gpui_kit::test]
fn habits_quick_add_check_in_keyboard_and_archive(cx: &mut TestAppContext) {
    use ottero_core::habit::Schedule;
    use ottero_desktop::actions::{GoHabits, NewHabit};

    let app = launch(cx);
    dispatch(&app, cx, NewHabit);
    assert_eq!(route(&app, cx), Route::Habits);
    type_text(&app, cx, "Drink water 3 times a day");
    press(&app, cx, "enter");
    type_text(&app, cx, "Run 3x a week");
    press(&app, cx, "enter");
    let habits = cx.read(|cx| app.shell.read(cx).habits().clone());
    let list = cx.read(|cx| habits.read(cx).habits().to_vec());
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].habit.name(), "Drink water");
    assert_eq!(list[0].habit.target(), 3);
    assert_eq!(list[1].habit.schedule(), Schedule::PerWeek(3));

    // Keyboard: select the first habit, Space counts up toward the goal.
    dispatch(&app, cx, GoHabits);
    press(&app, cx, "down");
    press(&app, cx, "space");
    press(&app, cx, "space");
    let water = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert_eq!(water.today_count, 2);
    assert!(!water.done_today());
    press(&app, cx, "space");
    let water = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert!(water.done_today());
    assert_eq!(water.stats.current_streak, 1);

    // Archiving moves it out of the active list; the history stays.
    let id = water.habit.id();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.archive_habit(id, cx).expect("archive");
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| habits.read(cx).habits().len()), 1);
    cx.update(|cx| habits.update(cx, |h, cx| h.set_archived(true, cx)));
    let archived = cx.read(|cx| habits.read(cx).habits().to_vec());
    assert_eq!(archived[0].today_count, 3, "history kept");

    // Future check-ins are refused.
    let tomorrow = ottero_core::clock::today().tomorrow().expect("date");
    let other = list[1].habit.id();
    let err = cx.update(|cx| {
        app.shell
            .read(cx)
            .library()
            .clone()
            .update(cx, |lib, cx| lib.tap_habit(other, tomorrow, cx).is_err())
    });
    assert!(err);
}

#[gpui_kit::test]
fn habit_insights_render_with_history(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoHabits;
    use ottero_desktop::views::habits::HabitTab;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            let read = lib.quick_add_habit("Read daily", cx).expect("habit");
            lib.quick_add_habit("Run 3x a week", cx).expect("habit");
            for k in 1..20 {
                let day = today.checked_add(jiff::Span::new().days(-k)).expect("date");
                lib.set_habit_count(read, day, 1, cx).expect("check-in");
            }
        })
    });
    dispatch(&app, cx, GoHabits);
    let habits = cx.read(|cx| app.shell.read(cx).habits().clone());
    cx.update(|cx| habits.update(cx, |h, cx| h.set_tab(HabitTab::Insights, cx)));
    cx.update_window(app.window, |_, window, _| window.refresh())
        .expect("window");
    cx.run_until_parked();
    let read = cx.read(|cx| habits.read(cx).habits()[0].clone());
    assert_eq!(read.stats.current_streak, 19, "backfilled days count");
    assert!(read.stats.strength > 0.5);
    // Keyboard selection does nothing on Insights (no rows to act on).
    press(&app, cx, "down");
    assert_eq!(cx.read(|cx| habits.read(cx).selected_habit()), None);
}

#[gpui_kit::test]
fn calendar_routines_can_be_hidden(cx: &mut TestAppContext) {
    use ottero_desktop::actions::GoCalendar;

    let app = launch(cx);
    let today = ottero_core::clock::today();
    cx.update(|cx| {
        app.shell.read(cx).library().clone().update(cx, |lib, cx| {
            lib.quick_add_event("Gym 7am every day", today, cx)
                .expect("event");
            lib.quick_add_event("Dentist 4pm", today, cx)
                .expect("event");
        })
    });
    dispatch(&app, cx, GoCalendar);
    let calendar = cx.read(|cx| app.shell.read(cx).calendar().clone());
    let shown = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let c = calendar.read(cx);
            c.span()
                .on_day(today)
                .filter(|o| c.shows_for_test(o, cx))
                .count()
        })
    };
    assert_eq!(shown(cx), 2);
    cx.update(|cx| ottero_ui::appearance::update(cx, |s| s.calendar_routines = false));
    cx.update_window(app.window, |_, window, _| window.refresh())
        .expect("window");
    cx.run_until_parked();
    assert_eq!(shown(cx), 1, "only the one-off dentist visit");
}

#[gpui_kit::test]
fn today_dashboard_shows_schedule_habits_and_respects_layout(cx: &mut TestAppContext) {
    use ottero_desktop::views::today::set_shown;
    use ottero_desktop::views::today_layout::TodaySection;

    let app = launch(cx);
    let lib = library(&app, cx);
    let today = ottero_core::clock::today();
    let (event, habit) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let e = l
                .quick_add_event("Dentist 11:30pm", today, cx)
                .expect("event");
            let h = l.quick_add_habit("Read daily", cx).expect("habit");
            (e, h)
        })
    });
    dispatch(&app, cx, GoToday);
    let find = |cx: &mut TestAppContext, id: String| {
        cx.update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(leak(id)).is_some()
        })
        .expect("window")
    };
    assert!(
        find(cx, format!("today-event-{event}")),
        "event in today's schedule"
    );
    assert!(find(cx, "today-glance".into()));
    assert!(
        find(cx, format!("habit-check-{habit}")),
        "habit check on Today"
    );

    // Checking the habit from Today saves it.
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        let id: &'static str = leak(format!("habit-check-{habit}"));
        window.click(id, cx);
    })
    .expect("window");
    cx.run_until_parked();
    let done = cx.read(|cx| lib.read(cx).habits(false).expect("habits")[0].done_today());
    assert!(done, "checked from Today");

    // Hiding a section removes it; the setting persists in appearance.
    cx.update(|cx| set_shown(TodaySection::Agenda, false, cx));
    cx.run_until_parked();
    assert!(!find(cx, "today-agenda".into()));
    assert!(find(cx, "today-habits".into()));
}
