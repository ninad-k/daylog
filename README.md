# daylog

**A transparent, privacy-respecting activity logger that shows where your time goes.**

![Rust](https://img.shields.io/badge/Rust-stable_1.90%2B-orange?logo=rust)
![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri)
![License: MIT](https://img.shields.io/badge/License-MIT-green)

> **Activity tracking by default:** daylog records the active application name, window title, start/end times, and duration. Optional **text recording on Windows** is off by default, requires encrypted storage, and uses your explicitly chosen scope: selected apps or **All supported apps**. It skips password-marked and inaccessible fields and recognized terminal/password-manager apps. Ordinary text fields can still contain sensitive information.
>
> daylog does not collect clipboard contents, screenshots, or screen contents. Idle detection reads only the OS's elapsed time since input.
>
> Activity stays on your device by default. Optional summary emails send app/category totals through your configured SMTP server; optional update checks contact the release feed. Both are off by default. There is no telemetry or daylog account.

## Features

- **Activity tracking** with pause/resume, exclusion blocklists, a system tray, and live session totals.
- **Optional text recording on Windows**, with a separate opt-in switch, selected-app or explicitly enabled all-supported-app scope, encrypted storage, a visible recording indicator, and searchable text collapsed beneath each log session. Recorded text is omitted from exports and summaries.
- **Idle detection** on Windows, macOS, and Linux X11. The default threshold is five minutes; change it or disable it in Settings. Unsupported sessions show that idle detection is unavailable.
- **Daily and weekly analytics**, app/category breakdowns, and local summary previews and text exports. Weeks begin Monday, and sessions crossing midnight contribute to each local day.
- **Application details**: click an app in the Dashboard or Log to open its recorded text and timestamped activity history, with date/keyword filters and separate text/activity pages.
- **App categories** such as Work, Communication, and Entertainment. Assign exact app names in Settings; other apps appear as Uncategorized.
- **Daily or weekly goals and limits** for an app or category, with progress and reached-limit indicators. Limits are informational and do not block applications.
- **Combined search** across app, category, date range, and keyword, with required words, quoted phrases, exclusions, saved filters, and pagination.
- **Cached history** avoids rereading or decrypting the entire log for each query. Events remain in memory only while the store is open.
- **CSV and Markdown exports** of the matching history, independent of the displayed page size.
- **Optional AES-256-GCM encryption** with an Argon2id-derived key. Your encryption password is never saved. Switching encryption modes migrates and merges existing history.
- **Configurable log path**, start on login, show window on launch, and automatic retention cleanup.
- **Optional daily or weekly email summaries**, sent over certificate-validated TLS without window titles or recorded text. SMTP passwords are held only for the current app session.
- **Optional signed update checks**, with explicit installation and restart. Release builds need a configured update feed and signing key.
- **Installer release workflow** for macOS app/DMG, Windows MSI/NSIS, and Linux AppImage/Debian packages, plus generated winget manifests. Publisher certificates, release publication, and winget submission require maintainer setup.
- **Plain HTML/CSS/JS frontend**, with no framework or npm build step.

## Build and run

daylog builds from source with Rust 1.90 or newer and the Tauri v2 CLI. Follow [docs/BUILD.md](docs/BUILD.md) for platform prerequisites, installer generation, signing, and update-feed setup.

~~~bash
git clone https://github.com/ninad-k/daylog.git
cd daylog
cargo install tauri-cli --version "^2"
cargo tauri dev
~~~

Use **cargo tauri build** for a release bundle. The release workflow creates draft GitHub releases; this repository does not establish a signed publisher identity or publish a winget package by itself.

## Getting started

1. Launch daylog. On macOS, grant the permissions in [docs/PERMISSIONS.md](docs/PERMISSIONS.md), then restart it.
2. Open Settings, add sensitive apps or window-title substrings to the exclusion blocklist, and enable exclusions. Consider enabling encryption before collecting sensitive history.
3. Adjust the idle threshold, assign app categories, and add daily or weekly goals and limits. Save Settings to apply your changes.
4. Use the Dashboard for current day/week totals and range-based breakdowns. Preview or save a daily/weekly summary locally.
5. Use the Log view to search, save filters, page through matches, or export the complete matching history.
6. Click an app name in the Log, a Dashboard app bar, or the top-app card to open that application's details. Use Back to return with the original view's filters intact.

### Search examples

Search is case-insensitive. All positive words or phrases must match the app name, window title, or recorded text, when present; excluded terms must be absent. Combine keyword search with the app, category, and date controls.

| Search | Matches |
| --- | --- |
| code daylog | Both words occur in the app name, title, or recorded text |
| "project review" | The phrase occurs together |
| daylog -meeting | Contains daylog and excludes meeting |
| "project review" -"private notes" | Includes the first phrase and excludes the second |

Saved filters retain the selected values, including specific dates. Previous/Next moves through matching sessions; exports include all matches for those filters.

### Application details

Click an application in the Dashboard or Log to open a dedicated screen for that exact app. It shows tracked time, activity-session totals, recorded-character totals, and the most recent matching activity. The **Recorded text** view displays existing text by session with its timestamp, duration, and window title. **Activity log** lists start/end times, duration, window title, and whether each session contains recorded text.

The details screen inherits the source view's dates and keyword, provides its own date/keyword filters, and keeps independent 50-session pages for recorded text and activity logs. **All history** clears only the details filters; **Back** returns to the source Dashboard or Log with its original filters and log page preserved. Use **Refresh** to load the latest stored or in-progress activity.

Opening details does not enable recording or change its scope. Text is available only when it was recorded with the optional text-recording feature; activity sessions without text remain visible in Activity log. Date filters select overlapping sessions, and recorded text remains grouped by its source session.

### Encryption and log migration

Enabling encryption migrates plaintext history into the sibling **.enc** file; disabling it migrates back to the configured base log file. If the destination already contains history, daylog merges distinct events and removes exact duplicates. It verifies the staged destination and saves the new configuration before deleting the source. A failure before configuration commit preserves the source and rolls back the destination.

A storage warning reports any old file that could not be removed after a successful migration. Check that warning before assuming all plaintext copies are gone. Normal deletion does not erase backups or guarantee forensic erasure of disk blocks. A base log path ending in **.enc** is rejected because it would collide with the encrypted destination. Moving to a new encrypted path requires the destination encryption password. Path, encryption, and other settings commit together after migration succeeds.

Logs containing recorded text cannot migrate to plaintext. Use **Delete recorded text…** in Settings first; it turns text recording off and removes recorded text while retaining activity times, app names, and window titles. Then turn encryption off if desired.

### Optional text recording

Text recording is supported on Windows and is disabled initially. Enable encryption, turn on text recording, and explicitly choose **Selected apps** or **All supported apps** in Settings, then use **Save settings**. Selected-app mode uses exact app names shown in daylog's header, ignoring capitalization; partial names and wildcards do not expand that list.

**All supported apps** includes current and future apps without adding each name. It still records only supported editable fields and keeps the password, sensitive-control, terminal/password-manager, pause, exclusion, and encryption checks. Selecting this broader mode is a separate explicit choice; it is off by default. An ordinary field in any included app can contain sensitive information.

A persistent notice shows that text recording is enabled and its scope, and a header indicator appears while the native recorder is active. Pause, idle, exclusions, or a locked encrypted log stop recording. The scope and selected-app list persist across restarts; encrypted logs start locked and record no text until unlocked.

Supported editable fields can record text, including sensitive content entered into an ordinary field. Password-marked fields, inaccessible controls, and recognized terminal/password-manager apps are skipped. Some editors, custom controls, input methods, and elevated windows may not be supported. This feature is not a complete document history or a reliable secret detector. Use selected-app mode or exclusions for apps where you do not want entered text retained.

The Windows recorder installs a keyboard hook only while an app in the saved recording scope is in the foreground, validates the focused editable control, and translates supported key presses to a typed character sequence. Backspace changes that recorded sequence; cursor movements are not reconstructed, so it can differ from the final document. It does not read the existing document or clipboard, and pasted content is not captured. Shortcuts, injected events, IME/AltGr input, unsupported providers, and stale or uncertain focus events may be skipped. See [docs/PERMISSIONS.md](docs/PERMISSIONS.md) for support limits.

Recorded text is searchable and displayed as escaped plain text inside a collapsed disclosure in the Log view, or as session cards in the application's Recorded text screen. CSV/Markdown exports, local summaries, and emails omit recorded text; summaries and emails also omit window titles. Turning recording off keeps existing text. Delete only recorded text with the Settings action, or use Clear logs to remove entire sessions. See [docs/PRIVACY.md](docs/PRIVACY.md).

### Summary emails

Emails are optional and disabled by default. Enter your SMTP host, sender, recipient, credentials, daily/weekly schedule, and local send hour in Settings. STARTTLS is required by default, usually on port 587; implicit TLS is also available, usually on port 465. SMTP passwords are never written to the configuration: enter the password again after restarting daylog when your server requires authentication.

While daylog is running and the log is unlocked, the scheduler sends the **previous complete local day or Monday-to-Monday week** once after the configured local hour. It catches up only the latest complete period rather than sending a backlog. The default hour is 09:00. “Send current summary now” sends the current, possibly incomplete period. Emails contain app, category, day, duration, and session totals; they omit window titles and recorded text.

### Updates

Automatic update checks are off by default. A release with an embedded HTTPS feed and public signing key supports manual checks and optional background checks. Checking does not upload your activity history. Installing an available update requires the **Install and restart** action, and update bundles must verify against the embedded signing key. Ordinary local builds have no feed until release signing is configured; see [docs/BUILD.md](docs/BUILD.md).

## Configuration

Settings are saved as JSON in the OS config directory. Encryption and SMTP passwords are excluded; text-recording scope and app selections, categories, goals, saved search terms, email addresses, and server settings are stored as readable configuration.

| Setting | Default |
| --- | --- |
| Log path | OS data directory, activity.log |
| Encryption | Off; plaintext JSONL |
| Text recording | Off; selected-app scope with an empty list; all-supported-app mode off; Windows and encryption required |
| Idle threshold | 300 seconds; blank disables it |
| Exclusions | Off; empty blocklist |
| App categories, goals, saved filters | Empty |
| Start on login | Off |
| Show window on launch | On |
| Auto-purge | Off; keep history |
| Poll interval | 1500 ms; accepted range 250–60000 ms |
| Summary emails | Off; daily at 09:00 when enabled |
| SMTP connection | Port 587, required STARTTLS |
| Automatic update checks | Off |

## Architecture

| Layer | Responsibility |
| --- | --- |
| capture/ | Active app/title, OS elapsed idle time, and optional Windows text recording |
| storage/ | Cached JSONL/AES-GCM stores, durable writes, transactional migration |
| core | Sessions, pause/exclusion/idle handling, retention, combined queries |
| insights | Local daily/weekly aggregation, categories, goal progress |
| reports | Local summary rendering and opt-in TLS SMTP delivery |
| updates | Opt-in signed update checks and explicit installation |
| commands | Tauri IPC bridge |
| ui/ | Static dashboard, log, and settings interface |

## Permissions and privacy

Windows needs no special capture permission. macOS requires Screen Recording for window titles and may prompt for Accessibility. Linux X11 supports active-window and idle detection; Wayland restricts window visibility and idle detection is reported unavailable. See [docs/PERMISSIONS.md](docs/PERMISSIONS.md).

Window titles can contain document names, webpage titles, email subjects, or contact names. Plaintext logs and exports are readable to anything with access to those files. Encryption protects the log at rest; exclusions prevent selected focus sessions from being written. Optional email sends summary data to the selected recipient and SMTP provider. Read [docs/PRIVACY.md](docs/PRIVACY.md) for the full data and network behavior.

## Roadmap and contributing

Tracking, idle detection, migration, richer analytics, summary emails, and search improvements are implemented. Release automation is provided; production signing identities, published update releases, and winget acceptance remain external setup. See [ROADMAP.md](ROADMAP.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

Clipboard, screenshot, and screen-content capture remain outside the app's scope. Text recording must remain explicit about its selected-app or all-supported-app scope, visible, and encrypted.

## License

MIT © 2026 Ninad K. See [LICENSE](LICENSE).

Repository: [ninad-k/daylog](https://github.com/ninad-k/daylog)
