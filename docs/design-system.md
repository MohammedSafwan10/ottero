# Design system

Normative guides: `.kiro/skills/gpui-kit-design-guides` (read before UI work).
This file records Ottero's choices on top of them.

## Principles

- Quiet, monochrome, precise. Black and white carry structure; color only
  carries meaning (danger red, success green, warning amber).
- Desktop conventions: persistent sidebar, keyboard first (every action has a
  command, most have a shortcut), command palette (⌘K / Ctrl K) reaches everything.
- One focal point per screen. Few borders, no nested cards, no decorative shadows.

## Theme

Ottero ships one curated house theme pair in `crates/ui/themes/ottero.json`:
**Ottero Light** (Quiet Alabaster: warm stone-ivory sidebar, alabaster paper
canvas, white elevated cards, charcoal-ink primary CTA, muted sage/ochre/slate
data tints) and **Ottero Dark** (Smoked Obsidian: recessed `#0e0f11` sidebar,
`#131417` canvas, elevated `#1a1c20` cards, crisp off-white primary CTA, muted
sage/sand/steel data tints). Mode (System/Light/Dark) switches between them.
Every theme defines `highlight.syntax` (enforced by test).

- Radius 10 / 14 (large), subtle card elevation (`shadow: true` in light mode).
- Feature code uses tokens only: `background`, `foreground`, `muted_foreground`,
  `border`, `list_hover`, `list_active`, `group_box`, `sidebar_*`, `success`,
  `warning`, `info`, `danger`...

## Appearance settings (user-controlled)

| Setting | Values | Effect |
| --- | --- | --- |
| Appearance | System / Light / Dark | Theme mode; System follows the OS live. |
| Content width | Compact / Comfortable / Wide | Max width of the note editor (38/48/72 rem). |
| Reduce motion | System / On / Off | Disables menu/dialog/toast animations. |
| Interface font | Inter / System + size 12–20 px (default 15) | Size is the `rem`, so it zooms the whole UI. |
| Code font | JetBrains Mono / System + size 10–20 px | Code in notes. |

Stored in `appearance.json`; `Restore default appearance` resets.

## Typography

Inter (UI) and JetBrains Mono (code), bundled, SIL OFL 1.1. Scale: `text_xs`
metadata, `text_sm` body/list, `text_lg` pane titles, `text_2xl` page titles.
Weights: Medium for row titles, Semibold for headings. No uppercase labels.

## Spacing and size

Use rem helpers (`p_4`, `gap_2`, `rems(48.)`), never raw `px` for layout, so
interface size scales everything. Exceptions: hairlines and the dialog width.

## Icons

Lucide (1,800+ icons, one consistent stroke style, ISC). Use
`gpui_kit::assets::IconName as Lucide`, and register each in
`crates/ui/src/assets.rs` (only listed icons are embedded, keeping the binary small).

## Sidebar

- Data-driven in `apps/desktop/src/nav.rs`: Section → Entry (icon, route,
  badge) → Child deep links (e.g. Tasks → Today/Upcoming/All/Done/Trash).
- Collapses to icons (⌘\ or the title-bar button at the same spot), animated.
- Resizable by dragging its edge (180–360 px); double-click the edge resets.
  Width and collapsed state persist.
- Settings lives in the footer.
- Settings → Workspace controls sidebar visibility and order with switches
  and direct up/down buttons. Keyboard focus stays on a moved control. No drag
  handles or row overflow menus. Reset offers Undo and does not change startup.
- Hidden features appear under an inline More entry immediately after the
  visible features; omit More when everything is shown. Search and Settings
  remain available even when every feature is hidden. Hiding is presentation
  only: commands, data, drafts, vault locking and running timers stay intact.
- Feature metadata lives in `nav.rs`; `navigation.rs` resolves stable saved IDs
  against that catalog. Fresh installations show Today/Notes/Tasks/Calendar;
  upgrades preserve prior entries and the legacy Developer preference. Newly
  introduced features go to More until selected. Save order, visibility and
  startup in `layout.json`, independently of theme preferences.

## Settings organization

Account, General (startup), Workspace (sidebar), Appearance, Features (existing
Notes, Today, Calendar/holiday preferences), Privacy & data (vault security,
backups, storage), Keyboard, About. Keep future feature preferences under
Features; only add controls for implemented behavior. Native search spans all
categories, including preferences for features hidden from the main sidebar.
Customize sidebar (palette or More) opens Workspace directly. The selected
startup page may be hidden without being pinned or opening it immediately.

## Feedback rules (`ottero_ui::feedback`)

| Situation | Pattern |
| --- | --- |
| Reversible (move to trash, complete) | Do it now, `undo_toast("… moved to trash.")` |
| Permanent (delete forever, empty trash) | `confirm_destructive` with a verb button ("Delete", "Empty trash"), Cancel default, Escape cancels |
| Failure | `error_toast` with a user-safe message |
| Status / informational | `info_toast` (deduplicated by message) |
| Editing or deleting one occurrence of a repeating event | `ask_repeat_scope` ("Only this event" primary, "All events", Cancel); a drag moves only that one and offers Undo |

Toasts sit bottom-right, capped at `max_items = 3` and `340 px` wide, with
deduplication IDs on update, clipboard, error, undo, and info notices so
repeated alerts replace in place instead of stacking.
Never confirm reversible actions. Never use "OK" or "Yes" as the confirm label.

## Panels

One layout grammar for every screen: **Sidebar (app nav) → List panel → Content → optional Inspector**.

- List panels and inspectors use `ottero_ui::panel::SidePanel`: drag the
  inner edge to resize (follows the pointer exactly, clamped by its `PanelSpec`),
  double-click the edge to reset, collapse/expand animates ~180 ms (snaps with reduced motion).
- Width and collapsed state live in the shared `LayoutStore` (`layout.json`), one entry per panel id.
- Toggle buttons sit in the title bar so they never move when a panel collapses.
  Sidebar ⌘\\, note list ⌘⇧L. A new panel adds a `PanelSpec`, an action, and a title-bar toggle shown on its screen.

## Shortcuts

- One table, `actions::SHORTCUTS` (keys, label, group, action), drives key
  bindings, the Keyboard settings page (grouped: General, Create, Go to, View,
  Writing) and the palette hints. Add a shortcut there only.
- Keys use the platform modifier: `cmd-` on macOS, `ctrl-` on Windows/Linux.
  `Kbd` renders the platform symbols (⌘⇧N vs Ctrl+Shift+N). ⌘Q is macOS only.
- The palette shows a shortcut next to each command so people learn them
  where they already are (Linear/VS Code/Raycast pattern).

## Sidebar header and footer

- No logo or app name (that's the menu bar and Dock). The top row is
  "Search ⌘K"; its hover covers only that row.
- Settings is an icon button in the footer (tooltip shows name + shortcut).

## Overlays

- One dialog at a time. ⌘K toggles the command palette (a second press closes
  it) and never opens over another dialog; `confirm_destructive` won't stack either.
- Opening a sheet replaces the current one.
- The palette's visible entry point is "Search ⌘K" at the top of the sidebar
  (an icon button when the sidebar is collapsed); the title bar holds only panel toggles.

## Menus

Every row menu follows the same shape (see `views/task_row.rs`, `note_row_menu`):

1. **Open** (details / open).
2. **Change** values. A property with several options becomes a submenu whose
   label shows the current value ("Due: Tomorrow", "Priority: High"); the
   current option is checked inside it.
3. **Status** actions: only transitions the state machine allows.
4. **Destructive** last, after a separator ("Move to trash"; in Trash: "Restore", "Delete forever…").

- Max ~7 top-level items; group with separators, never section headers inside the top level.
- Every item has an icon from the same Lucide set.
- The "…" button and right-click on the row open the same menu (one builder).
- "…" in a label means another step follows (a dialog or confirmation).

## Note editor

- One mode: live preview (ADR 0004). Formatting shows while typing; Markdown
  markers are hidden and reappear only on the caret's line. No Edit/Preview
  switch. "Show Markdown source" (… menu, palette) shows the raw text.
- Toolbar has two controls, right-aligned: **Aa** (format menu) and **…**
  (pin, Markdown source, export, move to trash).
- Selecting text shows a small bar just above it: bold, italic,
  strikethrough, highlight, code, link. It appears when the drag ends, hides
  on typing, Escape, scroll or collapse, and never takes focus from the text.
- Format menu: the six inline styles, then **Headings ▸** and **Lists ▸**.
- `[[` opens note suggestions at the caret: ↑/↓ choose, Enter/Tab insert,
  Esc closes. Links and wiki-links are blue (`theme.blue`) so they stand out
  in running text.
- Notes stay Markdown. Colors, underline and colored highlights from the
  editor's right-click menu are inline HTML (`<span style>`, `<u>`, `<mark>`),
  which Obsidian and GitHub also render.
- `$$…$$` math renders (ratex); the caret inside shows the LaTeX.
- ```` ```mermaid ```` renders as a diagram in theme colors on a transparent
  background; the caret inside shows the source.
- Fenced code is colored by language (`views/code_highlight.rs`); colors live in
  `ottero.json` under `highlight.syntax`, so every theme defines its own.
- The writing column is centered with a max width (Settings → Content width).
- The editor is as tall as its text; the page is the only scroll area.
  Clicking below the text continues writing at the end.
- Spelling: red squiggles under misspelled prose (code, links, tags, URLs and
  math are never flagged); right-click for suggestions. Off in Settings → Writing.
- Find (⌘F) takes the toolbar's left slot: field, "2 of 7" (red "No matches"),
  ↑ ↓, close. ⌘G / ⇧⌘G and Enter / ⇧Enter step and wrap; Esc closes and returns
  to the text at the match. Only on the Notes screen.
- Trashed notes render read-only. Toasts sit bottom-right.

## Calendar

- Events use `primary` at 16 % (24 % hover); past items fade to 60 % opacity.
  Today is a filled `primary` pill; the now line is `primary`. No colored
  leading bars.
- Geometry in rems (`calendar_grid.rs`): day header 1.75, line 1.375, hour 3.
  Multi-day and all-day events are bars that stack (`pack_spans`); overlapping
  timed events split the column (`lay_out_timed`).
- Month cells stay calm: bars for all-day / 24 h+ events, one line per
  one-off event or task (title first, compact time "9a" after), routines
  (repeat 3+ times a week, `Event::is_routine`) folded into small dots at the
  bottom, "+N more" when full. On wide windows the selected day's full list
  sits beside the grid (`day_panel`). View options (⚙ in the header) or
  Settings → Calendar can hide routines everywhere.
- Week/day: routines fill at 8 %, plans at 16 %; blocks under an hour put the
  time inline; the now line is a 1.5 px hairline.
- Plain-key shortcuts (`CALENDAR_SHORTCUTS`) live only on the grid, never in
  the quick-add field.

## Habits

- One card, hairline rows; columns line up with the header (same padding and
  gaps, fixed lane widths in `habits.rs`).
- Past days are marks, not controls-sized boxes: solid dot = done, faint dot
  = partly, hollow ring = missed, pinpoint = not expected. Only today gets a
  real round check (`habit_widgets::check_button`).
- Charts: bars at `primary` 55 %, full `primary` only for the point that
  matters (today, best weekday); empty days at `muted_foreground` 25 %.

## Today

- Sections are cards (title + count + one quiet action, then content, then
  its own empty line). Half-width sections pair up; full-width stand alone.
- Add a section in `views/today_layout.rs` (key, title, description, width)
  and render it in `today.rs`; saved layouts are by key.

## Shared components (`crates/ui/src/components.rs`)

`segmented` (controlled segmented control), `page_header`, `empty_state`,
`section_label`. Promote a pattern here only when a second screen needs it.

## Copy

Sentence case. Short, specific, no exclamation marks. Empty states say what
the space is for and how to fill it.
