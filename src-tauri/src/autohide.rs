use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::taskbar::DetectedTaskbar;

// toggle_all_strips is the sole writer; true means the user explicitly hid the strips,
// so the poll below must skip entirely rather than auto-show them back.
static USER_HIDDEN: AtomicBool = AtomicBool::new(false);

// Set by a manual show; suppresses only the fullscreen hide condition, not
// taskbar_hidden, so the strip still tracks the taskbar per the dev's rule.
// Clears itself on the next change to the covered-monitor set, not on a timer.
static USER_FORCED_VISIBLE: AtomicBool = AtomicBool::new(false);

// 250ms rather than 500: this is also how fast the strip reappears when an
// auto-hide taskbar slides back in, and the taskbar's own animation is ~200ms.
const POLL_INTERVAL_MS: u64 = 250;

// The WinEventHook callback is extern "system" and can't capture state, so this is
// how every live strip's hwnd reaches it. Refreshed every poll tick.
#[cfg(target_os = "windows")]
static STRIP_HWNDS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

// Latest poll tick's covered-monitor snapshot, for other modules that need to know
// which monitor a fullscreen window is covering right now (e.g. moving a covered
// monitor's widgets to the uncovered one). Written once per tick by `spawn_poller`.
#[cfg(target_os = "windows")]
static COVERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn set_user_hidden(hidden: bool) {
    USER_HIDDEN.store(hidden, Ordering::SeqCst);
}

pub fn set_user_forced_visible(forced: bool) {
    USER_FORCED_VISIBLE.store(forced, Ordering::SeqCst);
}

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

/// Mirrors the taskbar's visibility when follow_taskbar is on, and re-asserts topmost
/// only when a strip actually lost the band or the taskbar climbed above it. Also folds
/// in `strip::reconcile` so a monitor hotplug creates/destroys strip windows: there is
/// no WM_DISPLAYCHANGE listener in this codebase, so blind re-polling is the mechanism.
pub fn spawn_poller(app: AppHandle) {
    #[cfg(target_os = "windows")]
    spawn_foreground_watcher();
    std::thread::spawn(move || {
        let mut prev_covered = covered_monitor_devices();
        store_covered(&prev_covered);
        // One poller thread, so a plain local beats a static: it tracks the effective
        // layout (settings' monitor_widgets plus any fullscreen relocation) this same
        // loop last emitted "widgets-changed" for, so that event fires on a real layout
        // change only, never once per 250ms tick.
        let mut prev_effective = crate::relocation::effective_from_app(&app);
        // Same change-only contract as prev_effective, for overlays: a relocated
        // overlay needs its window actually moved (overlay::reconcile), which
        // widgets-changed alone doesn't do - that event only makes a live overlay
        // re-read opacity/settings, never geometry (see overlay-main.ts's listener).
        let mut prev_effective_overlays = crate::relocation::effective_overlays_from_app(&app);
        loop {
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            // Tracked even while USER_HIDDEN, so a stale flip doesn't fire the
            // instant a later manual show sets USER_FORCED_VISIBLE.
            let covered = covered_monitor_devices();
            if covered != prev_covered {
                USER_FORCED_VISIBLE.store(false, Ordering::SeqCst);
            }
            store_covered(&covered);
            prev_covered = covered.clone();
            // Recomputed every tick (cheap map compare) since a relocation-relevant
            // change isn't only the covered set: move_on_fullscreen can flip from a
            // save_settings call this thread never otherwise observes.
            let effective = crate::relocation::effective_from_app(&app);
            if effective != prev_effective {
                let _ = app.emit("widgets-changed", ());
                prev_effective = effective;
            }
            let effective_overlays = crate::relocation::effective_overlays_from_app(&app);
            if effective_overlays != prev_effective_overlays {
                crate::overlay::reconcile(&app);
                prev_effective_overlays = effective_overlays;
            }
            crate::strip::reconcile(&app);
            if USER_HIDDEN.load(Ordering::SeqCst) {
                continue;
            }
            run_tick(&app, &covered);
        }
    });
}

#[cfg(target_os = "windows")]
fn store_covered(devices: &[String]) {
    if let Ok(mut guard) = COVERED.lock() {
        *guard = devices.to_vec();
    }
}

#[cfg(not(target_os = "windows"))]
fn store_covered(_devices: &[String]) {}

/// One tick: ONE `enumerate_taskbars()` Win32 pass, then every live strip is matched
/// against that in-memory `Vec<DetectedTaskbar>` via `taskbar_hidden_for` - no strip
/// re-enumerates, which is the N-times-the-polling-cost the plan forbids.
fn run_tick(app: &AppHandle, covered: &[String]) {
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
    // Effective (post-relocation) layout, so a strip whose own lane reads empty here
    // - emptied in Settings, or just relocated away - hides even though it isn't
    // docked-taskbar-hidden or fullscreen-covered. `monitor_key_for_label` resolves by
    // OS-primary identity, same convention `strip_monitor_key` uses for the frontend.
    let effective = crate::relocation::effective_from_app(app);
    let effective_keys: Vec<String> = effective.0.keys().cloned().collect();
    let os_primary = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());

    #[cfg(target_os = "windows")]
    let mut hwnds: Vec<isize> = Vec::new();

    for label in crate::strip::live_labels(app) {
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
        let monitor_key = crate::strip::monitor_key_for_label(&effective_keys, os_primary.as_deref(), &label);
        let empty_lane = crate::relocation::lane_is_present_and_empty(&effective, &monitor_key);
        let hide_needed =
            empty_lane || strip_should_hide(device.as_deref(), &taskbars, follow_taskbar, covered, forced_visible);
        let visible = win.is_visible().unwrap_or(true);
        if hide_needed && visible {
            let _ = win.hide();
            // A hidden strip still swallows clicks in its 468x48 band, which lands on top
            // of whatever fullscreen app replaced it. No unpark: the show() branch below
            // re-derives the position via reassert_position.
            crate::window_park::park(&win.as_ref().window());
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
/// (via the shared `taskbars` snapshot, never re-enumerated) OR `device` being in the
/// set of monitors a fullscreen window currently covers - covered, not merely
/// foreground, since the user can click over to a second monitor without the
/// fullscreen window losing its coverage of the first. `taskbar_hidden_for` is
/// windows-only, so off-windows the taskbar check is always false, same as this
/// crate's other off-windows Win32 stubs.
fn strip_should_hide(
    device: Option<&str>,
    taskbars: &[DetectedTaskbar],
    follow_taskbar: bool,
    covered: &[String],
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

    let hidden_by_fullscreen =
        !forced_visible && device.is_some_and(|d| covered.iter().any(|c| c == d));
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
) -> bool {
    // Progman/WorkerW (desktop) spans the whole monitor same as real fullscreen;
    // never treat the bare desktop as a reason to hide the strip.
    if matches!(class, "Progman" | "WorkerW") {
        return false;
    }
    if owner_pid == own_pid {
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

        is_fullscreen_state(&class_name, owner_pid, own_pid, presentation, covers_monitor)
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
fn is_covering_window(
    class: &str,
    owner_pid: u32,
    own_pid: u32,
    iconic: bool,
    cloaked: bool,
    covers_monitor: bool,
    ex_style: u32,
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

    if is_covering_window(&class_name, owner_pid, ctx.own_pid, iconic, cloaked, covers_monitor, ex_style) {
        ctx.covered.insert(wm.device_name);
    }
    1
}

/// The set of monitor device names currently covered by a fullscreen window, focused or
/// not - replaces the old foreground-only check so a video fullscreen on monitor 1
/// keeps covering it after the user clicks over to work on monitor 2.
#[cfg(target_os = "windows")]
fn covered_monitor_devices() -> Vec<String> {
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
fn covered_monitor_devices() -> Vec<String> {
    Vec::new()
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
        assert!(!strip_should_hide(Some("A"), &[], false, &[String::from("B")], false));
    }

    #[test]
    fn fullscreen_on_its_own_monitor_hides() {
        assert!(strip_should_hide(Some("A"), &[], false, &[String::from("A")], false));
    }

    #[test]
    fn two_monitors_covered_hides_both() {
        let covered = [String::from("A"), String::from("B")];
        assert!(strip_should_hide(Some("A"), &[], false, &covered, false));
        assert!(strip_should_hide(Some("B"), &[], false, &covered, false));
    }

    #[test]
    fn forced_visible_suppresses_the_fullscreen_hide() {
        assert!(!strip_should_hide(Some("A"), &[], false, &[String::from("A")], true));
    }

    #[test]
    fn unknown_device_never_matches_fullscreen() {
        assert!(!strip_should_hide(None, &[], false, &[String::from("A")], false));
    }

    #[test]
    fn follow_taskbar_off_ignores_taskbar_state() {
        assert!(!strip_should_hide(Some("A"), &[], false, &[], false));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn device_with_no_matching_taskbar_counts_as_hidden() {
        assert!(strip_should_hide(Some("A"), &[], true, &[], false));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn device_matching_an_invalid_hwnd_taskbar_is_hidden() {
        // hwnd 0 makes IsWindowVisible fail closed inside taskbar_hidden_for.
        let taskbars = [taskbar("A")];
        assert!(strip_should_hide(Some("A"), &taskbars, true, &[], false));
    }

    // Reproduces the strip ping-pong: clicking a tile makes the strip itself the
    // foreground window, and SHQueryUserNotificationState's busy/presentation bits can
    // be set from unrelated system state, not just from that window. Without the
    // own-pid guard this reads as "fullscreen", the strip hides, and the other
    // monitor's strip takes the activation next and hides in turn.
    #[test]
    fn own_process_foreground_is_never_fullscreen_even_under_presentation_state() {
        assert!(!is_fullscreen_state("StripWindowClass", 1234, 1234, true, true));
    }

    #[test]
    fn foreign_process_foreground_under_presentation_state_is_fullscreen() {
        assert!(is_fullscreen_state("ForeignWindowClass", 5555, 1234, true, false));
    }

    #[test]
    fn foreign_process_foreground_covering_monitor_is_fullscreen() {
        assert!(is_fullscreen_state("ForeignWindowClass", 5555, 1234, false, true));
    }

    #[test]
    fn foreign_process_foreground_with_neither_signal_is_not_fullscreen() {
        assert!(!is_fullscreen_state("ForeignWindowClass", 5555, 1234, false, false));
    }

    #[test]
    fn desktop_class_is_excluded_even_for_a_foreign_pid_under_presentation_state() {
        assert!(!is_fullscreen_state("Progman", 5555, 1234, true, true));
        assert!(!is_fullscreen_state("WorkerW", 5555, 1234, true, true));
    }

    // is_covering_window: the per-window filter behind covered_monitor_devices(). A
    // foreign, non-minimized, non-cloaked window whose rect covers its monitor is the
    // only case that counts; every exclusion below is a window that passes the
    // covers_monitor geometry test but still must not count.

    #[test]
    fn foreign_covering_window_counts() {
        assert!(is_covering_window("ForeignWindowClass", 5555, 1234, false, false, true, 0));
    }

    #[test]
    fn non_covering_window_does_not_count() {
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, false, false, false, 0));
    }

    #[test]
    fn cloaked_window_is_excluded_even_when_covering() {
        // A cloaked window sits on another virtual desktop or is a suspended UWP
        // frame: invisible in practice despite IsWindowVisible and a covering rect.
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, false, true, true, 0));
    }

    #[test]
    fn iconic_window_is_excluded_even_when_covering() {
        assert!(!is_covering_window("ForeignWindowClass", 5555, 1234, true, false, true, 0));
    }

    #[test]
    fn own_process_window_is_excluded_even_when_covering() {
        assert!(!is_covering_window("StripWindowClass", 1234, 1234, false, false, true, 0));
    }

    #[test]
    fn desktop_class_is_excluded_even_when_covering() {
        assert!(!is_covering_window("Progman", 5555, 1234, false, false, true, 0));
        assert!(!is_covering_window("WorkerW", 5555, 1234, false, false, true, 0));
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
        ));
    }
}
