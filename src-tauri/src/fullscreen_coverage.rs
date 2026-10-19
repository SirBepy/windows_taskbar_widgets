use std::sync::Mutex;

use crate::excluded_processes::is_excluded_process_exe;
#[cfg(target_os = "windows")]
use crate::excluded_processes::process_exe_name;

// Latest poll tick's covered-monitor snapshot, for other modules that need to know
// which monitor a fullscreen window is covering right now (e.g. moving a covered
// monitor's widgets to the uncovered one). Written once per tick by `spawn_poller`.
#[cfg(target_os = "windows")]
static COVERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Sorted so callers can compare snapshots by equality rather than set semantics.
/// `relocation::effective_from_app` reads this to decide which monitors' widgets
/// move onto another monitor's strip.
#[cfg(target_os = "windows")]
pub(crate) fn covered_devices() -> Vec<String> {
    COVERED.lock().map(|g| g.clone()).unwrap_or_default()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn covered_devices() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "windows")]
pub(crate) fn store_covered(devices: &[String]) {
    if let Ok(mut guard) = COVERED.lock() {
        *guard = devices.to_vec();
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn store_covered(_devices: &[String]) {}

/// Pulled out of `foreground_presentation_device` so the decision is a plain value
/// comparison a test can drive without mocking Win32. Own-process windows (a strip, the
/// flyout, an overlay, Settings) never count: clicking a tile makes that strip itself
/// the foreground window, and without this guard `SHQueryUserNotificationState`'s
/// busy/presentation bits (which can be set while any fullscreen or busy state exists
/// anywhere, not just under the foreground window) made that strip read as "fullscreen
/// on its own monitor", hiding it; Windows then handed activation to the other
/// monitor's strip, which read as fullscreen in turn, and the two strips hid each other
/// back and forth until the user focused a different app.
fn is_fullscreen_state(
    class: &str,
    owner_pid: u32,
    own_pid: u32,
    presentation: bool,
    covers_monitor: bool,
    exe_name: Option<&str>,
) -> bool {
    // Progman/WorkerW (desktop) spans the whole monitor same as real fullscreen;
    // never treat the bare desktop as a reason to hide the strip.
    if matches!(class, "Progman" | "WorkerW") {
        return false;
    }
    if owner_pid == own_pid {
        return false;
    }
    if exe_name.is_some_and(is_excluded_process_exe) {
        return false;
    }
    presentation || covers_monitor
}

/// SHQueryUserNotificationState catches D3D/presentation/busy fullscreen; it misses
/// some borderless fullscreen, so fall back to foreground-window-covers-monitor. Both
/// branches need the foreground window's monitor to attribute which strip should hide,
/// so a state hit with no resolvable foreground window is treated as no fullscreen.
/// This only sees the foreground window; `covered_monitor_devices` unions this in
/// alongside the covers-rect pass over every top-level window, since a D3D/presentation
/// fullscreen window can run a few px short of the monitor rect and miss that pass.
#[cfg(target_os = "windows")]
fn foreground_presentation_device() -> Option<String> {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return None;
        }
        let mut class = [0u16; 256];
        let len = GetClassNameW(fg, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class_name = String::from_utf16_lossy(&class[..len]);

        let mut owner_pid = 0u32;
        GetWindowThreadProcessId(fg, &mut owner_pid);
        let own_pid = GetCurrentProcessId();

        let wm = crate::taskbar::window_and_monitor_rect(fg, MONITOR_DEFAULTTONULL)?;

        let mut state = 0;
        let presentation = SHQueryUserNotificationState(&mut state) == 0
            && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE | QUNS_BUSY);
        let covers_monitor = wm.window.left <= wm.monitor.left
            && wm.window.top <= wm.monitor.top
            && wm.window.right >= wm.monitor.right
            && wm.window.bottom >= wm.monitor.bottom;

        let exe_name = (presentation || covers_monitor).then(|| process_exe_name(owner_pid)).flatten();

        is_fullscreen_state(
            &class_name,
            owner_pid,
            own_pid,
            presentation,
            covers_monitor,
            exe_name.as_deref(),
        )
        .then_some(wm.device_name)
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_presentation_device() -> Option<String> {
    None
}

/// Per-window filter for the monitor-coverage enumeration, pure so a test can drive it
/// without mocking Win32. Minimized and cloaked windows pass `IsWindowVisible` but
/// render nothing - a cloaked window sits on another virtual desktop or is a suspended
/// UWP frame - so neither can actually cover a monitor despite matching the geometry
/// and class checks below.
#[allow(clippy::too_many_arguments)]
fn is_covering_window(
    class: &str,
    owner_pid: u32,
    own_pid: u32,
    iconic: bool,
    cloaked: bool,
    covers_monitor: bool,
    ex_style: u32,
    exe_name: Option<&str>,
) -> bool {
    if iconic || cloaked {
        return false;
    }
    // Progman/WorkerW (desktop) spans the whole monitor same as real fullscreen;
    // never treat the bare desktop as covering it.
    if matches!(class, "Progman" | "WorkerW") {
        return false;
    }
    if owner_pid == own_pid {
        return false;
    }
    // A real fullscreen app is never click-through (WS_EX_TRANSPARENT) or a tool
    // window (WS_EX_TOOLWINDOW); both are the signature of a permanent full-monitor
    // overlay instead (GeForce/ShadowPlay capture overlay, Steam/Discord in-game
    // overlay, a screen dimmer) that would otherwise hide the strip forever.
    if ex_style & (WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW) != 0 {
        return false;
    }
    if exe_name.is_some_and(is_excluded_process_exe) {
        return false;
    }
    covers_monitor
}

#[cfg(target_os = "windows")]
const WS_EX_TRANSPARENT: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT;
#[cfg(target_os = "windows")]
const WS_EX_TOOLWINDOW: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW;
#[cfg(not(target_os = "windows"))]
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
#[cfg(not(target_os = "windows"))]
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;

// DWMWA_CLOAKED, the DWMWINDOWATTRIBUTE enum's cloak flag (dwmapi.h). This crate's
// windows-sys feature list doesn't enable Win32_Graphics_Dwm, so this is a direct
// dwmapi.dll extern rather than a new Cargo feature pulled in for one constant.
#[cfg(target_os = "windows")]
const DWMWA_CLOAKED: u32 = 14;

#[cfg(target_os = "windows")]
#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(
        hwnd: windows_sys::Win32::Foundation::HWND,
        dw_attribute: u32,
        pv_attribute: *mut core::ffi::c_void,
        cb_attribute: u32,
    ) -> i32;
}

#[cfg(target_os = "windows")]
fn window_is_cloaked(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    let mut cloaked: u32 = 0;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        ) == 0
            && cloaked != 0
    }
}

/// EnumWindows context: `own_pid` is resolved once per tick rather than per window, and
/// `covered` collects device names as a set since several windows can share a monitor.
#[cfg(target_os = "windows")]
struct CoverageCtx {
    own_pid: u32,
    covered: std::collections::HashSet<String>,
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_coverage_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    use windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowLongW, GetWindowThreadProcessId, GWL_EXSTYLE, IsIconic,
        IsWindowVisible,
    };

    // Skip invisible windows before any further Win32 calls: this runs over every
    // top-level window on a 250ms poll tick, and most of them are invisible.
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let ctx = &mut *(lparam as *mut CoverageCtx);

    let mut owner_pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut owner_pid);

    let mut class_buf = [0u16; 256];
    let len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32).max(0) as usize;
    let class_name = String::from_utf16_lossy(&class_buf[..len]);

    let iconic = IsIconic(hwnd) != 0;
    let cloaked = window_is_cloaked(hwnd);

    let Some(wm) = crate::taskbar::window_and_monitor_rect(hwnd, MONITOR_DEFAULTTONULL) else {
        return 1;
    };
    let covers_monitor = wm.window.left <= wm.monitor.left
        && wm.window.top <= wm.monitor.top
        && wm.window.right >= wm.monitor.right
        && wm.window.bottom >= wm.monitor.bottom;

    // Only read once a window has survived every cheaper check above - GWL_EXSTYLE is
    // one more Win32 call per candidate, not per every top-level window.
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

    // Rarer still, and 3 syscalls (OpenProcess+Query+CloseHandle): only resolved for
    // a window that actually covers its monitor, where it decides a real exclusion.
    let exe_name = covers_monitor.then(|| process_exe_name(owner_pid)).flatten();

    if is_covering_window(
        &class_name,
        owner_pid,
        ctx.own_pid,
        iconic,
        cloaked,
        covers_monitor,
        ex_style,
        exe_name.as_deref(),
    ) {
        ctx.covered.insert(wm.device_name);
    }
    1
}

/// The set of monitor device names currently covered by a fullscreen window, focused or
/// not - replaces the old foreground-only check so a video fullscreen on monitor 1
/// keeps covering it after the user clicks over to work on monitor 2.
#[cfg(target_os = "windows")]
pub(crate) fn covered_monitor_devices() -> Vec<String> {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

    let mut ctx = CoverageCtx {
        own_pid: unsafe { GetCurrentProcessId() },
        covered: std::collections::HashSet::new(),
    };
    unsafe {
        EnumWindows(Some(enum_coverage_proc), &mut ctx as *mut _ as isize);
    }
    if let Some(device) = foreground_presentation_device() {
        ctx.covered.insert(device);
    }
    let mut devices: Vec<String> = ctx.covered.into_iter().collect();
    devices.sort();
    devices
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn covered_monitor_devices() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reproduces the strip ping-pong: clicking a tile makes the strip itself the
    // foreground window, and SHQueryUserNotificationState's busy/presentation bits can
    // be set from unrelated system state, not just from that window. Without the
    // own-pid guard this reads as "fullscreen", the strip hides, and the other
    // monitor's strip takes the activation next and hides in turn.
    #[test]
    fn own_process_foreground_is_never_fullscreen_even_under_presentation_state() {
        assert!(!is_fullscreen_state("StripWindowClass", 1234, 1234, true, true, None));
    }

    #[test]
    fn foreign_process_foreground_under_presentation_state_is_fullscreen() {
        assert!(is_fullscreen_state("ForeignWindowClass", 5555, 1234, true, false, None));
    }

    #[test]
    fn foreign_process_foreground_covering_monitor_is_fullscreen() {
        assert!(is_fullscreen_state("ForeignWindowClass", 5555, 1234, false, true, None));
    }

    #[test]
    fn foreign_process_foreground_with_neither_signal_is_not_fullscreen() {
        assert!(!is_fullscreen_state("ForeignWindowClass", 5555, 1234, false, false, None));
    }

    #[test]
    fn desktop_class_is_excluded_even_for_a_foreign_pid_under_presentation_state() {
        assert!(!is_fullscreen_state("Progman", 5555, 1234, true, true, None));
        assert!(!is_fullscreen_state("WorkerW", 5555, 1234, true, true, None));
    }

    // The Start menu / Search / Game Bar / the built-in screen recorder can present a
    // full-monitor rect and presentation-state bits during ordinary use; neither
    // should ever relocate a strip (reported 2026-10-19).

    #[test]
    fn start_menu_foreground_under_presentation_state_is_not_fullscreen() {
        assert!(!is_fullscreen_state(
            "Windows.UI.Core.CoreWindow",
            5555,
            1234,
            true,
            true,
            Some("StartMenuExperienceHost.exe"),
        ));
    }

    #[test]
    fn excluded_exe_name_match_is_case_insensitive() {
        assert!(!is_fullscreen_state(
            "Windows.UI.Core.CoreWindow",
            5555,
            1234,
            true,
            true,
            Some("startmenuexperiencehost.EXE"),
        ));
    }

    // is_covering_window: the per-window filter behind covered_monitor_devices(). A
    // foreign, non-minimized, non-cloaked window whose rect covers its monitor is the
    // only case that counts; every exclusion below is a window that passes the
    // covers_monitor geometry test but still must not count.

    #[test]
    fn foreign_covering_window_counts() {
        assert!(is_covering_window("ForeignWindowClass", 5555, 1234, false, false, true, 0, None));
    }

    #[test]
    fn non_covering_window_does_not_count() {
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, false, false, false, 0, None));
    }

    #[test]
    fn cloaked_window_is_excluded_even_when_covering() {
        // A cloaked window sits on another virtual desktop or is a suspended UWP
        // frame: invisible in practice despite IsWindowVisible and a covering rect.
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, false, true, true, 0, None));
    }

    #[test]
    fn iconic_window_is_excluded_even_when_covering() {
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, true, false, true, 0, None));
    }

    #[test]
    fn own_process_window_is_excluded_even_when_covering() {
        assert!(!is_covering_window("StripWindowClass", 1234, 1234, false, false, true, 0, None));
    }

    #[test]
    fn desktop_class_is_excluded_even_when_covering() {
        assert!(!is_covering_window("Progman", 5555, 1234, false, false, true, 0, None));
        assert!(!is_covering_window("WorkerW", 5555, 1234, false, false, true, 0, None));
    }

    // Start menu / Search / Game Bar / the built-in screen recorder: see the
    // is_fullscreen_state exclusion tests above for the reported symptom.

    #[test]
    fn start_menu_covering_window_does_not_count() {
        assert!(!is_covering_window(
            "Windows.UI.Core.CoreWindow",
            5555,
            1234,
            false,
            false,
            true,
            0,
            Some("StartMenuExperienceHost.exe"),
        ));
    }

    #[test]
    fn screen_recorder_covering_window_does_not_count() {
        assert!(!is_covering_window(
            "ApplicationFrameWindow",
            5555,
            1234,
            false,
            false,
            true,
            0,
            Some("ScreenClippingHost.exe"),
        ));
    }

    #[test]
    fn unrelated_exe_covering_window_still_counts() {
        assert!(is_covering_window(
            "ForeignWindowClass",
            5555,
            1234,
            false,
            false,
            true,
            0,
            Some("SomeGame.exe"),
        ));
    }

    // Real fullscreen apps never carry WS_EX_TRANSPARENT (click-through) or
    // WS_EX_TOOLWINDOW: both are the signature of a permanent overlay (GeForce/
    // ShadowPlay capture overlay, Steam/Discord in-game overlay, a screen dimmer) that
    // sits full-monitor and visible indefinitely. Counting those as "covering" would
    // hide the strip forever on a desktop that never shows anything fullscreen.

    #[test]
    fn covering_click_through_window_does_not_count() {
        assert!(!is_covering_window(
            "ForeignWindowClass",
            5555,
            1234,
            false,
            false,
            true,
            WS_EX_TRANSPARENT,
            None,
        ));
    }

    #[test]
    fn covering_tool_window_does_not_count() {
        assert!(!is_covering_window(
            "ForeignWindowClass",
            5555,
            1234,
            false,
            false,
            true,
            WS_EX_TOOLWINDOW,
            None,
        ));
    }
}
