use tauri::{AppHandle, Manager};

/// One EnumWindows-detected taskbar: its own window plus the monitor it's docked to.
/// `device_name` is the stable key (survives HMONITOR/enum-index churn across
/// sessions); see settings.rs's `taskbar_monitor` field for why it's used as the key.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedTaskbar {
    pub hwnd: isize,
    pub device_name: String,
    pub is_primary: bool,
    pub taskbar_rect: (i32, i32, i32, i32),
    pub monitor_rect: (i32, i32, i32, i32),
}

/// Resolves `wanted` (a device_name, or "" for "whatever is primary") against the
/// currently detected taskbars. Falls back to primary when `wanted` isn't present -
/// e.g. its monitor got unplugged - without ever touching the caller's saved setting.
pub fn select_taskbar<'a>(
    taskbars: &'a [DetectedTaskbar],
    wanted: &str,
) -> Option<&'a DetectedTaskbar> {
    if !wanted.is_empty() {
        if let Some(t) = taskbars.iter().find(|t| t.device_name == wanted) {
            return Some(t);
        }
    }
    taskbars.iter().find(|t| t.is_primary).or_else(|| taskbars.first())
}

/// The app's configured `taskbar_monitor` setting, resolved against a fresh
/// enumeration (never cached, same "re-find every call" property FindWindowW had).
pub fn selected_taskbar(app: &AppHandle) -> Option<DetectedTaskbar> {
    let wanted = app
        .try_state::<crate::settings::SettingsState>()
        .and_then(|s| s.0.lock().ok().map(|s| s.taskbar_monitor.clone()))
        .unwrap_or_default();
    select_taskbar(&enumerate_taskbars(), &wanted).cloned()
}

#[cfg(target_os = "windows")]
pub(crate) fn is_taskbar_class(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;
    unsafe {
        let mut class = [0u16; 32];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        matches!(
            String::from_utf16_lossy(&class[..len]).as_str(),
            "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_taskbar_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    let found = &mut *(lparam as *mut Vec<isize>);
    if is_taskbar_class(hwnd) {
        found.push(hwnd as isize);
    }
    1
}

/// Windows creates one Shell_TrayWnd (primary) plus one Shell_SecondaryTrayWnd per
/// secondary display; this is the only place both are enumerated together.
#[cfg(target_os = "windows")]
pub fn enumerate_taskbars() -> Vec<DetectedTaskbar> {
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut found: Vec<isize> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_taskbar_proc), &mut found as *mut _ as isize);
    }
    found.into_iter().filter_map(|h| detect_taskbar(h as _)).collect()
}

#[cfg(not(target_os = "windows"))]
pub fn enumerate_taskbars() -> Vec<DetectedTaskbar> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn detect_taskbar(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<DetectedTaskbar> {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;

    let wm = super::rect::window_and_monitor_rect(hwnd, MONITOR_DEFAULTTONEAREST)?;
    Some(DetectedTaskbar {
        hwnd: hwnd as isize,
        device_name: wm.device_name,
        is_primary: wm.is_primary,
        taskbar_rect: (wm.window.left, wm.window.top, wm.window.right, wm.window.bottom),
        monitor_rect: (wm.monitor.left, wm.monitor.top, wm.monitor.right, wm.monitor.bottom),
    })
}

/// Settings picker option: one row per detected taskbar, labeled with its monitor's
/// resolution so "Primary - 2560x1440" reads correctly even on a single-monitor box.
#[derive(Clone, serde::Serialize)]
pub struct TaskbarMonitorOption {
    pub device_name: String,
    pub is_primary: bool,
    pub width: i32,
    pub height: i32,
}

#[tauri::command]
pub fn list_taskbar_monitors() -> Vec<TaskbarMonitorOption> {
    enumerate_taskbars()
        .into_iter()
        .map(|t| TaskbarMonitorOption {
            width: t.monitor_rect.2 - t.monitor_rect.0,
            height: t.monitor_rect.3 - t.monitor_rect.1,
            is_primary: t.is_primary,
            device_name: t.device_name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taskbar(device_name: &str, is_primary: bool) -> DetectedTaskbar {
        DetectedTaskbar {
            hwnd: 0,
            device_name: device_name.to_string(),
            is_primary,
            taskbar_rect: (0, 1400, 2560, 1440),
            monitor_rect: (0, 0, 2560, 1440),
        }
    }

    #[test]
    fn empty_wanted_selects_primary() {
        let taskbars = [taskbar(r"\\.\DISPLAY2", false), taskbar(r"\\.\DISPLAY1", true)];
        let picked = select_taskbar(&taskbars, "").unwrap();
        assert_eq!(picked.device_name, r"\\.\DISPLAY1");
    }

    #[test]
    fn matching_device_name_is_selected_even_when_not_primary() {
        let taskbars = [taskbar(r"\\.\DISPLAY1", true), taskbar(r"\\.\DISPLAY2", false)];
        let picked = select_taskbar(&taskbars, r"\\.\DISPLAY2").unwrap();
        assert_eq!(picked.device_name, r"\\.\DISPLAY2");
        assert!(!picked.is_primary);
    }

    #[test]
    fn unplugged_monitor_falls_back_to_primary() {
        let taskbars = [taskbar(r"\\.\DISPLAY1", true)];
        let picked = select_taskbar(&taskbars, r"\\.\DISPLAY2").unwrap();
        assert_eq!(picked.device_name, r"\\.\DISPLAY1");
    }

    #[test]
    fn single_monitor_default_picks_the_only_taskbar() {
        let taskbars = [taskbar(r"\\.\DISPLAY1", true)];
        let picked = select_taskbar(&taskbars, "").unwrap();
        assert_eq!(picked.device_name, r"\\.\DISPLAY1");
    }

    #[test]
    fn no_taskbars_selects_nothing() {
        let taskbars: [DetectedTaskbar; 0] = [];
        assert!(select_taskbar(&taskbars, "").is_none());
    }

    #[test]
    fn no_primary_flagged_falls_back_to_first() {
        let taskbars = [taskbar(r"\\.\DISPLAY1", false), taskbar(r"\\.\DISPLAY2", false)];
        let picked = select_taskbar(&taskbars, "").unwrap();
        assert_eq!(picked.device_name, r"\\.\DISPLAY1");
    }
}
