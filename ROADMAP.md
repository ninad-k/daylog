# Roadmap

daylog grows in add-on phases. The core tracker and reliability work are implemented. Remaining phases below are planned enhancements.

> Across all phases, daylog will never capture keystrokes, typed text, passwords, clipboard, or screen contents.

## Phase 1: Core tracker (done ✅)

- Active app, window title, and duration tracking via `active-win-pos-rs`
- Per-app and per-day analytics
- Search and filter by app, date, and keyword
- CSV and Markdown export
- Configurable log path
- Two storage backends behind a `Store` trait: plaintext JSONL (default) and optional AES-256-GCM with Argon2id-derived key
- Sensitive-app exclusion blocklist (pauses logging when a blocked app or title is focused)
- Start-on-login toggle (`tauri-plugin-autostart`)
- Auto-purge after N days
- Pause / resume and clear logs
- System tray with quick actions

## Phase 2: Idle detection and storage migration (implemented)

- OS idle timers on macOS, Windows, and Linux/X11, without input hooks
- Sleep and clock-gap protection
- Durable session checkpoints and atomic retention
- Cached history, live analytics, and visible storage failures
- Configurable idle threshold
- Migrate existing events between plaintext and encrypted storage when encryption changes
- Password verification for empty encrypted logs
- Backend and frontend regression tests, run in CI

## Phase 3: Richer analytics (planned)

- Dedicated daily and weekly summary views (current dashboard already supports today, 7 days, 30 days, and all time)
- Category tagging of apps (e.g. work, communication, entertainment)
- Time goals and limits per app or category

## Phase 4: Search improvements (planned)

- Indexed search for histories too large for the current in-memory cache
- Saved filters
- Combined filters across app, category, date range, and keyword

## Phase 5: Packaging and distribution (partially implemented)

- Manual CI workflow builds unsigned installer artifacts for all three desktop platforms
- macOS application bundle built locally; other platforms require their CI runners

- macOS: code signing and notarization
- Windows: MSI installer and winget package, code signing
- Linux: AppImage and `.deb` packages
- Auto-update
