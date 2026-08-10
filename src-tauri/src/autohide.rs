use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

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
// how the strip's hwnd reaches it. Refreshed every poll tick.
#[cfg(target_os = "windows")]
static STRIP_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_user_hidden(hidden: bool) {
    USER_HIDDEN.store(hidden, Ordering::SeqCst);
}

pub fn set_user_forced_visible(forced: bool) {
    USER_FORCED_VISIBLE.store(forced, Ordering::SeqCst);
}

/// Mirrors the taskbar's visibility when follow_taskbar is on, and re-asserts topmost
/// only when the strip actually lost the band or the taskbar climbed above it.
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
            prev_fullscreen = fullscreen;
            if USER_HIDDEN.load(Ordering::SeqCst) {
                continue;
            }
            let Some(win) = app.get_webview_window("strip") else { continue };
            #[cfg(target_os = "windows")]
            if let Ok(hwnd) = win.hwnd() {
                STRIP_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
            }
            let follow_taskbar = app
                .try_state::<crate::settings::SettingsState>()
                .and_then(|s| s.0.lock().ok().map(|s| s.follow_taskbar))
                .unwrap_or(true);
            let forced_visible = USER_FORCED_VISIBLE.load(Ordering::SeqCst);
            let hide_needed = (follow_taskbar && crate::taskbar::taskbar_hidden(&app))
                || (fullscreen && !forced_visible);
            let visible = win.is_visible().unwrap_or(true);
            if hide_needed && visible {
                let _ = win.hide();
                // An open flyout is its own always-on-top window; hiding only the strip
                // would leave it floating over the fullscreen app.
                crate::flyout::close_flyout(app.clone());
            } else if !hide_needed {
                if !visible {
                    let _ = win.show();
                }
                raise_topmost(&win);
                crate::taskbar::reassert_strip_position(&app);
            }
        }
    });
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
    let strip = STRIP_HWND.load(Ordering::SeqCst);
    if strip != 0 {
        raise_topmost_hwnd(strip);
    }
}

/// SHQueryUserNotificationState catches D3D/presentation/busy fullscreen; it misses
/// some borderless fullscreen, so fall back to foreground-window-covers-monitor.
#[cfg(target_os = "windows")]
fn foreground_fullscreen() -> bool {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL;
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};

    unsafe {
        let mut state = 0;
        if SHQueryUserNotificationState(&mut state) == 0
            && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE | QUNS_BUSY)
        {
            return true;
        }

        let fg = GetForegroundWindow();
        if fg.is_null() {
            return false;
        }
        // Progman/WorkerW (desktop) spans the whole monitor same as real fullscreen;
        // never treat the bare desktop as a reason to hide the strip.
        let mut class = [0u16; 256];
        let len = GetClassNameW(fg, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW") {
            return false;
        }

        let Some(wm) = crate::taskbar::window_and_monitor_rect(fg, MONITOR_DEFAULTTONULL) else {
            return false;
        };
        wm.window.left <= wm.monitor.left
            && wm.window.top <= wm.monitor.top
            && wm.window.right >= wm.monitor.right
            && wm.window.bottom >= wm.monitor.bottom
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_fullscreen() -> bool {
    false
}
