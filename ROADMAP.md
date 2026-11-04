# Roadmap

daylog grows in add-on phases. Phase 1 is shipped; later phases are planned and not yet built.

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

## Phase 2: Idle detection and storage migration (planned)

- OS-level idle detection so time is not counted while the machine is idle or the user is away
- Configurable idle threshold
- Migrate existing events between plaintext and encrypted storage when the encryption toggle is flipped

## Phase 3: Richer analytics (planned)

- Daily and weekly summary views, plus optional summary emails
- Category tagging of apps (e.g. work, communication, entertainment)
- Time goals and limits per app or category

## Phase 4: Search improvements (planned)

- Faster, more flexible search over large logs
- Saved filters
- Combined filters across app, category, date range, and keyword

## Phase 5: Packaging and distribution (planned)

- macOS: code signing and notarization
- Windows: MSI installer and winget package, code signing
- Linux: AppImage and `.deb` packages
- Auto-update
