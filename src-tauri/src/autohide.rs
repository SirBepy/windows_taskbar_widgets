use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

// toggle_strip is the sole writer; true means the user explicitly hid the strip,
// so the poll below must skip entirely rather than auto-show it back.
static USER_HIDDEN: AtomicBool = AtomicBool::new(false);

// 250ms rather than 500: this is also how fast the strip reappears when an
// auto-hide taskbar slides back in, and the taskbar's own animation is ~200ms.
const POLL_INTERVAL_MS: u64 = 250;

pub fn set_user_hidden(hidden: bool) {
    USER_HIDDEN.store(hidden, Ordering::SeqCst);
}

/// The strip mirrors the taskbar's own visibility. Re-asserts topmost every tick:
/// the taskbar is topmost too, and whichever asserted it last wins.
pub fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        if USER_HIDDEN.load(Ordering::SeqCst) {
            continue;
        }
        let Some(win) = app.get_webview_window("strip") else { continue };
        let hide_needed = crate::taskbar::taskbar_hidden() || foreground_fullscreen();
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
        }
    });
}

/// Explorer restarts and fullscreen transitions silently demote a topmost window,
/// and Windows never restores it; re-asserting is the only way back up.
#[cfg(target_os = "windows")]
fn raise_topmost(win: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let Ok(hwnd) = win.hwnd() else { return };
    unsafe {
        SetWindowPos(
            hwnd.0 as _,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn raise_topmost(_win: &tauri::WebviewWindow) {}

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

        let Some((wnd, rc_monitor)) =
            crate::taskbar::window_and_monitor_rect(fg, MONITOR_DEFAULTTONULL)
        else {
            return false;
        };
        wnd.left <= rc_monitor.left
            && wnd.top <= rc_monitor.top
            && wnd.right >= rc_monitor.right
            && wnd.bottom >= rc_monitor.bottom
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_fullscreen() -> bool {
    false
}
