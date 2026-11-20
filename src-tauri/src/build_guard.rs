//! Self-heal for a window build that never returns. `WebviewWindowBuilder::build()` runs
//! on the event loop's own thread inside `drain_pending`, and WebView2 creation
//! occasionally never completes (measured 2026-11-20: a secondary strip built one second
//! after an autostart launch logged "building" and never "booted"). The build then sits in
//! a nested message pump forever, so the event loop never ticks again: no tiles render,
//! tray Quit does nothing, and the process has to be killed by hand. The root cause is
//! inside WebView2, so this does not try to prevent it; it bounds the damage by relaunching
//! the app once a build has been in flight past `STALL_LIMIT_MS`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A healthy WebView2 build returns in about a second; this is far past any slow-but-alive
/// machine (a cold logon with every autostart app loading) so a live build is never killed.
const STALL_LIMIT_MS: u64 = 45_000;
const POLL_SECS: u64 = 5;
/// Gives the old process time to die before the new one starts, or the single-instance
/// plugin would see the old one still alive and exit the new one.
const RELAUNCH_DELAY_PINGS: u32 = 4;

/// Epoch-ms of the build now in flight, 0 when none is.
static BUILD_STARTED_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Pure: has a build that started at `started_ms` (0 = none in flight) outlived `limit_ms`.
fn is_stalled(started_ms: u64, now_ms: u64, limit_ms: u64) -> bool {
    started_ms != 0 && now_ms.saturating_sub(started_ms) > limit_ms
}

/// Marks a build in flight until dropped. Held across the whole `build()` call.
pub struct BuildGuard;

impl BuildGuard {
    pub fn begin() -> Self {
        BUILD_STARTED_MS.store(now_ms(), Ordering::SeqCst);
        BuildGuard
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        BUILD_STARTED_MS.store(0, Ordering::SeqCst);
    }
}

pub fn spawn() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));
        let started = BUILD_STARTED_MS.load(Ordering::SeqCst);
        if is_stalled(started, now_ms(), STALL_LIMIT_MS) {
            log::error!(
                "build_guard: a window build has been in flight for over {}s, the event loop \
                 is wedged - relaunching the app",
                STALL_LIMIT_MS / 1000
            );
            relaunch_and_exit();
        }
    });
}

#[cfg(target_os = "windows")]
fn relaunch_and_exit() -> ! {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Ok(exe) = std::env::current_exe() {
        let script = format!(
            "ping -n {} 127.0.0.1 >nul & start \"\" \"{}\"",
            RELAUNCH_DELAY_PINGS,
            exe.display()
        );
        let _ = std::process::Command::new("cmd").args(["/C", &script]).creation_flags(CREATE_NO_WINDOW).spawn();
    }
    std::process::exit(1);
}

#[cfg(not(target_os = "windows"))]
fn relaunch_and_exit() -> ! {
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_build_in_flight_is_never_stalled() {
        assert!(!is_stalled(0, u64::MAX, 1_000));
    }

    #[test]
    fn stalls_only_past_the_limit() {
        assert!(!is_stalled(1_000, 1_500, 1_000));
        assert!(!is_stalled(1_000, 2_000, 1_000));
        assert!(is_stalled(1_000, 2_001, 1_000));
    }

    #[test]
    fn a_clock_step_backwards_is_not_a_stall() {
        assert!(!is_stalled(5_000, 1_000, 1_000));
    }

    #[test]
    fn guard_marks_in_flight_and_clears_on_drop() {
        {
            let _g = BuildGuard::begin();
            assert_ne!(BUILD_STARTED_MS.load(Ordering::SeqCst), 0);
        }
        assert_eq!(BUILD_STARTED_MS.load(Ordering::SeqCst), 0);
    }
}
