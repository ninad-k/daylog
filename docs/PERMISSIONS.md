# Permissions

daylog reads the active application name and window title. Optional idle detection reads only the OS's elapsed time since input. Optional Windows text recording is a separate, default-off feature that requires an encrypted log and explicit selected-app or all-supported-app scope consent. The text recorder uses keyboard events only while an app in the saved recording scope is in the foreground. No feature reads the clipboard or captures screenshots or screen contents.

## macOS

macOS gates other applications' window titles behind Screen Recording permission. daylog uses the permission to read titles; it does not record the screen. Active-window detection may also prompt for Accessibility.

### Screen Recording

1. Open System Settings → Privacy & Security → Screen Recording.
2. Enable daylog. If it is not listed, launch it once to trigger the request, or add the app.
3. Quit daylog completely, including its tray/menu-bar process, and restart it.

If macOS prompts for Accessibility, enable daylog in System Settings → Privacy & Security → Accessibility, then restart.

Without title permission, the OS may return empty titles or only application names. During development, the permission entry may belong to the terminal, IDE, or development binary. Rebuilt unsigned applications can require permission to be granted again.

Idle detection uses Quartz's elapsed idle-time query. It reads a duration and does not require daylog to install an input hook or collect key/button identities.

## Windows

Active-window and idle detection require no special permission. Idle detection uses GetLastInputInfo to read the session's last-input timestamp, without receiving the input itself.

Elevated application windows may not be readable by a non-elevated daylog process. An unsigned installer or executable can trigger SmartScreen or generic reputation warnings. Publisher signing is supported by the release workflow when a maintainer supplies a valid certificate; a local build is not automatically signed.

### Optional text recording on Windows

Enable encrypted storage, turn on the separate text-recording switch in Settings, and explicitly choose **Selected apps** or **All supported apps**. Selected-app mode uses exact names shown in daylog's header, ignoring capitalization. All-supported-app mode is off by default and includes future apps without adding them to the list. It still records only supported editable fields and retains every password, sensitive-control, process, pause, exclusion, and encryption guard. The preference persists, but recording stops while the encrypted log is locked and requires unlocking again after each launch. A persistent enabled notice and an active-recording indicator make the state visible.

The recorder installs a Windows low-level keyboard hook only while an app in the saved recording scope is in the foreground. It validates the focused control through Windows UI Automation at keydown and checks its identity again before recording. Eligible controls must expose editable, enabled, focused, non-password metadata. Password-marked fields, inaccessible controls, recognized terminals/password managers, daylog itself, and recognized remote-desktop/viewer apps are skipped. Use selected-app mode or exclusions for apps whose entered text should not be retained: an ordinary field can contain a password, token, or other secret that field metadata cannot identify.

The recorder translates supported key presses using the foreground keyboard layout into a typed character sequence. Backspace edits that sequence; cursor movement is not reconstructed, so the result can differ from the final document. It does not read the current document or clipboard, and pasted content is not captured. Ctrl/Alt/Win shortcuts, injected events, nontext navigation, stale events, and uncertain focus changes are dropped. IME input, AltGr, complex composition, custom controls, elevated applications, or inaccessible edit providers may be skipped. Queues and event age are bounded; a stalled accessibility provider or removal of the hook by Windows can interrupt recording. The feature is therefore not a complete document history.

Text recording is currently unavailable on macOS and Linux; their normal focus/idle tracking remains available. Windows builds need no elevation for supported ordinary apps. No administrator-access workaround is provided for protected controls.

Recorded text remains encrypted. CSV/Markdown exports, summaries, and emails omit it. Before disabling encryption, use **Delete recorded text…**, which turns recording off and removes text while preserving activity times, app names, and window titles. Plaintext migration is rejected while recorded text remains.

## Linux

Active-window and idle detection work on X11. Idle detection uses the X11 Screen Saver extension and requires libXss; build dependencies are listed in [BUILD.md](BUILD.md).

Wayland restricts one client from inspecting other clients' windows. Window titles or focus data may be missing, depending on the compositor and application. Log into an X11 session for full support.

Idle detection deliberately reports unavailable under Wayland, including sessions with XWayland. XWayland's idle clock does not cover native Wayland applications, so using it would incorrectly discard time spent in them. When idle detection is unavailable, daylog shows that status and uses focus tracking alone.

Tray icons require libayatana-appindicator3 and a desktop environment with status-notifier support.

## Idle settings

The default idle threshold is 300 seconds. Settings accepts 15–86400 seconds; leave the field blank to disable detection. The app reports whether the OS idle query is available in the current session.

When the threshold is reached, time since the last input is excluded from focus totals. Long reading or video playback without input can count as idle. Pausing logging remains available regardless of OS idle support.

## Optional network features

Ordinary tracking requires no network access. Summary emails and update checks are separate, optional features, both off by default.

- Summary emails need access to your configured SMTP host and port. STARTTLS or implicit TLS with certificate validation is required. Re-enter the SMTP password after restarting when your server requires authentication.
- Update checks need access to the embedded HTTPS feed, and installation downloads a signed update after an explicit action. Release signing/feed setup is described in [BUILD.md](BUILD.md).
- Firewall or organization network rules may block these connections. Neither feature requires administrator access merely to read activity data.

## Platform overview

| OS | Window-title permission | Idle detection | Main limitation |
| --- | --- | --- | --- |
| macOS | Screen Recording; Accessibility if prompted | Quartz elapsed idle time | Restart after granting permissions |
| Windows | None | GetLastInputInfo | Elevated windows may be unreadable |
| Linux X11 | None | X Screen Saver extension | libXss and tray dependencies |
| Linux Wayland | Compositor-dependent restrictions | Unavailable | Partial or unavailable focus/title data |
