# 10 Mobile (Android First, iOS Later)

Status: In progress — Native Kotlin (Jetpack Compose) + Rust (`UniFFI`) in the
same `ottero` monorepo (`crates/mobile-ffi` + `apps/android`).

## Architecture Decision

- **UI:** Native **Jetpack Compose (Material 3, AGP 9, Kotlin)** for Android
  first, then **SwiftUI** for iOS later.
- **Core:** 100% shared Rust core (`ottero-core`, `ottero-storage`,
  `ottero-crypto`, `ottero-sync-protocol`) exposed over **Mozilla UniFFI**
  (`crates/mobile-ffi`, compiled via `cargo-ndk` with NDK 28+ for 16 KB page
  alignment on Android 15/16).
- **Why Native + UniFFI over cross-platform UI:**
  - OS Password Autofill (`AutofillService` on Android,
    `ASCredentialProviderViewController` on iOS) and Home Screen Widgets
    (`Glance` / `WidgetKit`) must be native and run in memory-capped processes
    that can call our Rust vault directly via UniFFI.
  - Smallest APK, instant cold start, 100% native keyboard/IME/text selection,
    haptics, edge-to-edge bars, and locked 120Hz scrolling.
- **Monorepo & Open Source boundary:** `apps/android` and `crates/mobile-ffi`
  live in the public `ottero` repo under GPL-3.0-only; cloud backend stays
  strictly in the private `ottero-cloud` repo.

## Design System Parity

- Exact same house themes as Desktop (`crates/ui/themes/ottero.json`):
  - **Ottero Light ("Quiet Alabaster"):** `#FAF8F5` canvas, `#F0EDE7` recessed
    surfaces, `#FFFFFF` elevated cards, `#1C1B1A` charcoal-ink primary CTA,
    muted sage (`#7E9A84`), ochre (`#B89356`), slate (`#697D91`).
  - **Ottero Dark ("Smoked Obsidian"):** `#131417` canvas, `#0E0F11` recessed
    surfaces, `#1A1C20` elevated cards, `#F3F4F6` crisp primary CTA, muted sage
    (`#7C9A82`), sand (`#C2A26C`), steel (`#6A829E`).
- **Unslop UI discipline:** Zero decorative badges, zero filler subtitles,
  zero AI-slop gradients/glows, adaptive across compact phones & large screens.

## Modular App Structure (`apps/android`)

```text
apps/android/app/src/main/java/com/nexdark/ottero/
  MainActivity.kt            # Edge-to-edge FragmentActivity + theme host
  core/
    theme/                   # Quiet Alabaster & Smoked Obsidian tokens + typography
    bridge/                  # OtteroEngine wrapper over UniFFI + Coroutines IO dispatcher
    components/              # Reusable cards, empty states, section headers, quick-add sheet
  feature/
    today/                   # TodayScreen + TodayViewModel (schedule, tasks, habits)
    notes/                   # NotesListScreen + NoteEditorScreen + NotesViewModel
    tasks/                   # TasksScreen + QuickAdd + TasksViewModel
    passwords/               # VaultScreen (setup/lock/unlock/items) + VaultViewModel
    settings/                # SettingsScreen (theme mode, sync status, about)
  navigation/                # Top-level destinations & adaptive nav bar / nav rail
```

## Milestones

- [x] **M0 — Toolchain & Plan:** Install `android` CLI, official Android skills,
      Rust Android targets (`aarch64-linux-android`, `x86_64-linux-android`),
      and `cargo-ndk`.
- [x] **M1 — Rust UniFFI Bridge (`crates/mobile-ffi`):** Expose `OtteroMobile`
      handle (SQLite `Store`, Notes CRUD/search/restore, Tasks CRUD/toggle/quick-add/restore,
      Habits check-in, Today summary) with Rust unit tests and portable `cargo-ndk`
      build script (`scripts/build-android-rust.sh`).
- [x] **M2 — Modular Jetpack Compose Shell (`apps/android`):** AGP 9 Compose
      project, Quiet Alabaster & Smoked Obsidian theme, edge-to-edge adaptive
      navigation (`Today`, `Notes`, `Tasks`, `Settings`), serialized `OtteroEngine`,
      Undo snackbars, and Senior Reviewer audit fixes. `app-debug.apk` builds cleanly.
- [ ] **M3 — Real-Device Verification & Vault:** Install & verify on physical
      Android device via `android` CLI / `adb` (light + dark, IME, predictive back,
      120Hz scroll), then add Passwords Vault screen + biometric unlock.
- [ ] **M4 — Cloud Sync & Biometric Unlock:** Share `AccountClient` + E2EE
      `SyncEngine` + WebSocket live push + Android Keystore biometric unlock +
      Android `AutofillService`.
