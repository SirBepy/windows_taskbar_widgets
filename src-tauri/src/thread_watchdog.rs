//! Emergency safety net for the runaway-thread bug recorded in
//! `.claude/todos/79-live-thread-count-explosion-after-multi-day-uptime.md`. Root cause
//! is still unconfirmed, so this does not try to fix anything: it just caps the blast
//! radius. A process with tens of thousands of threads drags down the whole machine,
//! not just itself, so once this crosses a threshold that is never legitimate for this
//! app, it is better to log loudly and self-exit than to let it climb unchecked again.

use std::time::Duration;

const POLL_SECS: u64 = 30;
// This app's normal steady-state is well under 100 threads (one per poller/bridge/
// flyout-open, plus WebView2's own). 500 is already an order of magnitude over that.
const WARN_THRESHOLD: u32 = 500;
// Observed growth during the 2026-09-24 incident was ~100-200 new threads/second once
// it started; 3000 catches it within seconds of onset while staying far above any
// plausible legitimate peak (e.g. several overlay windows open at once).
const EMERGENCY_THRESHOLD: u32 = 3000;

pub fn spawn() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));
        let Some(count) = own_thread_count() else { continue };
        if count >= EMERGENCY_THRESHOLD {
            log::error!(
                "thread_watchdog: {count} threads, at or past the emergency ceiling \
                 ({EMERGENCY_THRESHOLD}) - exiting now rather than letting a runaway \
                 leak keep growing. See todo 79."
            );
            std::process::exit(1);
        } else if count >= WARN_THRESHOLD {
            log::warn!("thread_watchdog: {count} threads (>= warn threshold {WARN_THRESHOLD})");
        }
    });
}

#[cfg(target_os = "windows")]
fn own_thread_count() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let pid = GetCurrentProcessId();
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut count = 0u32;
        if Thread32First(snap, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    count += 1;
                }
                if Thread32Next(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        Some(count)
    }
}

#[cfg(not(target_os = "windows"))]
fn own_thread_count() -> Option<u32> {
    None
}
