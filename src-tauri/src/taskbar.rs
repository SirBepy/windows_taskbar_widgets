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
