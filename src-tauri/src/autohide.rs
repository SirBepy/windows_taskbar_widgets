use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::monitor_widgets::MonitorWidgets;
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

// Re-exported so existing callers (relocation.rs, overlay.rs) keep working at
// `crate::autohide::covered_devices()` even though the fullscreen-coverage detector
// that backs it now lives in its own module.
pub(crate) use crate::fullscreen_coverage::covered_devices;

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
        let mut prev_covered = crate::fullscreen_coverage::covered_monitor_devices();
        crate::fullscreen_coverage::store_covered(&prev_covered);
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
            let covered = crate::fullscreen_coverage::covered_monitor_devices();
            if covered != prev_covered {
                USER_FORCED_VISIBLE.store(false, Ordering::SeqCst);
            }
            crate::fullscreen_coverage::store_covered(&covered);
            prev_covered = covered.clone();
            // Recomputed every tick (cheap map compare) since a relocation-relevant
            // change isn't only the covered set: move_on_fullscreen can flip from a
            // save_settings call this thread never otherwise observes. Computed once
            // here and shared with strip::reconcile_with/run_tick below, rather than
            // each recomputing its own settings lock plus monitor enumeration.
            let effective = crate::relocation::effective_from_app(&app);
            if effective != prev_effective {
                let _ = app.emit("widgets-changed", ());
                prev_effective = effective.clone();
            }
            let effective_overlays = crate::relocation::effective_overlays_from_app(&app);
            if effective_overlays != prev_effective_overlays {
                crate::overlay::reconcile_with(&app, effective_overlays.clone());
                prev_effective_overlays = effective_overlays;
            }
            crate::strip::reconcile_with(&app, &effective);
            if USER_HIDDEN.load(Ordering::SeqCst) {
                continue;
            }
            run_tick(&app, &covered, &effective);
        }
    });
}

/// One tick: ONE `enumerate_taskbars()` Win32 pass, then every live strip is matched
/// against that in-memory `Vec<DetectedTaskbar>` via `taskbar_hidden_for` - no strip
/// re-enumerates, which is the N-times-the-polling-cost the plan forbids. `effective`
/// is `spawn_poller`'s own layout for this same tick, passed in rather than
/// recomputed here, for the same reason.
fn run_tick(app: &AppHandle, covered: &[String], effective: &MonitorWidgets) {
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
        let empty_lane = crate::relocation::lane_is_present_and_empty(effective, &monitor_key);
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
}
