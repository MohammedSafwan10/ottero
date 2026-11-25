use super::*;
use crate::core_ui::*;

// ---- Notes, Tasks and Today: the "finish" batch ----

pub(crate) fn secondary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub(crate) fn library(
    app: &App,
    cx: &mut TestAppContext,
) -> Entity<ottero_desktop::library::Library> {
    cx.read(|cx| app.shell.read(cx).library().clone())
}

pub(crate) fn task_by_id(
    app: &App,
    cx: &mut TestAppContext,
    id: ottero_core::TaskId,
) -> ottero_core::Task {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .tasks_by_id(id)
            .expect("task")
    })
}

pub(crate) fn open_tasks(app: &App, cx: &mut TestAppContext) -> Vec<ottero_core::Task> {
    cx.read(|cx| {
        app.shell
            .read(cx)
            .library()
            .read(cx)
            .tasks(TaskFilter::Open)
            .expect("tasks")
    })
}

#[gpui_kit::test]
fn quick_add_reads_date_priority_and_project_from_the_text(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let home = cx.update(|cx| {
        lib.update(cx, |l, cx| l.create_project("Home", cx))
            .expect("project")
    });

    dispatch(&app, cx, NewTask);
    type_text(&app, cx, "Plan Monday standup tomorrow !high #Home");
    press(&app, cx, "enter");
    // An unknown project stays in the title instead of being guessed.
    type_text(&app, cx, "Buy paint #Garage");
    press(&app, cx, "enter");

    let tasks = open_tasks(&app, cx);
    let plan = tasks
        .iter()
        .find(|t| t.title() == "Plan Monday standup")
        .expect("dates are read only from the end of the text");
    assert_eq!(plan.due(), ottero_core::clock::today().tomorrow().ok());
    assert_eq!(plan.priority(), ottero_core::Priority::High);
    assert_eq!(plan.project(), Some(home));

    let paint = tasks
        .iter()
        .find(|t| t.title().starts_with("Buy paint"))
        .expect("paint");
    assert_eq!(paint.title(), "Buy paint #Garage");
    assert_eq!(paint.project(), None);
    assert_eq!(
        paint.due(),
        Some(ottero_core::clock::today()),
        "no date in the text: the Today tab's default applies"
    );
}

#[gpui_kit::test]
fn completing_a_repeating_task_schedules_the_next_one(cx: &mut TestAppContext) {
    let app = launch(cx);
    dispatch(&app, cx, NewTask);
    type_text(&app, cx, "Water plants every day");
    press(&app, cx, "enter");

    let today = ottero_core::clock::today();
    let first = open_tasks(&app, cx);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].title(), "Water plants");
    assert_eq!(
        first[0].recurrence(),
        Some(ottero_core::Recurrence::Days(1))
    );
    assert_eq!(first[0].due(), Some(today));

    let id = first[0].id();
    let lib = library(&app, cx);
    cx.update(|cx| {
        lib.update(cx, |l, cx| l.toggle_task(id, cx))
            .expect("complete")
    });

    assert_eq!(task_by_id(&app, cx, id).status(), TaskStatus::Done);
    let next = open_tasks(&app, cx);
    assert_eq!(next.len(), 1, "exactly one new instance");
    assert_ne!(next[0].id(), id);
    assert_eq!(next[0].title(), "Water plants");
    assert_eq!(next[0].due(), today.tomorrow().ok());
    assert_eq!(next[0].recurrence(), Some(ottero_core::Recurrence::Days(1)));
}

#[gpui_kit::test]
fn task_detail_adds_checks_and_removes_steps(cx: &mut TestAppContext) {
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let lib = library(&app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| l.create_task("Move house", None, cx))
            .expect("create")
    });
    dispatch(&app, cx, GoTasks);
    cx.update_window(app.window, |_, _, cx| {
        app.shell
            .read(cx)
            .tasks()
            .clone()
            .update(cx, |t, cx| t.set_tab(TaskTab::All, cx));
    })
    .expect("window");
    cx.run_until_parked();
    click(&app, cx, leak(format!("task-{id}-open")));

    click(&app, cx, "new-step");
    type_text(&app, cx, "Book van");
    press(&app, cx, "enter");
    type_text(&app, cx, "Pack books");
    press(&app, cx, "enter");

    let steps = |cx: &mut TestAppContext| cx.read(|cx| lib.read(cx).subtasks(id));
    let progress =
        |cx: &mut TestAppContext| cx.read(|cx| lib.read(cx).subtask_progress().get(&id).copied());
    let titles: Vec<String> = steps(cx).iter().map(|s| s.title().to_owned()).collect();
    assert_eq!(
        titles,
        ["Book van", "Pack books"],
        "Enter adds a step and clears the field"
    );
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((0, 2)));
    assert!(
        open_tasks(&app, cx).iter().all(|t| t.parent().is_none()),
        "steps don't show up as top-level tasks"
    );

    let van = steps(cx)[0].id();
    click(&app, cx, leak(format!("step-{van}")));
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((1, 2)));

    click(&app, cx, leak(format!("step-{van}-remove")));
    assert_eq!(steps(cx).len(), 1);
    assert_eq!(progress(cx).map(|p| (p.done, p.total)), Some((0, 1)));
}

#[gpui_kit::test]
fn projects_are_created_shown_renamed_and_deleted_without_losing_tasks(cx: &mut TestAppContext) {
    use ottero_desktop::actions::NewProject;
    let app = launch(cx);
    cx.update(|cx| cx.set_reduce_motion(true));

    dispatch(&app, cx, NewProject);
    assert!(dialog_open(&app, cx), "New project asks for a name");
    type_text(&app, cx, "Garden");
    press(&app, cx, "enter");
    assert!(!dialog_open(&app, cx));

    let lib = library(&app, cx);
    let projects = cx.read(|cx| lib.read(cx).projects());
    assert_eq!(projects.len(), 1);
    let garden = projects[0].0.id();
    assert_eq!(projects[0].0.name(), "Garden");
    assert_eq!(route(&app, cx), Route::Tasks);
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).tasks().read(cx).project()),
        Some(garden),
        "the new project opens"
    );

    // Quick add inside a project files the task there, with no default date.
    click(&app, cx, "task-input");
    type_text(&app, cx, "Plant tulips");
    press(&app, cx, "enter");
    let tulips = open_tasks(&app, cx)
        .into_iter()
        .find(|t| t.title() == "Plant tulips")
        .expect("task");
    assert_eq!(tulips.project(), Some(garden));
    assert_eq!(tulips.due(), None);

    click(&app, cx, "project-more");
    click_label(&app, cx, "Rename…");
    assert!(dialog_open(&app, cx));
    press(&app, cx, &secondary("a"));
    type_text(&app, cx, "Backyard");
    press(&app, cx, "enter");
    assert_eq!(
        cx.read(|cx| lib.read(cx).project(garden))
            .map(|p| p.name().to_owned()),
        Some("Backyard".to_owned())
    );

    click(&app, cx, "project-more");
    click_label(&app, cx, "Delete project…");
    assert!(dialog_open(&app, cx), "deleting a project asks first");
    press(&app, cx, "enter");
    assert!(cx.read(|cx| lib.read(cx).projects()).is_empty());
    let kept = task_by_id(&app, cx, tulips.id());
    assert_eq!(kept.project(), None, "its tasks stay, unfiled");
    assert!(!kept.is_trashed());
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).tasks().read(cx).project()),
        None
    );
}

#[gpui_kit::test]
fn daily_note_opens_todays_note_and_never_duplicates_it(cx: &mut TestAppContext) {
    use ottero_desktop::actions::OpenDailyNote;
    let app = launch(cx);
    dispatch(&app, cx, OpenDailyNote);
    assert_eq!(route(&app, cx), Route::Notes);
    let title = ottero_desktop::library::daily_note_title(ottero_core::clock::today());
    let first = cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id());
    assert!(first.is_some());
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_title(cx)),
        Some(title.clone())
    );

    dispatch(&app, cx, GoToday);
    dispatch(&app, cx, OpenDailyNote);
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id()),
        first
    );
    assert_eq!(
        all_note_titles(&app, cx)
            .iter()
            .filter(|t| **t == title)
            .count(),
        1
    );
}

#[gpui_kit::test]
fn tag_filter_shows_only_tagged_notes(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let (car, recipe) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let car = l.create_note("Car", cx).expect("create");
            l.edit_note(car, "Car", "Tyres next month #car #todo", cx)
                .expect("edit");
            let recipe = l.create_note("Soup", cx).expect("create");
            l.edit_note(recipe, "Soup", "Lentils #cooking", cx)
                .expect("edit");
            (car, recipe)
        })
    });
    let tags: Vec<String> = cx
        .read(|cx| lib.read(cx).note_tags())
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert!(
        tags.contains(&"car".to_owned()) && tags.contains(&"cooking".to_owned()),
        "{tags:?}"
    );

    dispatch(&app, cx, GoNotes);
    let shown = |id: ottero_core::NoteId, cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(leak(format!("note-{id}"))).is_some()
        })
        .expect("window")
    };
    assert!(shown(car, cx) && shown(recipe, cx));

    cx.update_window(app.window, |_, window, cx| {
        app.shell.read(cx).notes().clone().update(cx, |n, cx| {
            n.set_tag_filter(Some("#Car".into()), window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| app
            .shell
            .read(cx)
            .notes()
            .read(cx)
            .tag_filter()
            .map(str::to_owned)),
        Some("car".to_owned()),
        "tags are normalized"
    );
    assert!(shown(car, cx));
    assert!(!shown(recipe, cx), "untagged notes are hidden");

    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.set_tag_filter(None, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    assert!(
        shown(recipe, cx),
        "clearing the filter shows everything again"
    );
}

#[gpui_kit::test]
fn unlinked_mentions_can_be_turned_into_links(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let trip = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let trip = l.create_note("Trip", cx).expect("create");
            let packing = l.create_note("Packing", cx).expect("create");
            l.edit_note(packing, "Packing", "Things for the trip: tent", cx)
                .expect("edit");
            trip
        })
    });
    let mentions = cx.read(|cx| lib.read(cx).unlinked_mentions(trip, "Trip"));
    assert_eq!(mentions.len(), 1);
    let source = mentions[0].id;

    let linked = cx.update(|cx| {
        lib.update(cx, |l, cx| l.link_mention(source, "Trip", cx))
            .expect("link")
    });
    assert!(linked);
    let body = cx.read(|cx| {
        lib.read(cx)
            .note(source)
            .expect("load")
            .expect("note")
            .body()
            .to_owned()
    });
    assert_eq!(
        body, "Things for the [[trip]]: tent",
        "keeps the original casing"
    );
    assert!(
        cx.read(|cx| lib.read(cx).unlinked_mentions(trip, "Trip"))
            .is_empty()
    );
    assert_eq!(
        cx.read(|cx| lib.read(cx).backlinks("Trip"))
            .expect("backlinks")
            .len(),
        1
    );
}

#[gpui_kit::test]
fn format_shortcuts_edit_the_selection_as_markdown(cx: &mut TestAppContext) {
    let app = launch(cx);
    create_and_open_note(&app, cx, "Formatting");
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.focus_body(window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    let body =
        |cx: &mut TestAppContext| cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx));

    press(&app, cx, &secondary("a"));
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "**body**");
    // The selection stays on the text, so the same shortcut toggles it off.
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "body");

    press(&app, cx, &secondary("alt-1"));
    assert_eq!(body(cx), "# body");

    // Shortcuts only act inside the editor: in the title they do nothing.
    cx.update_window(app.window, |_, window, cx| {
        app.shell.read(cx).notes().clone().update(cx, |n, cx| {
            n.focus_search(window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    press(&app, cx, &secondary("b"));
    assert_eq!(body(cx), "# body");
}

#[gpui_kit::test]
fn long_notes_scroll_the_page_not_the_editor(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let long: String = (1..=80).map(|i| format!("Line {i}\n")).collect();
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_note("Long", cx).expect("create");
            l.edit_note(id, "Long", &long, cx).expect("edit");
            id
        })
    });
    cx.update_window(app.window, |_, window, cx| {
        app.shell.update(cx, |s, cx| s.open_note(id, window, cx));
    })
    .expect("window");
    cx.run_until_parked();

    // Row counts are known after layout.
    let (body, tail, page, window_height) = cx
        .update_window(app.window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            (
                window.find("note-body").bounds(),
                window.find("note-body-tail").bounds(),
                window.find("note-scroll").bounds(),
                window.bounds().size.height,
            )
        })
        .expect("window");
    let font = cx.read(|cx| Theme::global(cx).font_size);
    let text_height = body.size.height - tail.size.height;
    let rows_shown = text_height / (font * zorite_editor::LINE_HEIGHT_RATIO);
    assert!(
        rows_shown >= 80.,
        "editor is as tall as its 80 lines, so it never scrolls inside: {rows_shown}"
    );
    assert!(
        body.size.height > page.size.height && page.size.height <= window_height,
        "the page is the one scroll area: editor {body:?}, page {page:?}"
    );
}

#[gpui_kit::test]
fn today_shows_the_week_and_the_next_seven_days(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let today = ottero_core::clock::today();
    let in_three = today.checked_add(jiff::Span::new().days(3)).expect("date");
    let in_twenty = today.checked_add(jiff::Span::new().days(20)).expect("date");
    let (soon, later) = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let soon = l
                .create_task("Dentist", Some(in_three), cx)
                .expect("create");
            let later = l
                .create_task("Renew passport", Some(in_twenty), cx)
                .expect("create");
            let done = l.create_task("Pay rent", Some(today), cx).expect("create");
            l.toggle_task(done, cx).expect("complete");
            (soon, later)
        })
    });

    let week = cx.read(|cx| lib.read(cx).completions_by_day(7));
    assert_eq!(week.len(), 7);
    assert_eq!(
        week.last(),
        Some(&(today, 1)),
        "today's completion is counted"
    );

    dispatch(&app, cx, GoToday);
    cx.update_window(app.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("week-chart").is_some(),
            "week chart renders"
        );
        assert!(
            window.try_find(leak(format!("upcoming-{soon}"))).is_some(),
            "due in 3 days is listed"
        );
        assert!(
            window.try_find(leak(format!("upcoming-{later}"))).is_none(),
            "20 days out is not"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn synced_note_update_refreshes_open_editor_live(cx: &mut TestAppContext) {
    let app = launch(cx);
    let lib = library(&app, cx);
    let id = cx.update(|cx| {
        lib.update(cx, |l, cx| {
            let id = l.create_note("Sync test", cx).expect("create");
            l.edit_note(id, "Sync test", "Initial text", cx)
                .expect("edit");
            id
        })
    });

    dispatch(&app, cx, GoNotes);
    cx.update_window(app.window, |_, window, cx| {
        app.shell
            .read(cx)
            .notes()
            .clone()
            .update(cx, |n, cx| n.open_note(id, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx)),
        "Initial text"
    );

    // Simulate a remote sync writing a newer title and body to SQLite and
    // emitting LibraryEvent::NotesChanged while the note is open and clean.
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.edit_note(id, "Synced title", "Updated from another device", cx)
                .expect("remote edit");
        })
    });
    cx.run_until_parked();

    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_title(cx)),
        Some("Synced title".to_owned())
    );
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).body_text(cx)),
        "Updated from another device"
    );

    // Remote permanent deletion clears the open editor cleanly.
    cx.update(|cx| {
        lib.update(cx, |l, cx| {
            l.trash_note(id, cx).expect("trash");
            l.delete_note_permanently(id, cx).expect("delete");
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| app.shell.read(cx).notes().read(cx).open_note_id()),
        None
    );
}
