use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

/// Primary-taskbar rect in physical px via SHAppBarMessage; None off-Windows or if the call fails.
#[cfg(target_os = "windows")]
pub fn taskbar_rect() -> Option<(i32, i32, i32, i32)> {
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

#[cfg(not(target_os = "windows"))]
pub fn taskbar_rect() -> Option<(i32, i32, i32, i32)> {
    None
}

// An auto-hide taskbar slid off screen still leaves a ~2px sliver behind, so
// "is it on screen" has to be a real overlap test, not a nonzero one.
#[cfg(target_os = "windows")]
const SLIVER_PX: i32 = 8;

/// True when the taskbar is not on screen right now. Deliberately checks the LIVE
/// window, not ABM_GETSTATE's auto-hide flag: that flag reports the mode, so it
/// stays set while the user hovers and the taskbar is actually slid in and visible.
#[cfg(target_os = "windows")]
pub fn taskbar_hidden() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowRect, IsWindowVisible,
    };

    unsafe {
        let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
        let tray = FindWindowW(class.as_ptr(), std::ptr::null());
        // Explorer restarting destroys and recreates Shell_TrayWnd; re-finding it
        // every call is what lets the strip come back on its own afterwards.
        if tray.is_null() || IsWindowVisible(tray) == 0 {
            return true;
        }

        let mut rc: RECT = std::mem::zeroed();
        let monitor = MonitorFromWindow(tray, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetWindowRect(tray, &mut rc) == 0 || GetMonitorInfoW(monitor, &mut mi) == 0 {
            return false;
        }

        let overlap_h = rc.bottom.min(mi.rcMonitor.bottom) - rc.top.max(mi.rcMonitor.top);
        let overlap_w = rc.right.min(mi.rcMonitor.right) - rc.left.max(mi.rcMonitor.left);
        overlap_h.min(overlap_w) < SLIVER_PX
    }
}

#[cfg(not(target_os = "windows"))]
pub fn taskbar_hidden() -> bool {
    false
}

/// Left-anchor the strip over the taskbar's empty left region (Win11 centers
/// pinned icons, so the left edge is free). Falls back to bottom-left of the
/// work area when the taskbar rect is unavailable.
pub fn position_strip(app: &AppHandle, strip_css_width: f64) -> tauri::Result<()> {
    let Some(win) = app.get_webview_window("strip") else {
        return Ok(());
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    let settings = app.state::<crate::settings::SettingsState>();
    let left_margin = settings.0.lock().map(|s| s.left_margin).unwrap_or(12) as f64;

    let w = (strip_css_width * scale).round() as u32;
    if let Some((left, top, _, bottom)) = taskbar_rect() {
        let h = (bottom - top).max(1) as u32;
        let x = left + (left_margin * scale).round() as i32;
        win.set_size(PhysicalSize::new(w.max(1), h))?;
        win.set_position(PhysicalPosition::new(x, top))?;
    } else if let Ok(Some(monitor)) = win.primary_monitor() {
        let h = (48.0 * scale).round() as u32;
        let wa = monitor.work_area();
        let x = wa.position.x + (left_margin * scale) as i32;
        let y = wa.position.y + wa.size.height as i32 - h as i32;
        win.set_size(PhysicalSize::new(w.max(1), h))?;
        win.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}
