use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

// toggle_strip is the sole writer; true means the user explicitly hid the strip,
// so the poll below must skip entirely rather than auto-show it back.
static USER_HIDDEN: AtomicBool = AtomicBool::new(false);

const POLL_INTERVAL_MS: u64 = 500;

pub fn set_user_hidden(hidden: bool) {
    USER_HIDDEN.store(hidden, Ordering::SeqCst);
}

/// Hides the strip while the taskbar auto-hides or a fullscreen app owns the
/// foreground, shows it again once both clear. Never runs while user-hidden.
pub fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        if USER_HIDDEN.load(Ordering::SeqCst) {
            continue;
        }
        let Some(win) = app.get_webview_window("strip") else { continue };
        let hide_needed = crate::taskbar::taskbar_autohide() || foreground_fullscreen();
        let visible = win.is_visible().unwrap_or(true);
        if hide_needed && visible {
            let _ = win.hide();
            // An open flyout is its own always-on-top window; hiding only the strip
            // would leave it floating over the fullscreen app.
            crate::flyout::close_flyout(app.clone());
        } else if !hide_needed && !visible {
            let _ = win.show();
        }
    });
}

/// SHQueryUserNotificationState catches D3D/presentation/busy fullscreen; it misses
/// some borderless fullscreen, so fall back to foreground-window-covers-monitor.
#[cfg(target_os = "windows")]
fn foreground_fullscreen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowRect,
    };

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

        let mut wnd: RECT = std::mem::zeroed();
        if GetWindowRect(fg, &mut wnd) == 0 {
            return false;
        }
        let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
        if monitor.is_null() {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut mi) == 0 {
            return false;
        }
        wnd.left <= mi.rcMonitor.left
            && wnd.top <= mi.rcMonitor.top
            && wnd.right >= mi.rcMonitor.right
            && wnd.bottom >= mi.rcMonitor.bottom
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_fullscreen() -> bool {
    false
}
