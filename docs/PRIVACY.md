# Privacy statement

daylog is a self-tracking tool that you run on your own machine. It has a normal window, appears in the process list, and provides a tray icon. Activity storage is local by default; optional summary emails and update checks are described below.

## Activity data

For each focus session, daylog records:

- Application name
- Window title
- Start and end timestamps
- Duration
- Optional recorded text, only on Windows after enabling encrypted storage and explicitly choosing selected-app or all-supported-app scope

Window titles may contain document names, webpage titles, email subjects, contact names, or other sensitive information. An app/category assignment, a saved search term, or a goal name can also reveal information about your work.

Ordinary activity tracking does not record entered text. Optional text recording is described below. daylog does not collect clipboard contents, screenshots, screen contents, mouse coordinates, or network traffic.

## Optional text recording

Text recording is off by default and requires Windows, an encrypted and unlocked log, the separate text-recording switch, and an explicitly chosen scope. Selected-app mode uses an exact app list, ignoring capitalization; partial names and wildcards do not expand it. **All supported apps** is a separate explicit choice, off by default, that includes current and future apps without listing each name. Both modes retain field, sensitive-context, process, pause, exclusion, and encryption checks. The scope, list, and enabled preference are saved as readable configuration and persist across restarts, but the encrypted log starts locked and no text is recorded until unlocked.

When active in a supported editable control, the recorder stores entered text with the focus session. It skips password-marked fields, inaccessible edit controls, and recognized terminal/password-manager apps. These checks cannot identify every secret: a password, token, personal message, or other sensitive content entered into an ordinary field can be recorded. Some custom editors, input methods, and elevated windows may be unsupported. Recorded text is not a complete document history.

The Windows implementation installs a low-level keyboard hook only while an app in the saved recording scope is in the foreground. It translates supported key presses using that app's keyboard layout, validates the focused editable control with Windows UI Automation, and checks the control identity again before recording. It records a typed character sequence: Backspace changes that sequence, but cursor movements are not reconstructed. The result can differ from the final document. It does not read existing document contents or capture pasted content. Injected events, shortcuts, unsupported or password-marked controls, and stale focus events are dropped. IME, AltGr, complex composition, custom editors, or slow accessibility providers can produce incomplete or absent recorded text.

A persistent notice appears while the feature is enabled. A separate header indicator appears while the native recorder reports that it is recording. Pause, idle, exclusions, or a locked log stop recording. Use selected-app mode or exclusions for apps whose supported entered text should not be retained. All-supported-app mode broadens this exposure to future apps too; it does not make ordinary fields free of sensitive content.

Recorded text is stored only in encrypted logs and is held in memory while the log is unlocked. The Log view shows it as escaped plain text inside a collapsed disclosure, and keyword search can match it. A dedicated application-details screen can display the same existing text as expanded session cards. CSV and Markdown exports omit recorded text. Local summaries and summary emails omit recorded text and window titles.

Turning the text-recording switch off prevents future recording and keeps existing text. **Delete recorded text…** turns recording off and removes text from stored and in-progress sessions while preserving app names, titles, and activity times. Clear logs removes entire sessions. Logs containing recorded text cannot be migrated to plaintext: delete recorded text first, then disable encryption if desired. Neither action removes external backups or copies already made by another process.

## Reviewing an application's history

Clicking an app in the Dashboard or Log opens details for that exact application. Recorded text appears grouped by session and window, with timestamps and duration. Activity log lists app-focus sessions, including sessions without text, with start/end times and window titles. Viewing them does not enable text recording, change its scope or app list, or collect new types of data.

Dates and keywords filter the details, and recorded text/activity lists have separate pages. Date filters select overlapping sessions; each text entry remains its source session's recorded sequence. Back returns to the original view without changing its filters. The details text panel is cleared when leaving the screen or when the encrypted log becomes locked. Already recorded text can be viewed while recording is off, provided the encrypted log is unlocked.

## Idle detection

The app reads the OS's elapsed time since the last input to decide when to stop counting focus time. It does not receive the input itself or record which key or mouse button was used. Elapsed idle samples are not written to the activity log.

Idle detection is available on Windows, macOS, and Linux X11. The default threshold is 300 seconds; change it or leave the setting blank to disable it. Once the threshold is reached, time after the last input is removed from focus totals. Reading or watching a video without input can therefore be treated as idle.

On unsupported sessions, including Wayland, idle detection reports unavailable and tracking uses window focus alone. A large capture gap, such as system suspend, does not become hours of focus time.

## Local storage and settings

| Item | Location and contents |
| --- | --- |
| Settings | JSON in the OS config directory; paths, exclusions, text-recording scope and app opt-ins, categories, goals, saved filters, SMTP settings and addresses, email schedule/last-sent marker, and update preference |
| Activity log | OS data directory by default, using activity.log for plaintext and its sibling activity.enc for encryption |
| Cached history | Decrypted events in process memory while the store is open |
| Exports | The file destination selected by the user |

Settings are readable JSON even when activity-log encryption is enabled. Saved keyword filters, app/category names, and goal labels are not encrypted. Encryption passwords and SMTP passwords are not saved in the configuration.

## Encryption and migration

Plaintext JSONL is the default. Anything with permission to read that file can read its history, including processes running as your user, backup tools, and anyone with access to an unprotected disk.

Optional encryption uses AES-256-GCM with an Argon2id-derived key. The encrypted header stores a salt, key-derivation parameters, and an encrypted password verifier; it does not store your password or derived key. The derived key is held in memory while the log is unlocked. A wrong password is rejected even when the encrypted log is empty or has been cleared. Legacy encrypted logs are supported and upgraded after their records validate. There is no password recovery.

Switching between plaintext and encryption migrates existing events and merges any history already at the destination, removing exact duplicates while preserving distinct sessions. The destination is staged and verified before the new configuration is committed. A failure before commit retains the source and rolls back the destination. The old source is deleted only after success. The same process preserves history when moving a log through Settings. Plaintext migration is rejected while recorded text remains in the history.

If an old source or temporary backup cannot be deleted after a successful migration, a storage warning identifies it. An undeleted plaintext file remains readable. Normal file deletion does not securely erase disk blocks, remove backups, or delete copies made by other tools. Encryption protects the active log at rest; it does not encrypt earlier backups or exported files.

## Exclusions and retention

Add app names or window-title substrings to the exclusion blocklist and enable exclusions. Matching is case-insensitive. No focus session is recorded while an excluded app or title is focused; the current app/title can still be displayed transiently in the running app.

Pause/resume stops and restarts capture. Automatic retention deletes events older than the chosen number of days. Clear logs empties the active store and discards its in-progress/pending sessions. These actions do not remove exports, backups, emailed reports, or old files identified by an earlier cleanup warning.

## Optional summary emails

Summary emails are off by default. Enabling them sends reports through the SMTP server and to the recipient you configure. Reports contain app names, category names, local day totals, durations, and session counts. They omit window titles, recorded text, and individual session records.

SMTP connections require certificate-validated TLS, using STARTTLS or implicit TLS. Plaintext SMTP is unavailable. The SMTP password is held in memory for the current app session and is not persisted; enter it again after restart if authentication is required. SMTP hostname, port, username, sender/recipient addresses, and schedule are saved as readable settings.

Scheduled mail sends the latest complete local day or Monday-based week after the selected local hour while daylog is running and the activity log is unlocked. Only the most recent complete period is caught up. The manual send action sends the current, potentially incomplete period.

Your configured SMTP provider and recipient receive the summary content and normal email metadata. They may retain their own copies. Activity-log encryption does not protect a summary after it is sent.

## Optional update requests

Automatic update checks are off by default. Manual checks and enabled background checks contact the HTTPS release feed embedded in a configured release build. They exchange update metadata; daylog does not upload activity events, window titles, categories, goals, or saved filters to the feed.

The feed/download host can observe normal connection information such as the requesting IP address. Installing an update requires the explicit Install and restart action. Downloaded update bundles must verify against the public signing key embedded in the app. Local builds without a configured feed cannot check for updates.

## Telemetry and limitations

There is no telemetry, daylog account, remote activity dashboard, or automatic activity-log upload. With email and automatic update checks disabled, ordinary tracking makes no network requests. Clicking a manual update check or enabling email deliberately initiates the corresponding network activity.

CSV, Markdown, and text summary exports are plain files. Encryption protects a closed log on disk, not activity already displayed or cached in an unlocked process. Malware with access to the running process or your account may still access sensitive data.

Optional text recording must remain explicit about its selected-app or all-supported-app scope, visible, and encrypted. Clipboard, screenshot, and screen-content capture remain outside the app's scope. See [CONTRIBUTING.md](../CONTRIBUTING.md).
