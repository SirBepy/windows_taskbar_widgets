use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

#[cfg(target_os = "windows")]
use super::monitors::{enumerate_taskbars, selected_taskbar, DetectedTaskbar};

// Cached for reassert_strip_position to redo layout without a fresh JS width.
static LAST_STRIP_WIDTH_CSS: AtomicU64 = AtomicU64::new(320.0f64.to_bits());

/// Primary-taskbar rect in physical px via SHAppBarMessage; None off-Windows or if the
/// call fails. ABM_GETTASKBARPOS reports the docked position even while auto-hidden,
/// which is exactly why `taskbar_hidden` below deliberately does NOT use this API.
#[cfg(target_os = "windows")]
fn primary_taskbar_rect() -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
    unsafe {
        let mut abd: APPBARDATA = std::mem::zeroed();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) == 0 {
            return None;
        }
        Some((abd.rc.left, abd.rc.top, abd.rc.right, abd.rc.bottom))
    }
}

/// Rect of the `taskbar_monitor` setting's chosen taskbar. Primary goes through
/// `primary_taskbar_rect` (SHAppBarMessage is primary-only by design); a secondary
/// taskbar uses its own live window rect from `enumerate_taskbars`.
#[cfg(target_os = "windows")]
pub fn taskbar_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    match selected_taskbar(app) {
        Some(t) if !t.is_primary => Some(docked_secondary_rect(t.taskbar_rect, t.monitor_rect)),
        _ => primary_taskbar_rect(),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn taskbar_rect(_app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    None
}

/// Monitor rect of the chosen taskbar's display, for callers that clamp to the screen
/// the strip actually lives on. None means "couldn't detect", so callers keep their
/// own primary-monitor fallback rather than clamping to something wrong.
#[cfg(target_os = "windows")]
pub fn selected_monitor_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    selected_taskbar(app).map(|t| t.monitor_rect)
}

#[cfg(not(target_os = "windows"))]
pub fn selected_monitor_rect(_app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    None
}

// An auto-hide taskbar slid off screen still leaves a ~2px sliver behind, so
// "is it on screen" has to be a real overlap test, not a nonzero one.
#[cfg(target_os = "windows")]
const SLIVER_PX: i32 = 8;

/// True when `rc` overlaps `monitor` by less than SLIVER_PX on either axis - the
/// slid-off-screen state of an auto-hidden taskbar. Shared by `taskbar_hidden`
/// and `docked_secondary_rect` so there is one definition of "hidden".
#[cfg(target_os = "windows")]
fn rect_mostly_off_monitor(rc: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32)) -> bool {
    let overlap_h = rc.3.min(monitor.3) - rc.1.max(monitor.1);
    let overlap_w = rc.2.min(monitor.2) - rc.0.max(monitor.0);
    overlap_h.min(overlap_w) < SLIVER_PX
}

/// Reconstructs a secondary taskbar's docked rect when auto-hide has slid it off
/// screen. `GetWindowRect` keeps the window's full thickness while hidden, only its
/// position moves, so this snaps that thickness flush to whichever monitor edge the
/// rect's centre says it's docked to (wide+short -> top/bottom, tall+narrow -> left/right).
#[cfg(target_os = "windows")]
fn docked_secondary_rect(
    taskbar_rect: (i32, i32, i32, i32),
    monitor_rect: (i32, i32, i32, i32),
) -> (i32, i32, i32, i32) {
    if !rect_mostly_off_monitor(taskbar_rect, monitor_rect) {
        return taskbar_rect;
    }
    let (left, top, right, bottom) = taskbar_rect;
    let (m_left, m_top, m_right, m_bottom) = monitor_rect;
    let width = right - left;
    let height = bottom - top;

    if width >= height {
        if top + bottom < m_top + m_bottom {
            (m_left, m_top, m_left + width, m_top + height)
        } else {
            (m_left, m_bottom - height, m_left + width, m_bottom)
        }
    } else if left + right < m_left + m_right {
        (m_left, m_top, m_left + width, m_top + height)
    } else {
        (m_right - width, m_top, m_right, m_top + height)
    }
}

/// Window rect, its monitor's rect, and (Windows only) that monitor's device name
/// and primary flag - one MONITORINFOEXW call gives all four, so every caller gets
/// the name even if only `detect_taskbar` uses it.
#[cfg(target_os = "windows")]
pub struct WindowMonitorRect {
    pub window: windows_sys::Win32::Foundation::RECT,
    pub monitor: windows_sys::Win32::Foundation::RECT,
    pub device_name: String,
    pub is_primary: bool,
}

/// Shared by taskbar.rs, monitors.rs and autohide.rs: a window's rect plus its
/// monitor's rect. `monitor_flags` is a MonitorFromWindow dwFlags value; callers
/// differ on whether "no monitor found" should be treated as a hard failure
/// (MONITOR_DEFAULTTONULL) or fall back to the nearest one (MONITOR_DEFAULTTONEAREST).
#[cfg(target_os = "windows")]
pub fn window_and_monitor_rect(
    hwnd: windows_sys::Win32::Foundation::HWND,
    monitor_flags: u32,
) -> Option<WindowMonitorRect> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, MONITORINFOF_PRIMARY};

    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rc) == 0 {
            return None;
        }
        let monitor = MonitorFromWindow(hwnd, monitor_flags);
        if monitor.is_null() {
            return None;
        }
        let mut mi: MONITORINFOEXW = std::mem::zeroed();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, &mut mi as *mut _ as *mut _) == 0 {
            return None;
        }
        let len = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(mi.szDevice.len());
        Some(WindowMonitorRect {
            window: rc,
            monitor: mi.monitorInfo.rcMonitor,
            device_name: String::from_utf16_lossy(&mi.szDevice[..len]),
            is_primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        })
    }
}

/// True when `taskbar`'s live window is off screen - pure, no fresh Win32
/// enumeration beyond IsWindowVisible. A caller already holding a `DetectedTaskbar`
/// from one `enumerate_taskbars()` pass can call this per-strip without re-enumerating.
#[cfg(target_os = "windows")]
pub fn taskbar_hidden_for(taskbar: &DetectedTaskbar) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe {
        if IsWindowVisible(taskbar.hwnd as _) == 0 {
            return true;
        }
    }
    rect_mostly_off_monitor(taskbar.taskbar_rect, taskbar.monitor_rect)
}

/// True when the chosen taskbar is not on screen right now. Deliberately checks the
/// LIVE window, not ABM_GETSTATE's auto-hide flag: that flag reports the mode, so it
/// stays set while the user hovers and the taskbar is actually slid in and visible.
#[cfg(target_os = "windows")]
pub fn taskbar_hidden(app: &AppHandle) -> bool {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    if let Some(t) = selected_taskbar(app) {
        if !t.is_primary {
            return taskbar_hidden_for(&t);
        }
    }
    // Explorer restarting destroys and recreates the tray window; re-finding it fresh
    // every call is what lets the primary strip come back on its own afterwards.
    let hwnd = unsafe {
        let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
        FindWindowW(class.as_ptr(), std::ptr::null())
    };
    if hwnd.is_null() {
        return true;
    }
    let Some(wm) = window_and_monitor_rect(hwnd, MONITOR_DEFAULTTONEAREST) else {
        return false;
    };
    taskbar_hidden_for(&DetectedTaskbar {
        hwnd: hwnd as isize,
        device_name: wm.device_name,
        is_primary: wm.is_primary,
        taskbar_rect: (wm.window.left, wm.window.top, wm.window.right, wm.window.bottom),
        monitor_rect: (wm.monitor.left, wm.monitor.top, wm.monitor.right, wm.monitor.bottom),
    })
}

#[cfg(not(target_os = "windows"))]
pub fn taskbar_hidden(_app: &AppHandle) -> bool {
    false
}

/// Rect of the taskbar `window` should dock to. The primary strip resolves the
/// user's `taskbar_monitor` choice via `taskbar_rect`; every other strip is pinned
/// unconditionally to its own monitor via `StripState` - no user choice, it is
/// pinned there by construction (Decision, per-monitor-phase2-plan.md).
#[cfg(target_os = "windows")]
fn taskbar_rect_for_window(app: &AppHandle, window: &tauri::Window) -> Option<(i32, i32, i32, i32)> {
    if window.label() == crate::strip::PRIMARY_LABEL {
        return taskbar_rect(app);
    }
    let device_name =
        app.state::<crate::strip::StripState>().0.lock().ok()?.get(window.label()).cloned()?;
    let t = enumerate_taskbars().into_iter().find(|t| t.device_name == device_name)?;
    if t.is_primary {
        primary_taskbar_rect()
    } else {
        Some(docked_secondary_rect(t.taskbar_rect, t.monitor_rect))
    }
}

#[cfg(not(target_os = "windows"))]
fn taskbar_rect_for_window(_app: &AppHandle, _window: &tauri::Window) -> Option<(i32, i32, i32, i32)> {
    None
}

/// Left-anchor the strip over the taskbar's empty left region (Win11 centers
/// pinned icons, so the left edge is free). Falls back to bottom-left of the
/// work area when the taskbar rect is unavailable. `window` is whichever strip
/// (primary or a runtime secondary) is being positioned.
pub fn position_strip(app: &AppHandle, window: &tauri::Window, strip_css_width: f64) -> tauri::Result<()> {
    // Only the primary's width feeds reassert_strip_position's cache: a secondary
    // strip overwriting it would misposition the primary on the next reassert.
    if window.label() == crate::strip::PRIMARY_LABEL {
        LAST_STRIP_WIDTH_CSS.store(strip_css_width.to_bits(), Ordering::Relaxed);
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let settings = app.state::<crate::settings::SettingsState>();
    let left_margin = settings.0.lock().map(|s| s.left_margin).unwrap_or(12) as f64;

    let w = (strip_css_width * scale).round() as u32;
    let (size, pos) = if let Some((left, top, _, bottom)) = taskbar_rect_for_window(app, window) {
        let h = (bottom - top).max(1) as u32;
        let x = left + (left_margin * scale).round() as i32;
        (PhysicalSize::new(w.max(1), h), PhysicalPosition::new(x, top))
    } else if let Ok(Some(monitor)) = window.primary_monitor() {
        let h = (48.0 * scale).round() as u32;
        let wa = monitor.work_area();
        let x = wa.position.x + (left_margin * scale) as i32;
        let y = wa.position.y + wa.size.height as i32 - h as i32;
        (PhysicalSize::new(w.max(1), h), PhysicalPosition::new(x, y))
    } else {
        return Ok(());
    };
    // Skip no-op Win32 calls: this runs 4x/sec from autohide's poller.
    if window.outer_size().ok() != Some(size) {
        window.set_size(size)?;
    }
    if window.outer_position().ok() != Some(pos) {
        window.set_position(pos)?;
    }
    Ok(())
}

/// Re-anchors the strip using the last known width - a monitor unplug/replug
/// relocates it without touching CSS layout, so ResizeObserver never refires.
pub fn reassert_strip_position(app: &AppHandle) {
    let width_css = f64::from_bits(LAST_STRIP_WIDTH_CSS.load(Ordering::Relaxed));
    if let Some(win) = app.get_webview_window(crate::strip::PRIMARY_LABEL) {
        let window = win.as_ref().window();
        let _ = position_strip(app, &window, width_css);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    const MONITOR: (i32, i32, i32, i32) = (0, 0, 2560, 1440);

    #[test]
    fn taskbar_hidden_for_treats_invalid_hwnd_as_hidden() {
        // hwnd 0 is never a valid window, so IsWindowVisible short-circuits before the
        // rect comparison. It's the only branch testable without a real live HWND.
        let t = DetectedTaskbar {
            hwnd: 0,
            device_name: r"\\.\DISPLAY1".to_string(),
            is_primary: true,
            taskbar_rect: (0, 1400, 2560, 1440),
            monitor_rect: MONITOR,
        };
        assert!(taskbar_hidden_for(&t));
    }

    #[test]
    fn bottom_docked_rect_is_returned_unchanged() {
        let docked = (0, 1400, 2560, 1440);
        assert_eq!(docked_secondary_rect(docked, MONITOR), docked);
    }

    #[test]
    fn bottom_hidden_rect_snaps_flush_to_bottom_edge() {
        let slid = (0, 1438, 2560, 1478);
        assert_eq!(docked_secondary_rect(slid, MONITOR), (0, 1400, 2560, 1440));
    }

    #[test]
    fn top_docked_rect_is_returned_unchanged() {
        let docked = (0, 0, 2560, 40);
        assert_eq!(docked_secondary_rect(docked, MONITOR), docked);
    }

    #[test]
    fn top_hidden_rect_snaps_flush_to_top_edge() {
        let slid = (0, -38, 2560, 2);
        assert_eq!(docked_secondary_rect(slid, MONITOR), (0, 0, 2560, 40));
    }

    #[test]
    fn left_docked_rect_is_returned_unchanged() {
        let docked = (0, 0, 40, 1440);
        assert_eq!(docked_secondary_rect(docked, MONITOR), docked);
    }

    #[test]
    fn left_hidden_rect_snaps_flush_to_left_edge() {
        let slid = (-38, 0, 2, 1440);
        assert_eq!(docked_secondary_rect(slid, MONITOR), (0, 0, 40, 1440));
    }

    #[test]
    fn right_docked_rect_is_returned_unchanged() {
        let docked = (2520, 0, 2560, 1440);
        assert_eq!(docked_secondary_rect(docked, MONITOR), docked);
    }

    #[test]
    fn right_hidden_rect_snaps_flush_to_right_edge() {
        let slid = (2558, 0, 2598, 1440);
        assert_eq!(docked_secondary_rect(slid, MONITOR), (2520, 0, 2560, 1440));
    }
}
