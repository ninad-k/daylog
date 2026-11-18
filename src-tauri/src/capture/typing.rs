//! Explicitly opted-in text capture. Nothing in this module writes to disk.
//!
//! On Windows the hook exists only while an opted-in eligible app is foreground. Both
//! the hook and the worker require a writable, non-password UI Automation
//! control. Providers that cannot prove those properties are skipped. Clipboard,
//! existing control values, screenshots, and key-up events are never recorded.

use crate::SharedCore;

pub const fn supported() -> bool {
    cfg!(target_os = "windows")
}

pub fn active() -> bool {
    #[cfg(target_os = "windows")]
    {
        platform::ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn spawn(core: SharedCore) {
    #[cfg(target_os = "windows")]
    platform::spawn(core);
    #[cfg(not(target_os = "windows"))]
    let _ = core;
}

#[cfg(any(target_os = "windows", test))]
fn denied_executable(path: &str) -> bool {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    [
        "cmd",
        "powershell",
        "pwsh",
        "windowsterminal",
        "wt",
        "conhost",
        "openconsole",
        "terminal",
        "wezterm",
        "alacritty",
        "kitty",
        "mintty",
        "putty",
        "ssh",
        "securecrt",
        "hyper",
        "tabby",
        "cmder",
        "conemu",
        "conemu64",
        "terminus",
        "wsl",
        "bash",
        "1password",
        "bitwarden",
        "keepass",
        "keepassxc",
        "dashlane",
        "enpass",
        "nordpass",
        "roboform",
        "lastpass",
        "keeper",
        "daylog",
        "credentialuibroker",
        "consent",
        "logonui",
        "lockapp",
        "winlogon",
        "sechealthui",
        "securityhealthsystray",
        "securityhealthhost",
        "securityhealthservice",
        "smartscreen",
        "credwiz",
        "mstsc",
        "msrdc",
        "anydesk",
        "teamviewer",
        "rustdesk",
        "virtualboxvm",
        "vmware",
    ]
    .contains(&name)
}

#[cfg(any(target_os = "windows", test))]
fn sensitive_context(metadata: &str) -> bool {
    // Strip punctuation so CSS/automation identifiers and human-readable
    // accessibility labels share the same conservative matching rules.
    let compact: String = metadata
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "terminal",
        "console",
        "xterm",
        "password",
        "passphrase",
        "passcode",
        "credential",
        "token",
        "secret",
        "privatekey",
        "pinentry",
        "secureentry",
        "sudo",
        "sshsession",
        "seedphrase",
        "recoveryphrase",
        "recoverycode",
        "onetimecode",
        "securitycode",
        "remotedesktop",
        "virtualmachine",
        "remoteconsole",
        "vmware",
        "virtualbox",
        "novnc",
        "citrix",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
}

#[cfg(any(target_os = "windows", test))]
fn text_key(vk: u32) -> bool {
    matches!(vk, 0x08 | 0x09 | 0x0d | 0x20 | 0x30..=0x39 | 0x41..=0x5a | 0x60..=0x6f | 0xba..=0xc0 | 0xdb..=0xdf | 0xe2)
}

#[cfg(any(target_os = "windows", test))]
fn shortcut(state: &[u8; 256]) -> bool {
    // AltGr deliberately follows the same rule as Ctrl+Alt. Dropping characters
    // is safer than treating command shortcuts as ordinary typed text.
    [0x11, 0x12, 0x5b, 0x5c, 0xa2, 0xa3, 0xa4, 0xa5]
        .iter()
        .any(|&key| state[key] & 0x80 != 0)
}

#[cfg(any(target_os = "windows", test))]
fn safe_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\u{8}'))
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{denied_executable, safe_text, sensitive_context, shortcut, text_key};
    use crate::{
        capture::window::{self, Focus},
        SharedCore,
    };
    use chrono::{DateTime, Utc};
    use std::{
        cell::RefCell,
        mem::size_of,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver, SyncSender},
        },
        time::{Duration, Instant},
    };
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            Globalization::{NormalizationC, NormalizationD, NormalizeString},
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_MULTITHREADED, SAFEARRAY,
                },
                LibraryLoader::GetModuleHandleW,
                Ole::{
                    SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetElemsize,
                    SafeArrayGetLBound, SafeArrayGetUBound,
                },
                Threading::{
                    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
                    PROCESS_QUERY_LIMITED_INFORMATION,
                },
                Variant::{VariantClear, VARIANT, VT_BOOL},
            },
            UI::{
                Accessibility::{
                    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
                    IUIAutomationValuePattern, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
                    UIA_IsReadOnlyAttributeId, UIA_TextPatternId, UIA_ValuePatternId,
                    UIA_WindowControlTypeId,
                },
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetKeyboardState,
                    ToUnicodeEx, HKL,
                },
                WindowsAndMessaging::{
                    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetGUIThreadInfo,
                    GetMessageW, GetWindowTextW, GetWindowThreadProcessId, KillTimer, SetTimer,
                    SetWindowsHookExW, UnhookWindowsHookEx, GUITHREADINFO, HHOOK, KBDLLHOOKSTRUCT,
                    LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
                    WM_TIMER,
                },
            },
        },
    };

    pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
    const QUEUE_CAPACITY: usize = 128;
    const MAX_EVENT_AGE: Duration = Duration::from_millis(400);

    struct ComApartment;
    impl ComApartment {
        fn new() -> Option<Self> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
            }
            Some(Self)
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct Array(*mut SAFEARRAY);
    impl Drop for Array {
        fn drop(&mut self) {
            unsafe {
                let _ = SafeArrayDestroy(self.0);
            }
        }
    }

    struct Attribute(VARIANT);
    impl Drop for Attribute {
        fn drop(&mut self) {
            unsafe {
                let _ = VariantClear(&mut self.0);
            }
        }
    }

    #[derive(Clone)]
    struct ApprovedWindow {
        hwnd: usize,
        pid: u32,
        thread: u32,
        focus: Focus,
    }

    struct KeyEvent {
        window: ApprovedWindow,
        control: Vec<i32>,
        keyboard: [u8; 256],
        vk: u32,
        scan: u32,
        layout: usize,
        focus_hwnd: usize,
        created: Instant,
        time: DateTime<Utc>,
    }

    struct HookContext {
        core: SharedCore,
        sender: SyncSender<KeyEvent>,
        automation: IUIAutomation,
        approved: Option<ApprovedWindow>,
        hook: Option<HHOOK>,
    }
    thread_local! { static CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) }; }

    fn process_name(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut path = [0u16; 32768];
            let mut length = path.len() as u32;
            let result = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(path.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(handle);
            result.ok()?;
            String::from_utf16(&path[..length as usize]).ok()
        }
    }

    fn foreground() -> Option<(usize, u32, u32)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() {
                return None;
            }
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            (pid != 0 && thread != 0).then_some((hwnd.0 as usize, pid, thread))
        }
    }

    fn title(hwnd: usize) -> Option<String> {
        let mut buffer = [0u16; 8192];
        let count = unsafe { GetWindowTextW(HWND(hwnd as *mut _), &mut buffer) };
        if count <= 0 {
            return None;
        }
        String::from_utf16(&buffer[..count as usize]).ok()
    }

    fn focus_handle(thread: u32) -> Option<usize> {
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetGUIThreadInfo(thread, &mut info).ok()?;
        }
        (!info.hwndFocus.is_invalid()).then_some(info.hwndFocus.0 as usize)
    }

    fn runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
        unsafe {
            let array = Array(element.GetRuntimeId().ok()?);
            if array.0.is_null()
                || SafeArrayGetDim(array.0) != 1
                || SafeArrayGetElemsize(array.0) != 4
            {
                return None;
            }
            let low = SafeArrayGetLBound(array.0, 1).ok()?;
            let high = SafeArrayGetUBound(array.0, 1).ok()?;
            if high < low || high.checked_sub(low)? > 31 {
                return None;
            }
            let mut result = Vec::new();
            for index in low..=high {
                let mut value = 0i32;
                SafeArrayGetElement(array.0, &index, (&mut value as *mut i32).cast()).ok()?;
                result.push(value);
            }
            Some(result)
        }
    }

    /// Never request CurrentValue or TextPattern contents. Accessibility labels
    /// are inspected transiently for sensitive contexts and never join events.
    fn safe_control(automation: &IUIAutomation, pid: u32, hwnd: usize) -> Option<Vec<i32>> {
        unsafe {
            let element = automation.GetFocusedElement().ok()?;
            if element.CurrentProcessId().ok()? != pid as i32
                || element.CurrentIsPassword().ok()?.as_bool()
                || !element.CurrentHasKeyboardFocus().ok()?.as_bool()
                || !element.CurrentIsKeyboardFocusable().ok()?.as_bool()
                || !element.CurrentIsEnabled().ok()?.as_bool()
            {
                return None;
            }
            let kind = element.CurrentControlType().ok()?;
            if kind != UIA_EditControlTypeId && kind != UIA_DocumentControlTypeId {
                return None;
            }
            if !writable(&element) {
                return None;
            }
            if !safe_ancestors(automation, &element, pid, hwnd) {
                return None;
            }
            runtime_id(&element)
        }
    }

    fn safe_ancestors(
        automation: &IUIAutomation,
        element: &IUIAutomationElement,
        pid: u32,
        hwnd: usize,
    ) -> bool {
        unsafe {
            let Ok(walker) = automation.ControlViewWalker() else {
                return false;
            };
            let mut current = element.clone();
            for depth in 0..12 {
                if current.CurrentProcessId().ok() != Some(pid as i32) {
                    return false;
                }
                if depth > 0
                    && current.CurrentControlType().ok() == Some(UIA_WindowControlTypeId)
                    && current
                        .CurrentNativeWindowHandle()
                        .ok()
                        .map(|handle| handle.0 as usize)
                        == Some(hwnd)
                {
                    return true;
                }
                let (Ok(id), Ok(class), Ok(name)) = (
                    current.CurrentAutomationId(),
                    current.CurrentClassName(),
                    current.CurrentName(),
                ) else {
                    return false;
                };
                // Some providers put field content in Name. These copies are
                // zeroized after classification and must never be persisted.
                let metadata = zeroize::Zeroizing::new(format!("{} {} {}", id, class, name));
                if sensitive_context(&metadata) {
                    return false;
                }
                let Ok(parent) = walker.GetParentElement(&current) else {
                    return false;
                };
                current = parent;
            }
            // An unexpectedly deep/inaccessible tree cannot prove its context.
            false
        }
    }

    fn writable(element: &IUIAutomationElement) -> bool {
        unsafe {
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            {
                // A supported ValuePattern is authoritative. Do not bypass its
                // explicit read-only answer with a second pattern.
                return pattern
                    .CurrentIsReadOnly()
                    .is_ok_and(|value| !value.as_bool());
            }
            let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            else {
                return false;
            };
            let Ok(range) = pattern.DocumentRange() else {
                return false;
            };
            let Ok(value) = range.GetAttributeValue(UIA_IsReadOnlyAttributeId) else {
                return false;
            };
            let value = Attribute(value);
            // Mixed/not-supported attributes are IUnknown variants, not bools:
            // only an explicit Boolean false proves the range is writable.
            value.0.Anonymous.Anonymous.vt == VT_BOOL
                && !value.0.Anonymous.Anonymous.Anonymous.boolVal.as_bool()
        }
    }

    fn keyboard_state(vk: u32) -> Option<[u8; 256]> {
        let mut state = [0u8; 256];
        unsafe {
            GetKeyboardState(&mut state).ok()?;
            // Low-level hooks run before the new key's async state changes.
            // Other modifier keys already have their previous, stable state.
            for key in [
                0x10usize, 0x11, 0x12, 0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5,
            ] {
                state[key] = if GetAsyncKeyState(key as i32) < 0 {
                    0x80
                } else {
                    0
                };
            }
            state[0x14] = (GetKeyState(0x14) & 1) as u8;
        }
        *state.get_mut(vk as usize)? |= 0x80;
        Some(state)
    }

    fn enqueue(key: &KBDLLHOOKSTRUCT, context: &HookContext) {
        if key.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0) != 0
            || !text_key(key.vkCode)
        {
            return;
        }
        let Some(approved) = &context.approved else {
            return;
        };
        let Some((hwnd, pid, thread)) = foreground() else {
            return;
        };
        if (hwnd, pid, thread) != (approved.hwnd, approved.pid, approved.thread) {
            return;
        }
        let Some(window_title) = title(hwnd) else {
            return;
        };
        let focus = Focus {
            app: approved.focus.app.clone(),
            title: window_title,
        };
        if !context
            .core
            .try_lock()
            .is_ok_and(|core| core.typing_allowed(&focus))
        {
            return;
        }
        let Some(keyboard) = keyboard_state(key.vkCode) else {
            return;
        };
        if shortcut(&keyboard) {
            return;
        }
        let Some(focus_hwnd) = focus_handle(thread) else {
            return;
        };
        let created = Instant::now();
        let Some(control) = safe_control(&context.automation, pid, hwnd) else {
            return;
        };
        if created.elapsed() > Duration::from_millis(100)
            || foreground() != Some((hwnd, pid, thread))
            || focus_handle(thread) != Some(focus_hwnd)
        {
            return;
        }
        let layout = unsafe { GetKeyboardLayout(thread).0 as usize };
        let _ = context.sender.try_send(KeyEvent {
            window: ApprovedWindow {
                focus,
                ..approved.clone()
            },
            control,
            keyboard,
            vk: key.vkCode,
            scan: key.scanCode,
            layout,
            focus_hwnd,
            created,
            time: Utc::now(),
        });
    }

    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && wparam.0 == WM_KEYDOWN as usize && lparam.0 != 0 {
            let _ = std::panic::catch_unwind(|| {
                CONTEXT.with(|slot| {
                    if let Ok(context) = slot.try_borrow() {
                        if let Some(context) = context.as_ref() {
                            // Windows owns this structure for the callback only.
                            let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
                            enqueue(key, context);
                        }
                    }
                });
            });
        }
        // Observing text must never suppress or modify the user's input.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn refresh_hook() {
        let target = foreground().and_then(|(hwnd, pid, thread)| {
            let focus = window::current_focus()?;
            let exe = process_name(pid)?;
            if denied_executable(&exe) || foreground() != Some((hwnd, pid, thread)) {
                return None;
            }
            Some(ApprovedWindow {
                hwnd,
                pid,
                thread,
                focus,
            })
        });
        CONTEXT.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return;
            };
            let Some(context) = slot.as_mut() else {
                return;
            };
            context.approved = target.filter(|target| {
                context
                    .core
                    .try_lock()
                    .is_ok_and(|core| core.typing_allowed(&target.focus))
            });
            unsafe {
                if context.approved.is_some() && context.hook.is_none() {
                    let module = GetModuleHandleW(None)
                        .ok()
                        .map(|module| HINSTANCE(module.0));
                    context.hook =
                        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0).ok();
                } else if context.approved.is_none() {
                    if let Some(hook) = context.hook.take() {
                        let _ = UnhookWindowsHookEx(hook);
                    }
                }
            }
            ACTIVE.store(context.hook.is_some(), Ordering::Relaxed);
        });
    }

    fn normalized(text: &str, form: windows::Win32::Globalization::NORM_FORM) -> Option<String> {
        let source: Vec<u16> = text.encode_utf16().collect();
        let mut output = [0u16; 64];
        let length = unsafe { NormalizeString(form, &source, Some(&mut output)) };
        if length <= 0 || length as usize > output.len() {
            return None;
        }
        String::from_utf16(&output[..length as usize]).ok()
    }

    fn combining_accent(accent: char) -> Option<char> {
        Some(match accent {
            '\u{b4}' => '\u{301}',
            '`' => '\u{300}',
            '^' => '\u{302}',
            '~' => '\u{303}',
            '\u{a8}' => '\u{308}',
            '\u{2c7}' => '\u{30c}',
            '\u{2da}' => '\u{30a}',
            '\u{b8}' => '\u{327}',
            '\u{2d8}' => '\u{306}',
            '\u{2d9}' => '\u{307}',
            '\u{2dd}' => '\u{30b}',
            '\u{2db}' => '\u{328}',
            '\u{af}' => '\u{304}',
            _ => return None,
        })
    }

    fn compose_dead(accent: char, output: &str) -> Option<String> {
        if output == " " || output == accent.to_string() {
            return Some(accent.to_string());
        }
        let mark = combining_accent(accent)?;
        if normalized(output, NormalizationD)?.contains(mark) {
            return Some(output.to_owned());
        }
        let mut chars = output.chars();
        let first = chars.next()?;
        let composed = normalized(&format!("{first}{mark}"), NormalizationC)?;
        if composed.chars().count() == 1 {
            Some(format!("{composed}{}", chars.as_str()))
        } else {
            Some(format!("{accent}{output}"))
        }
    }

    fn translate(event: &KeyEvent) -> Option<(String, bool)> {
        let special = match event.vk {
            0x08 => Some("\u{8}"),
            0x09 => Some("\t"),
            0x0d => Some("\n"),
            _ => None,
        };
        if let Some(text) = special {
            return Some((text.into(), false));
        }
        let mut output = [0u16; 16];
        // Bit2 prevents this observer from changing the keyboard's dead-key
        // buffer and interfering with the application's own text translation.
        let length = unsafe {
            ToUnicodeEx(
                event.vk,
                event.scan,
                &event.keyboard,
                &mut output,
                4,
                Some(HKL(event.layout as *mut _)),
            )
        };
        if length == 0 || length.unsigned_abs() as usize > output.len() {
            return None;
        }
        let text = String::from_utf16(&output[..length.unsigned_abs() as usize]).ok()?;
        safe_text(&text).then_some((text, length < 0))
    }

    fn worker(core: SharedCore, receiver: Receiver<KeyEvent>) {
        let Some(_apartment) = ComApartment::new() else {
            return;
        };
        let Ok(automation): windows::core::Result<IUIAutomation> =
            (unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) })
        else {
            return;
        };
        let mut dead: Option<(usize, Vec<i32>, usize, char)> = None;
        for event in receiver {
            let expected = (event.window.hwnd, event.window.pid, event.window.thread);
            if event.created.elapsed() > MAX_EVENT_AGE
                || foreground() != Some(expected)
                || focus_handle(event.window.thread) != Some(event.focus_hwnd)
                || safe_control(&automation, event.window.pid, event.window.hwnd).as_ref()
                    != Some(&event.control)
                || window::current_focus().as_ref() != Some(&event.window.focus)
            {
                dead = None;
                continue;
            }
            if !core
                .try_lock()
                .is_ok_and(|state| state.typing_allowed(&event.window.focus))
            {
                dead = None;
                continue;
            }
            let Some((mut text, is_dead)) = translate(&event) else {
                dead = None;
                continue;
            };
            if is_dead {
                dead = text
                    .chars()
                    .next()
                    .map(|accent| (event.window.hwnd, event.control, event.layout, accent));
                continue;
            }
            if let Some((hwnd, control, layout, accent)) = dead.take() {
                if hwnd == event.window.hwnd && control == event.control && layout == event.layout {
                    let Some(composed) = compose_dead(accent, &text) else {
                        continue;
                    };
                    text = composed;
                }
            }
            if safe_text(&text)
                && event.created.elapsed() <= MAX_EVENT_AGE
                && foreground() == Some(expected)
                && safe_control(&automation, event.window.pid, event.window.hwnd).as_ref()
                    == Some(&event.control)
            {
                // Provider calls happen outside the Core mutex. Recheck policy
                // after acquiring it, then synchronize focus so the first key
                // after an app switch need not wait for the activity poll.
                if let Ok(mut state) = core.try_lock() {
                    if state.typing_allowed(&event.window.focus)
                        && foreground() == Some(expected)
                        && title(event.window.hwnd).as_deref()
                            == Some(event.window.focus.title.as_str())
                        && event.created.elapsed() <= MAX_EVENT_AGE
                    {
                        state.tick_with_idle(
                            Some(event.window.focus.clone()),
                            event.time,
                            Some(Duration::ZERO),
                        );
                        state.record_text(&event.window.focus, &text, event.time);
                    }
                }
            }
        }
    }

    pub(super) fn spawn(core: SharedCore) {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let worker_core = core.clone();
        std::thread::spawn(move || worker(worker_core, receiver));
        std::thread::spawn(move || {
            let Some(_apartment) = ComApartment::new() else {
                return;
            };
            let Ok(automation): windows::core::Result<IUIAutomation> =
                (unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) })
            else {
                return;
            };
            CONTEXT.with(|slot| {
                *slot.borrow_mut() = Some(HookContext {
                    core,
                    sender,
                    automation,
                    approved: None,
                    hook: None,
                })
            });
            let timer = unsafe { SetTimer(None, 0, 200, None) };
            if timer == 0 {
                CONTEXT.with(|slot| *slot.borrow_mut() = None);
                return;
            }
            refresh_hook();
            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                if message.message == WM_TIMER && message.wParam.0 == timer {
                    refresh_hook();
                } else {
                    unsafe {
                        DispatchMessageW(&message);
                    }
                }
            }
            unsafe {
                let _ = KillTimer(None, timer);
            }
            CONTEXT.with(|slot| {
                if let Some(context) = slot.borrow_mut().take() {
                    if let Some(hook) = context.hook {
                        unsafe {
                            let _ = UnhookWindowsHookEx(hook);
                        }
                    }
                }
            });
            ACTIVE.store(false, Ordering::Relaxed);
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn dead_key_composition_does_not_duplicate_an_already_composed_character() {
            assert_eq!(compose_dead('\u{b4}', "e").as_deref(), Some("é"));
            assert_eq!(compose_dead('\u{b4}', "é").as_deref(), Some("é"));
            assert_eq!(compose_dead('^', " ").as_deref(), Some("^"));
            assert_eq!(compose_dead('\u{b4}', "x").as_deref(), Some("´x"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminals_password_managers_and_daylog_are_always_denied() {
        for path in [
            r"C:\Windows\System32\cmd.exe",
            "pwsh.exe",
            "WindowsTerminal.EXE",
            "keepassxc.exe",
            "1Password.exe",
            "daylog.exe",
            "mstsc.exe",
            "LockApp.exe",
            "WINLOGON.EXE",
            "SecHealthUI.exe",
            "SecurityHealthSystray.exe",
            "SecurityHealthHost.exe",
            "SecurityHealthService.exe",
            "smartscreen.exe",
            "credwiz.exe",
        ] {
            assert!(denied_executable(path));
        }
        assert!(!denied_executable(r"C:\Windows\notepad.exe"));
        assert!(!denied_executable("notepad++.exe"));
    }
    #[test]
    fn modifiers_and_navigation_are_not_typed_text() {
        let mut keys = [0u8; 256];
        assert!(!shortcut(&keys));
        for modifier in [0x11, 0x12, 0x5b, 0x5c, 0xa2, 0xa3, 0xa4, 0xa5] {
            keys[modifier] = 0x80;
            assert!(shortcut(&keys));
            keys[modifier] = 0;
        }
        keys[0x10] = 0x80;
        assert!(!shortcut(&keys));
        for vk in [0x10, 0x11, 0x12, 0x1b, 0x21, 0x25, 0x2e, 0x70, 0xe5, 0xe7] {
            assert!(!text_key(vk));
        }
        for vk in [0x08, 0x09, 0x0d, 0x20, 0x41, 0x31, 0x61, 0xba] {
            assert!(text_key(vk));
        }
    }
    #[test]
    fn only_plain_text_and_explicit_edit_controls_are_accepted() {
        for text in ["hello", "é", "😀", "\n", "\t", "\u{8}"] {
            assert!(safe_text(text));
        }
        for text in ["", "\0", "\u{1b}", "\u{7}"] {
            assert!(!safe_text(text));
        }
    }

    #[test]
    fn sensitive_embedded_terminal_and_credential_contexts_are_denied() {
        for label in [
            "xterm-helper-textarea",
            "Terminal input",
            "Integrated Console",
            "PasswordBox",
            "Credentials",
            "API token",
            "private-key",
            "Secret entry",
            "SSH Session",
            "Remote Desktop",
            "Virtual Machine",
            "noVNC viewer",
        ] {
            assert!(sensitive_context(label), "{label}");
        }
        for label in [
            "Editor input",
            "Text editor",
            "RichEditD2DPT",
            "Document",
            "Search",
            "Main code editor",
        ] {
            assert!(!sensitive_context(label), "{label}");
        }
    }
}
