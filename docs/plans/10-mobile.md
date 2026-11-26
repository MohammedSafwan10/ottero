# 10 Mobile — Android First, iOS Later (Phased Master Plan)

Status: **M0–M2 Complete & Verified on Physical Hardware** (`CPH2603` / Android 16 / `arm64-v8a`).
Next up: **Phase 3 (Navigation Architecture, Bug Fixes, Onboarding & Quiet Loop Icon)**.

---

## 1. Architecture & Repo Boundary

- **UI:** Native **Jetpack Compose (Material 3, AGP 9, Kotlin)** for Android first; **SwiftUI** for iOS later.
- **Core:** 100% shared Rust core (`ottero-core`, `ottero-storage`, `ottero-crypto`, `ottero-sync-protocol`) bridged via **Mozilla UniFFI** (`crates/mobile-ffi`, compiled with `cargo-ndk` + NDK 28 for 16 KB page alignment on Android 15/16).
- **Monorepo & Open Source Boundary:** `crates/mobile-ffi` and `apps/android` live in the public `ottero` repo (`GPL-3.0-only`). Cloud backend services stay strictly in the private `ottero-cloud` repo.
- **Release Bundle Target:** ~4–6 MB per-ABI (`arm64-v8a`) with Rust `--release` (`lto = true`, `strip = true`, `opt-level = "z"`) + Android R8 (`isMinifyEnabled = true`, `isShrinkResources = true`).

---

## 2. Information Architecture & Navigation (Scalable to All Modules)

Ottero is an all-in-one personal workspace (Today, Notes, Tasks, Calendar, Habits, Focus, Passwords Vault, Developer, and later Finance). A flat bottom bar with Settings does **not** scale past 4 destinations.

### Recommended Hybrid IA (Linear / Things / Obsidian / Notion Pattern)

1. **Bottom Navigation Bar (Compact Phones `< 600dp`) — 4 Calm Daily Drivers Only:**
   - **Today** (Home hub: Greeting, Date, Schedule, Due tasks, Habits, Recent notes, Quick-capture `+`)
   - **Notes** (Search, Pinned, Tags, Markdown editor)
   - **Tasks** (Today / Upcoming / All / Done, Projects, Quick-add)
   - **Calendar** *(or user-customizable 4th slot, e.g. Passwords Vault)*
   - **Never put Settings in the bottom bar.**
2. **Top Header Chrome (Calm, Zero Noise):**
   - **Top-left:** Drawer menu button (`≡`) to open the **Workspace Sidebar (`ModalNavigationDrawer`)**.
   - **Top-right:** Search icon (global command/quick-open) + **Account / Settings avatar button** (opens full-screen Settings & Account).
3. **Workspace Sidebar (`ModalNavigationDrawer` on Phone / Persistent `NavigationRail` on Tablet & Foldable):**
   - Mirrors Desktop's customizable sidebar order:
     - **Daily:** Today, Notes, Tasks, Calendar
     - **Life:** Habits, Focus Timer, Passwords Vault
     - **Work:** Developer *(optional toggle, hidden by default)*
     - **Footer:** Trash, Sync status pill, Settings
   - Swipe from left edge or tap `≡` on phones; always visible rail on `>= 600dp`.
4. **Navigation Engine & State Ownership:**
   - Migrate from raw `when (tab)` conditionals to **Navigation 3 (`androidx.navigation3`)** with a real typed backstack (`NavBackStack`), animated transitions, and native Android 15/16 predictive back.
   - Move screen state & coroutines from composable `LaunchedEffect` into **`ViewModel` (`viewModelScope`)** + a `runSuspendCatching` helper that **always rethrows `CancellationException`** (fixes the *"The coroutine scope left the composition"* toast during rapid tab switching and preserves scroll position across tabs).

---

## 3. Design System & Craft Standards

- **House Themes (100% Desktop Parity from `crates/ui/themes/ottero.json`):**
  - **Ottero Light ("Quiet Alabaster"):** `#FAF8F5` canvas, `#F0EDE7` recessed, `#FFFFFF` elevated cards, `#1C1B1A` charcoal-ink primary CTA, muted sage (`#7E9A84`), ochre (`#B89356`), slate (`#697D91`).
  - **Ottero Dark ("Smoked Obsidian"):** `#131417` canvas, `#0E0F11` recessed, `#1A1C20` elevated cards, `#F3F4F6` crisp primary CTA, muted sage (`#7C9A82`), sand (`#C2A26C`), steel (`#6A829E`).
- **Unslop UI Discipline:** Zero decorative badges, zero filler subtitles, zero AI gradients/glows, strict 48 dp touch targets, dynamic edge-to-edge `SystemBarStyle`, 120Hz zero-jank scrolling.
- **Brand Identity:** Replace default Android launcher icon with Ottero's **Quiet Loop** adaptive icon (`assets/brand/source/icon-master.png`) + Android 13+ monochrome themed icon layer + Android 12+ SplashScreen API (`androidx.core:core-splashscreen`).

---

## 4. Phased Delivery Roadmap

### Phase 0–2 — Foundation (Done)
- [x] **M0 — Toolchain & Skills:** `android` CLI, SDK 36, NDK 28, `cargo-ndk`, official Google Android skills.
- [x] **M1 — Rust UniFFI Bridge (`crates/mobile-ffi`):** SQLite `Store`, Notes, Tasks, Habits, Today, Undo restore, sanitized errors, portable `scripts/build-android-rust.sh`.
- [x] **M2 — Compose Shell (`apps/android`):** Quiet Alabaster & Smoked Obsidian themes, async off-main-thread startup, serialized `OtteroEngine`, predictive `BackHandler`, `ON_STOP` flush, Undo snackbars, Android 12+ backup exclusions. Verified live on OnePlus Android 16 (`arm64-v8a`).

### Phase 3 — Core UX Polish, Scalable Navigation & Onboarding (Done)
- [x] **Fix `CancellationException` leak:** Added `runSuspendCatching` (rethrows `CancellationException`) + global `OtteroFeedbackController` (`OtteroSnackbarHost` + `OtteroConfirmDialogHost`) so rapid tab switching never triggers false error toasts.
- [x] **Scalable Hybrid Navigation:**
  - Moved **Settings** out of the bottom bar to the **top-right header icon** + Drawer footer (full-screen route with predictive `BackHandler`).
  - Added **`ModalNavigationDrawer`** (phone) / persistent **`NavigationRail`** (tablet/foldable) + 4 daily bottom tabs (`Today`, `Notes`, `Tasks`, `Habits`).
- [x] **First-Run Onboarding & Actionable Empty States:**
  - Calm first-launch welcome (`OnboardingScreen.kt`): live theme picker (`System / Light / Dark`), **"Start with sample workspace"** (seeds starter Note, Tasks, Habit) or **"Start blank"**, plus actionable empty-state CTAs.
- [x] **App Icon & Splash Screen:**
  - Generated **Quiet Loop** Android Adaptive Icon + all 5 legacy mipmap densities from `assets/brand/source/icon-master.png`, plus Light/Dark Android 12+ Splash Screen themes (`values-v31` / `values-night-v31`).
- [x] **Universal Quick-Capture Sheet (`+`):**
  - Floating `+` opens `QuickCaptureSheet` for 1-tap **Task** (Rust quick-add), **Note**, or **Habit** creation.

### Phase 4 — Full Feature Parity (Offline-First)
- [ ] **Rich Notes Experience:**
  - Markdown formatting toolbar above the keyboard (Bold, Italic, Heading, Checklist, Code, `[[Wiki Link]]` autocomplete), Tags filter bar, Pinned section, and Backlinks sheet.
  - Note Trash screen with Restore & Permanent Delete confirmation.
- [ ] **Tasks & Projects:**
  - Task detail bottom sheet (due date picker, priority selector, recurrence picker, project selector, notes, subtasks).
  - Swipe actions on task rows (swipe right = complete, swipe left = trash with Undo).
- [ ] **Habits Module:**
  - Dedicated Habits screen + Today habit cards with quick-add (`Read 20 pages daily`, `Gym mon wed fri`), streak count, and weekly consistency grid.
- [ ] **Calendar Module:**
  - Agenda + Month view sharing Rust `Event` expansion (repeating events, exceptions, birthdays, holidays) and quick-add event parser.
- [ ] **Focus / Pomodoro Timer:**
  - Foreground service notification with live countdown, task-linked focus sessions, and haptics on phase completion.

### Phase 5 — Passwords Vault & Native Biometric Security
- [ ] **Vault Bridge (`crates/mobile-ffi/src/vault.rs`):**
  - Expose `ottero-crypto` + `Store` vault setup, unlock, lock, item CRUD, TOTP live 6-digit code generator, password generator, and auto-lock timeout.
- [ ] **Android Biometric Unlock:**
  - `BiometricPrompt` + Android Keystore (`AES-GCM` hardware-backed key wrapping) for instant fingerprint/face unlock, with master-password / Ottero account fallback.
- [ ] **FLAG_SECURE & Clipboard Hygiene:**
  - Set `WindowManager.LayoutParams.FLAG_SECURE` while Passwords Vault is open (blocks screenshots & recents preview) and mark copied passwords `ClipDescription.EXTRA_IS_SENSITIVE = true` with 30s auto-clear.
- [ ] **Android System `AutofillService`:**
  - Native OS password & TOTP autofill across Chrome and all Android apps, calling `crates/mobile-ffi` directly in-process.

### Phase 6 — Ottero Cloud Account & Live E2EE Sync
- [ ] **Shared Rust Sync Engine (`crates/mobile-ffi/src/sync.rs`):**
  - Expose Account sign-in (email + password + Secret Key + TOTP 2FA), Emergency Kit export, E2EE pull/push (`crates/sync-protocol`), and WebSocket live push.
  - Store session tokens in hardware-backed EncryptedSharedPreferences / Android Keystore (never plain prefs).
- [ ] **Background Sync (`WorkManager`):**
  - Periodic + connectivity-aware background sync + instant foreground sync on app resume, so edits made on Desktop appear on Android in <1 second and vice versa.

### Phase 7 — Native Mobile Superpowers & Release Engineering
- [ ] **Home Screen Widgets (`Jetpack Glance`):**
  - **Today Tasks** interactive checkbox widget, **Quick-Add** 1-tap widget, and **Habits** tap-to-log widget.
- [ ] **Android Share Sheet Target (`ACTION_SEND`):**
  - Share URLs, text, or snippets from any Android app directly into a new Ottero Note or Task.
- [ ] **Release Optimization & Signing:**
  - Enable R8 (`isMinifyEnabled = true`, `isShrinkResources = true`), Rust `--release` LTO/strip profile, and split APKs / Android App Bundle (`.aab`) targeting **~4–6 MB** download size.
  - CI workflow (`.github/workflows/android-release.yml`) for signed GitHub Releases & Play Store internal track.
- [ ] **iOS SwiftUI Companion (`apps/ios`):**
  - Reuse 100% of `crates/mobile-ffi` via `uniffi-bindgen --language swift` + `XCFramework` to build native SwiftUI + Face ID + `ASCredentialProviderViewController` + `WidgetKit`.
