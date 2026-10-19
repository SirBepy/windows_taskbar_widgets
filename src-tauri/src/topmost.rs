use std::sync::Mutex;

// The WinEventHook callback is extern "system" and can't capture state, so this is
// how every live strip's hwnd reaches it. Refreshed every poll tick.
#[cfg(target_os = "windows")]
static STRIP_HWNDS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

#[cfg(target_os = "windows")]
pub(crate) fn store_strip_hwnds(hwnds: Vec<isize>) {
    if let Ok(mut guard) = STRIP_HWNDS.lock() {
        *guard = hwnds;
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn store_strip_hwnds(_hwnds: Vec<isize>) {}

/// Explorer restarts and fullscreen transitions silently demote a topmost window,
/// and Windows never restores it; re-asserting is the only way back up.
#[cfg(target_os = "windows")]
pub(crate) fn raise_topmost(win: &tauri::WebviewWindow) {
    let Ok(hwnd) = win.hwnd() else { return };
    raise_topmost_hwnd(hwnd.0 as isize);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn raise_topmost(_win: &tauri::WebviewWindow) {}

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
pub(crate) fn spawn_foreground_watcher() {
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
