# Ottero work queue

The owner's ordered backlog (2026-11-08). Work top to bottom. Mark `[x]` only
when the item is built, tested at the right layer, looked at in light and
dark, documented, and `scripts/check.sh` passes. Each finished item is its own
commit. Read this file first when resuming.

Quality bar: AGENTS.md. Design rules: `.kiro/skills/gpui-kit-design-guides`.
Calm, organized, not noisy; keyboard path; empty/error states; one source of truth.

## 1. Editor performance (measured, not eyeballed)

- [x] `scripts/perf.sh`: real keystrokes/scroll through the HID tap, GPUI
      profiler (`--features perf`) reports input→present latency p50/p95/p99,
      draw time, late frames, missed refreshes, slow views/tasks.
- [x] Baseline numbers for 1.5 KB / 50 KB / 500 KB / 2 MB notes, typing and scrolling.
- [x] Fix what the numbers show: 7 root-cause patches in the vendored editor
      (`vendor/zorite-editor/PATCHES.md`). 50 KB typing p95 341 → 23 ms, 0 missed refreshes.
- [x] Before/after in `docs/perf.md`. Up to 50 KB: no dropped frames.
- [ ] 500 KB+ at 60 fps needs incremental layout in the editor (reuse unchanged
      lines across edits); today 500 KB ≈ 30 fps, 2 MB ≈ 7 fps. Do after the
      feature phases unless a real note gets that big.

## 2. Editor features left over

- [x] Find in note (⌘F): bar in the toolbar's left slot, "2 of 7", ⌘G/⇧⌘G and
      Enter/⇧Enter, wraps, Esc returns to the text, seeded from a one-line selection,
      counts follow edits, scrolls the match into view. UI test.
- [x] Code-block syntax colors: syntect + two-face (~200 languages, aliases like py/ts/sh),
      cached per block, colors from the theme JSON `highlight.syntax` (GitHub-like palettes).
- [x] Mermaid diagrams: `mermaid-rs-renderer` (pinned git rev, pure Rust) → themed SVG →
      GPUI SVG raster, natural size, cached; source on caret; bad source stays text.
- [x] Spell-check: OS checker (`os-spellcheck`), prose only (`core::format::non_prose_ranges`),
      400 ms debounce, skipped over 256 KB, right-click suggestions, Settings → Writing toggle.

## 3. Settings → Appearance, themes, typography

- [x] Appearance reorganized: Theme (mode) → Light theme → Dark theme (preview cards in
      each theme's own colors) → Text (fonts, sizes, line spacing, live preview) → Layout →
      Writing and motion → Reset.
- [x] Focused house theme pair: **Ottero Light (Quiet Alabaster)** and
      **Ottero Dark (Smoked Obsidian)** in `crates/ui/themes/ottero.json`.
      Mode (`System` / `Light` / `Dark`) switches instantly; third-party editor
      theme packs removed in `0.3.0`.
- [x] Typography: interface/code fonts and sizes (existing), note line spacing
      (Compact/Comfortable/Relaxed, vendored patch P8). UI test covers theme pick + persistence.

## 4. Calendar

- [x] Research native calendar UX (Fantastical, Apple Calendar, Notion Calendar, Cron).
- [x] Domain: events (all-day, timed, recurring), storage + migration 0003, tests.
- [x] UI: month / week / day / agenda, keyboard nav, quick add with natural language,
      event detail sheet, tasks with due dates shown (drag to reschedule), today line,
      empty/error states, Settings → Calendar (week start, clock). UI tests.

## 5. Habits

- [x] Research (Streaks, Loop Habit Tracker, Things-style calm).
- [x] Domain: habit, schedule (daily / weekdays / n per week), daily goal, check-ins,
      streaks, strength (EMA), insights (rates by day / weekday / period); migration 0004.
- [x] UI: Today list (week as dots, streak, check), Insights tab (headline numbers,
      30-day chart, weekday chart, strength ranking, per-habit table), Archived tab,
      habit panel (schedule, goal, stats, 90-day strength line, heatmap). Reminders later.

## 6. Today dashboard, next level

- [x] Rebuilt: at a glance (next event, tasks, habits), today's schedule with a now
      marker, habits with one-tap checks, due tasks, this week (tasks + habits kept),
      next 7 days, recent notes. Sections are a data table (`today_layout.rs`);
      Customize menu shows/hides, Settings → Today reorders. UI tests.

## 7. Password vault

- [x] Research (1Password, Bitwarden, KeePassXC, Proton Pass; OWASP Argon2; EFF words).
- [x] Vault per `docs/plans/05-vault.md` + `docs/security.md`: setup, unlock/lock
      state machine, auto-lock (idle, background, wake from sleep), logins/cards/notes,
      TOTP codes, generator (characters / EFF words), password health, copy with
      clipboard clear, search over thousands of items, CSV import, change master
      password. Master–detail UI. Security review fixes: drafts wiped on lock,
      generator closes on lock, RNG fails closed, sleep detection.

## Done

(Items move here with the commit hash.)
