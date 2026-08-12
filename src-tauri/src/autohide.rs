use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::taskbar::DetectedTaskbar;

// toggle_strip is the sole writer; true means the user explicitly hid the strip,
// so the poll below must skip entirely rather than auto-show it back.
static USER_HIDDEN: AtomicBool = AtomicBool::new(false);

// Set by a manual show; suppresses only the fullscreen hide condition, not
// taskbar_hidden, so the strip still tracks the taskbar per the dev's rule.
// Clears itself on the next foreground_fullscreen() flip, not on a timer.
static USER_FORCED_VISIBLE: AtomicBool = AtomicBool::new(false);

// 250ms rather than 500: this is also how fast the strip reappears when an
// auto-hide taskbar slides back in, and the taskbar's own animation is ~200ms.
const POLL_INTERVAL_MS: u64 = 250;

// The WinEventHook callback is extern "system" and can't capture state, so this is
// how every live strip's hwnd reaches it. Refreshed every poll tick.
#[cfg(target_os = "windows")]
static STRIP_HWNDS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub fn set_user_hidden(hidden: bool) {
    USER_HIDDEN.store(hidden, Ordering::SeqCst);
}

pub fn set_user_forced_visible(forced: bool) {
    USER_FORCED_VISIBLE.store(forced, Ordering::SeqCst);
}

/// Mirrors the taskbar's visibility when follow_taskbar is on, and re-asserts topmost
/// only when a strip actually lost the band or the taskbar climbed above it. Also folds
/// in `strip::reconcile` so a monitor hotplug creates/destroys strip windows: there is
/// no WM_DISPLAYCHANGE listener in this codebase, so blind re-polling is the mechanism.
pub fn spawn_poller(app: AppHandle) {
    #[cfg(target_os = "windows")]
    spawn_foreground_watcher();
    std::thread::spawn(move || {
        let mut prev_fullscreen = foreground_fullscreen();
        loop {
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            // Tracked even while USER_HIDDEN, so a stale flip doesn't fire the
            // instant a later manual show sets USER_FORCED_VISIBLE.
            let fullscreen = foreground_fullscreen();
            if fullscreen != prev_fullscreen {
                USER_FORCED_VISIBLE.store(false, Ordering::SeqCst);
            }
            prev_fullscreen = fullscreen.clone();
            crate::strip::reconcile(&app);
            if USER_HIDDEN.load(Ordering::SeqCst) {
                continue;
            }
            run_tick(&app, fullscreen.as_deref());
        }
    });
}

/// Every live strip: the bare primary label plus any runtime secondary
/// (`strip::LABEL_PREFIX`-prefixed) window `strip::reconcile` has built so far.
fn live_strip_labels(app: &AppHandle) -> Vec<String> {
    app.webview_windows()
        .into_keys()
        .filter(|l| l == crate::strip::PRIMARY_LABEL || l.starts_with(crate::strip::LABEL_PREFIX))
        .collect()
}

/// One tick: ONE `enumerate_taskbars()` Win32 pass, then every live strip is matched
/// against that in-memory `Vec<DetectedTaskbar>` via `taskbar_hidden_for` - no strip
/// re-enumerates, which is the N-times-the-polling-cost the plan forbids.
fn run_tick(app: &AppHandle, fullscreen_device: Option<&str>) {
    let taskbars = crate::taskbar::enumerate_taskbars();
    let (follow_taskbar, taskbar_monitor) = app
        .try_state::<crate::settings::SettingsState>()
        .and_then(|s| s.0.lock().ok().map(|s| (s.follow_taskbar, s.taskbar_monitor.clone())))
        .unwrap_or((true, String::new()));
    let forced_visible = USER_FORCED_VISIBLE.load(Ordering::SeqCst);
    // The primary label follows whichever taskbar the user picked in settings
    // (rect.rs's taskbar_rect_for_window docks it the same way); a secondary is
    // pinned unconditionally to its own monitor via StripState.
    let primary_device = crate::taskbar::select_taskbar(&taskbars, &taskbar_monitor)
        .map(|t| t.device_name.clone());

    #[cfg(target_os = "windows")]
    let mut hwnds: Vec<isize> = Vec::new();

    for label in live_strip_labels(app) {
        let Some(win) = app.get_webview_window(&label) else { continue };
        #[cfg(target_os = "windows")]
        if let Ok(hwnd) = win.hwnd() {
            hwnds.push(hwnd.0 as isize);
        }
        let device = if label == crate::strip::PRIMARY_LABEL {
            primary_device.clone()
        } else {
            app.state::<crate::strip::StripState>().0.lock().ok().and_then(|m| m.get(label.as_str()).cloned())
        };
        let hide_needed = strip_should_hide(device.as_deref(), &taskbars, follow_taskbar, fullscreen_device, forced_visible);
        let visible = win.is_visible().unwrap_or(true);
        if hide_needed && visible {
            let _ = win.hide();
            // The flyout re-anchors to whichever strip is hovered (flyout.rs, step 6),
            // so close it only when the hiding strip is the one it's currently on.
            if crate::flyout::is_anchored_to(&label) {
                crate::flyout::close_flyout(app.clone());
            }
        } else if !hide_needed {
            if !visible {
                let _ = win.show();
            }
            raise_topmost(&win);
            reassert_position(app, &win);
        }
    }

    #[cfg(target_os = "windows")]
    if let Ok(mut guard) = STRIP_HWNDS.lock() {
        *guard = hwnds;
    }
}

/// Whether the strip pinned to `device` should hide this tick: docked-taskbar state
/// (via the shared `taskbars` snapshot, never re-enumerated) OR a fullscreen app on
/// that same device. `taskbar_hidden_for` is windows-only, so off-windows the taskbar
/// check is always false, same as this crate's other off-windows Win32 stubs.
fn strip_should_hide(
    device: Option<&str>,
    taskbars: &[DetectedTaskbar],
    follow_taskbar: bool,
    fullscreen_device: Option<&str>,
    forced_visible: bool,
) -> bool {
    #[cfg(target_os = "windows")]
    let hidden_by_taskbar = follow_taskbar
        && match device.and_then(|d| taskbars.iter().find(|t| t.device_name == d)) {
            Some(t) => crate::taskbar::taskbar_hidden_for(t),
            None => true,
        };
    #[cfg(not(target_os = "windows"))]
    let hidden_by_taskbar = {
        let _ = (taskbars, follow_taskbar);
        false
    };

    let hidden_by_fullscreen = !forced_visible && device.is_some() && device == fullscreen_device;
    hidden_by_taskbar || hidden_by_fullscreen
}

/// Re-anchors a strip after a possible hotplug shift. The primary keeps rect.rs's
/// width-cache path; a secondary has no such cache yet, so it re-asserts from its own
/// current CSS width, which round-trips size/position math without changing size.
fn reassert_position(app: &AppHandle, win: &tauri::WebviewWindow) {
    if win.label() == crate::strip::PRIMARY_LABEL {
        crate::taskbar::reassert_strip_position(app);
        return;
    }
    let window = win.as_ref().window();
    let scale = win.scale_factor().unwrap_or(1.0);
    let css_width = win.outer_size().map(|s| s.width as f64 / scale).unwrap_or(320.0);
    let _ = crate::taskbar::position_strip(app, &window, css_width);
}

/// Explorer restarts and fullscreen transitions silently demote a topmost window,
/// and Windows never restores it; re-asserting is the only way back up.
#[cfg(target_os = "windows")]
fn raise_topmost(win: &tauri::WebviewWindow) {
    let Ok(hwnd) = win.hwnd() else { return };
    raise_topmost_hwnd(hwnd.0 as isize);
}

#[cfg(not(target_os = "windows"))]
fn raise_topmost(_win: &tauri::WebviewWindow) {}

#[cfg(target_os = "windows")]
fn raise_topmost_hwnd(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    if still_in_topmost_band(hwnd) {
        return;
    }
    unsafe {
        SetWindowPos(
            hwnd as _,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// WS_EX_TOPMOST never clears on a same-band reorder, so it can't alone tell whether
/// the strip fell out of the band. Re-assert when a visible window above it is either
/// non-topmost (band lost) or the taskbar (Shell_TrayWnd is topmost too, and it
/// winning the last assert is exactly the "strip is under the taskbar" bug).
#[cfg(target_os = "windows")]
fn still_in_topmost_band(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowLongW, IsWindowVisible, GWL_EXSTYLE, GW_HWNDPREV, WS_EX_TOPMOST,
    };
    unsafe {
        let mut cur = hwnd as _;
        loop {
            cur = GetWindow(cur, GW_HWNDPREV);
            if cur.is_null() {
                return true;
            }
            if IsWindowVisible(cur) == 0 {
                continue;
            }
            let non_topmost = GetWindowLongW(cur, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST == 0;
            if non_topmost || crate::taskbar::is_taskbar_class(cur) {
                return false;
            }
        }
    }
}

/// Must run its own message loop: WINEVENT_OUTOFCONTEXT delivers callbacks through
/// this thread's queue, and a hook with no pump never fires.
#[cfg(target_os = "windows")]
fn spawn_foreground_watcher() {
    use windows_sys::Win32::UI::Accessibility::SetWinEventHook;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND,
        WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, MSG,
    };
    std::thread::spawn(|| unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(on_foreground_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn on_foreground_event(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
    _id_object: i32,
    _id_child: i32,
    _thread_id: u32,
    _time: u32,
) {
    if hwnd.is_null() || !crate::taskbar::is_taskbar_class(hwnd) {
        return;
    }
    // Runs on the hook's own message-pump thread: it must never block, so a
    // contended lock just skips this tick's re-raise rather than stalling the OS callback.
    let Ok(hwnds) = STRIP_HWNDS.try_lock() else { return };
    for &strip in hwnds.iter() {
        raise_topmost_hwnd(strip);
    }
}

/// SHQueryUserNotificationState catches D3D/presentation/busy fullscreen; it misses
/// some borderless fullscreen, so fall back to foreground-window-covers-monitor. Both
/// branches need the foreground window's monitor to attribute which strip should hide,
/// so a state hit with no resolvable foreground window is treated as no fullscreen.
#[cfg(target_os = "windows")]
fn foreground_fullscreen() -> Option<String> {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL;
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};

    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return None;
        }
        // Progman/WorkerW (desktop) spans the whole monitor same as real fullscreen;
        // never treat the bare desktop as a reason to hide the strip.
        let mut class = [0u16; 256];
        let len = GetClassNameW(fg, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW") {
            return None;
        }
        let wm = crate::taskbar::window_and_monitor_rect(fg, MONITOR_DEFAULTTONULL)?;

        let mut state = 0;
        let presentation = SHQueryUserNotificationState(&mut state) == 0
            && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE | QUNS_BUSY);
        let covers_monitor = wm.window.left <= wm.monitor.left
            && wm.window.top <= wm.monitor.top
            && wm.window.right >= wm.monitor.right
            && wm.window.bottom >= wm.monitor.bottom;

        (presentation || covers_monitor).then_some(wm.device_name)
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_fullscreen() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    fn taskbar(device_name: &str) -> DetectedTaskbar {
        DetectedTaskbar {
            hwnd: 0,
            device_name: device_name.to_string(),
            is_primary: true,
            taskbar_rect: (0, 1400, 2560, 1440),
            monitor_rect: (0, 0, 2560, 1440),
        }
    }

    #[test]
    fn fullscreen_on_a_different_monitor_does_not_hide() {
        assert!(!strip_should_hide(Some("A"), &[], false, Some("B"), false));
    }

    #[test]
    fn fullscreen_on_its_own_monitor_hides() {
        assert!(strip_should_hide(Some("A"), &[], false, Some("A"), false));
    }

    #[test]
    fn forced_visible_suppresses_the_fullscreen_hide() {
        assert!(!strip_should_hide(Some("A"), &[], false, Some("A"), true));
    }

    #[test]
    fn unknown_device_never_matches_fullscreen() {
        assert!(!strip_should_hide(None, &[], false, Some("A"), false));
    }

    #[test]
    fn follow_taskbar_off_ignores_taskbar_state() {
        assert!(!strip_should_hide(Some("A"), &[], false, None, false));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn device_with_no_matching_taskbar_counts_as_hidden() {
        assert!(strip_should_hide(Some("A"), &[], true, None, false));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn device_matching_an_invalid_hwnd_taskbar_is_hidden() {
        // hwnd 0 makes IsWindowVisible fail closed inside taskbar_hidden_for.
        let taskbars = [taskbar("A")];
        assert!(strip_should_hide(Some("A"), &taskbars, true, None, false));
    }
}
