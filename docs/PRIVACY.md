# Privacy statement

daylog is a self-tracking tool that you run on your own machine. It is visible in the process list, has a normal window and a tray icon, and does what it says. This document states exactly what it does and does not do.

## What is collected

For each period of focus, daylog records:

- **Application name** (e.g. "Firefox")
- **Window title** (e.g. "Quarterly report.docx - Word")
- **Timestamps** (start and end)
- **Duration**

## What is NOT collected

- **Keystrokes** or key events. There is no keyboard hook.
- **Typed text**
- **Passwords**
- **Clipboard contents**
- **Screenshots** or any screen contents
- **Mouse** movement or clicks
- **Network** traffic or activity

On macOS, daylog needs the Screen Recording permission only because the OS requires it to expose window titles. daylog does not capture the screen.

## Local only

All data stays on your device. daylog makes **no network requests and sends no telemetry**. There are no accounts, no servers, and no analytics.

## Where files live

| Item | Location |
| --- | --- |
| Settings | OS config directory |
| Activity log | OS data directory by default, e.g. `~/Library/Application Support/com.ninadk.daylog/activity.log` on macOS (platform equivalents elsewhere) |

You can change the log path in Settings.

## Protecting your data

### Encryption

By default the log is **plaintext JSONL**, one JSON event per line. Anything that can read the file can read your history, including other processes running as your user, backup tools, and anyone with access to your disk.

Enable encryption in Settings to switch to the AES-256-GCM backend. The key is derived from your password with **Argon2id**. Only the **salt** is stored; the **password is never persisted**. If you forget the password, the encrypted log cannot be recovered.

### Exclude sensitive apps

Add apps or window-title patterns to the **exclusion blocklist** (password managers, banking, health portals, private chats). While a blocked app or title is focused, daylog **pauses logging** and writes nothing.

### Purge and clear

- **Auto-purge**: set a number of days; older events are deleted.
- **Pause / resume**: stop logging at any time from the window or tray.
- **Clear logs**: delete the entire log.
- You can also delete the log file directly.

## Honest caveats

- **Window titles can be sensitive.** A title can include a document name, a web page title, an email subject, or a chat contact. Even without any typed content, titles reveal a lot. Use the blocklist, encryption, and auto-purge accordingly.
- **The plaintext log is readable until you enable encryption.** Enabling encryption protects data written under that mode; see the [roadmap](../ROADMAP.md) for planned migration of existing events between modes.
- **Exports are not encrypted.** CSV and Markdown exports are plain files. Store them with care.
- **Disk-level threats remain.** Encryption protects the log file at rest. It does not protect against malware running while the app is unlocked.

## Non-goal

Capturing keystrokes or typed text is an explicit non-goal. Contributions adding it will not be accepted. See [CONTRIBUTING.md](../CONTRIBUTING.md).
