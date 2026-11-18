# Roadmap

The application features in phases 1–4 are implemented. Phase 5 includes release tooling and signed updater support; producing and publishing trusted releases still requires maintainer credentials and distribution setup.

> Activity tracking is the default. Optional Windows text recording requires encrypted storage and explicit selected-app or all-supported-app scope consent. Password-marked fields, inaccessible controls, and recognized terminal/password-manager apps are skipped; ordinary fields can still contain sensitive text. Clipboard, screenshot, and screen-content capture are outside the app's scope.

## Phase 1: Core tracker — implemented

- Active app, window title, and focus-duration tracking
- Per-app and local-day analytics
- App, date, and keyword filtering
- CSV and Markdown exports
- Configurable log path
- Plaintext JSONL and AES-256-GCM/Argon2id encrypted storage
- Sensitive-app/title exclusion blocklist
- Start on login, launch-window preference, and retention cleanup
- Pause/resume, clear logs, and system tray actions

## Phase 2: Idle detection and migration — implemented

- OS elapsed idle time on Windows, macOS, and Linux X11, without input hooks
- Configurable threshold, default 300 seconds, with an option to disable it
- Visible availability status; Wayland idle detection reports unavailable
- Idle time and suspended-machine gaps excluded from focus totals
- Plaintext/encrypted migration that merges destination history, preserves distinct events, and removes exact duplicates
- Staged writes and rollback if configuration commit fails; source removed only after success, with cleanup warnings if deletion fails
- Password verification for empty and cleared encrypted logs, and legacy encrypted-log compatibility

## Phase 3: Richer analytics — implemented

- Daily and Monday-based weekly views using local calendar boundaries
- Sessions crossing midnight split across the appropriate analytics buckets
- Local daily/weekly summary preview and text export
- App category assignments and category totals
- Clickable Dashboard/Log applications opening a dedicated details screen with recorded text and timestamped activity logs
- Per-app tracked/session/character totals, date/keyword filtering, separate text/activity pagination, and Back navigation that preserves source filters
- Daily/weekly goals and informational limits per app or category, with progress indicators
- Optional TLS SMTP summaries, omitting window titles and recorded text
- Scheduled delivery of the latest complete day/week after the selected local hour while the app is running and unlocked; no backlog replay
- SMTP passwords kept in memory for the app session and entered again after restart

## Phase 4: Search improvements — implemented

- In-memory cached history, avoiding repeated file reads and decryption
- Paginated newest-first queries without cloning or sorting the entire stored history for each page
- Saved filters
- Combined app/category/date/keyword filters
- Case-insensitive required terms, quoted phrases, and -excluded terms/phrases
- Exports covering all filtered matches, independently of the displayed page
- Recorded text participates in keyword search when present; exports omit its contents

## Optional text recording — implemented on Windows

- Separate default-off switch and exact, case-insensitive selected-app opt-ins
- Explicit all-supported-app mode, off by default, covering current and future apps while retaining field, process, encryption, pause, and exclusion checks
- Encrypted, unlocked storage required; metadata tracking remains usable without it
- Visible active-recording pill and persistent enabled notice
- Password-marked fields, inaccessible controls, and recognized terminal/password-manager apps excluded
- Pause, idle, and app/title exclusions stop text recording too
- Escaped plain text displayed in collapsed log disclosures and dedicated application-detail session cards
- Recorded text omitted from CSV/Markdown exports, summaries, and emails
- Delete-recorded-text action disables future recording and removes stored text while preserving activity times, apps, and window titles
- Plaintext migration rejected while recorded text remains
- macOS/Linux text recording and unsupported/custom Windows edit controls are not implemented

## Phase 5: Packaging and distribution — tooling implemented; external setup required

Implemented in the repository:

- macOS universal app and DMG release workflow, with Developer ID signing and notarization support when credentials are supplied
- Windows x64 MSI and NSIS installers, with Authenticode signing support when credentials are supplied
- Linux x64 AppImage and Debian packages
- winget manifest generation using the built installer's actual checksum
- Draft GitHub release creation
- Signed update artifacts and feed generation, embedded release public key/feed configuration, opt-in update checks, and explicit install/restart

Maintainer actions still required:

- Provision and configure macOS and Windows signing identities and Apple notarization credentials
- Generate and protect the updater signing key, configure its matching public key and HTTPS feed, and publish reviewed releases
- Validate generated winget manifests and submit them to the winget community repository; acceptance is external
- Build and verify release installers on their target platforms before distributing them

Local builds remain usable without these services. They do not acquire a trusted publisher identity or a live update feed automatically. Setup details are in [docs/BUILD.md](docs/BUILD.md).
