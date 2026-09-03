//! Parks hidden windows off every monitor: a hidden Tauri window still swallows every
//! click inside its rect, because its WebView2 composition layer keeps claiming the
//! region after `hide()`. Measured 2026-09-03; see "A hidden window is not an absent
//! window" in CLAUDE.md for the full account.

use std::sync::Mutex;
use tauri::{PhysicalPosition, Window};

/// Far outside any real virtual desktop, so no monitor can contain it.
const PARK: i32 = -32000;

/// Where each parked window sat before it moved, by label. Vec, not HashMap: `new` must be const.
static PARKED_FROM: Mutex<Vec<(String, PhysicalPosition<i32>)>> = Mutex::new(Vec::new());

/// Moves `win` off-screen, remembering where it was. Call after `hide()`, never before:
/// a still-visible window would slide across the screen.
pub fn park(win: &Window) {
    let Ok(pos) = win.outer_position() else { return };
    if pos.x <= PARK {
        return;
    }
    if let Ok(mut saved) = PARKED_FROM.lock() {
        let label = win.label().to_string();
        match saved.iter_mut().find(|(l, _)| *l == label) {
            Some(slot) => slot.1 = pos,
            None => saved.push((label, pos)),
        }
    }
    let _ = win.set_position(PhysicalPosition::new(PARK, PARK));
}

/// Restores the position `park` saved. Call before `show()`. A no-op for a window that
/// positions itself on every open, like the flyout.
pub fn unpark(win: &Window) {
    let saved = PARKED_FROM
        .lock()
        .ok()
        .and_then(|s| s.iter().find(|(l, _)| l == win.label()).map(|(_, p)| *p));
    if let Some(pos) = saved {
        let _ = win.set_position(pos);
    }
}
