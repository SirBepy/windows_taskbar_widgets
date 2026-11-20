//! The flyout is ONE transparent window that spans the strip's width and stays put while it
//! is open. The visible panel inside it (`#flyout` in flyout.html) moves, resizes and swaps
//! content in CSS, so every change animates on the compositor instead of resizing a native
//! window per frame (which stutters on WebView2). The window itself only ever grows while
//! open, and is click-through everywhere except over the panel.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, Window};

/// The panel the frontend should show, in the flyout window's own CSS pixels.
#[derive(Clone, Serialize)]
pub struct FlyoutShow {
    widget_id: String,
    left_css: f64,
    width_css: f64,
    height_css: f64,
}

/// Geometry of the open flyout, in physical pixels.
#[derive(Clone, Copy)]
struct PanelState {
    /// Where the panel is heading. The previous target is folded into `hot` instead of kept,
    /// because the panel is still sliding out of it for `COLD_CLOSE_MS`-scale time.
    hot: Rect,
    /// Window height. Grows with the tallest panel since open and shrinks only on close, so
    /// a tall panel animating down to a short one is never clipped by a shrinking window.
    win_h: i32,
}

static CURRENT: Mutex<Option<FlyoutShow>> = Mutex::new(None);
static PANEL: Mutex<Option<PanelState>> = Mutex::new(None);
// The strip label open_flyout last anchored to. Needed since decision 3 lets any
// strip host the flyout now, not just the primary: autohide.rs reads this to close
// the flyout only when ITS anchoring strip is the one that hid.
static CURRENT_ANCHOR: Mutex<Option<String>> = Mutex::new(None);

// Pins each poll loop to the open_flyout/close_flyout call that (in)validated it:
// a loop reads its own captured gen each tick and exits once POLL_GEN moves past it.
static POLL_GEN: AtomicU64 = AtomicU64::new(0);

// 40ms, not 150: this loop also flips click-through, and a pointer crossing the panel edge
// must not click through the panel for a visible beat.
const POLL_INTERVAL_MS: u64 = 40;
const COLD_CLOSE_MS: u64 = 400;
const HOVER_PAD_PX: i32 = 10;
const CLICK_PAD_PX: i32 = 2;
const FLYOUT_GAP_CSS: f64 = 8.0;
/// How far past the strip's edges the window reaches. At least the widest flyout, so the
/// panel can sit centred on the first or last tile without leaving the window.
const WINDOW_REACH_CSS: f64 = 340.0;
/// Backstop for a frontend that never reports its close animation done: the window is hidden
/// anyway, since a visible click-through window is harmless but a stuck one is not.
const FORCE_HIDE_MS: u64 = 900;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn padded(self, pad: i32) -> Rect {
        Rect {
            left: self.left - pad,
            top: self.top - pad,
            right: self.right + pad,
            bottom: self.bottom + pad,
        }
    }

    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    fn union(self, o: Rect) -> Rect {
        Rect {
            left: self.left.min(o.left),
            top: self.top.min(o.top),
            right: self.right.max(o.right),
            bottom: self.bottom.max(o.bottom),
        }
    }
}

/// Pure: the flyout window's horizontal extent. The strip's own span plus `reach` on each
/// side, clamped to its monitor, so any panel position a tile can produce fits inside.
fn window_span(mon_left: i32, mon_right: i32, strip_left: i32, strip_right: i32, reach: i32) -> (i32, i32) {
    ((strip_left - reach).max(mon_left), (strip_right + reach).min(mon_right))
}

fn window_rect(win: &WebviewWindow) -> Option<Rect> {
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    Some(Rect {
        left: pos.x,
        top: pos.y,
        right: pos.x + size.width as i32,
        bottom: pos.y + size.height as i32,
    })
}

#[tauri::command]
pub fn open_flyout(
    app: AppHandle,
    window: Window,
    widget_id: String,
    anchor_x_css: f64,
    width_css: f64,
    height_css: f64,
) -> Result<(), String> {
    // Tauri injects the calling window (whichever strip's tile was hovered), same
    // pattern as tile_menu.rs's show_tile_menu. Anchor against it, not a fixed label.
    let fly = app.get_webview_window("flyout").ok_or("no flyout window")?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let was_open = is_open();

    let strip_pos = window.outer_position().map_err(|e| e.to_string())?;
    let strip_w = window.outer_size().map(|s| s.width as i32).unwrap_or(0);
    let anchor_px = strip_pos.x + (anchor_x_css * scale).round() as i32;
    let w = (width_css * scale).round() as i32;
    let h = (height_css * scale).round() as i32;

    // Both of these resolve against the CALLING strip's own monitor, never the
    // `taskbar_monitor` setting's: a flyout opened from a secondary strip was
    // otherwise clamped back onto the selected monitor, off the tile it belongs to.
    let top_of_taskbar = crate::taskbar::taskbar_rect_for_window(&app, &window)
        .map(|(_, top, _, _)| top)
        .unwrap_or(strip_pos.y);
    let (mon_left, mon_right) = match window.current_monitor() {
        Ok(Some(m)) => (m.position().x, m.position().x + m.size().width as i32),
        _ => (strip_pos.x, strip_pos.x + strip_w),
    };
    let x = (anchor_px - w / 2).clamp(mon_left, (mon_right - w).max(mon_left));
    let panel_bottom = top_of_taskbar - (FLYOUT_GAP_CSS * scale).round() as i32;
    let panel = Rect { left: x, top: panel_bottom - h, right: x + w, bottom: panel_bottom };

    let reach = (WINDOW_REACH_CSS * scale).round() as i32;
    let (win_left, win_right) = window_span(mon_left, mon_right, strip_pos.x, strip_pos.x + strip_w, reach);
    let prev = PANEL.lock().ok().and_then(|p| *p).filter(|_| was_open);
    let win_h = prev.map_or(h, |p| p.win_h.max(h));
    let hot = prev.map_or(panel, |p| p.hot.union(panel));

    fly.set_size(PhysicalSize::new((win_right - win_left).max(w) as u32, win_h as u32))
        .map_err(|e| e.to_string())?;
    fly.set_position(PhysicalPosition::new(win_left, panel_bottom - win_h))
        .map_err(|e| e.to_string())?;

    let show = FlyoutShow {
        widget_id,
        left_css: (x - win_left) as f64 / scale,
        width_css,
        height_css,
    };
    if let Ok(mut p) = PANEL.lock() {
        *p = Some(PanelState { hot, win_h });
    }
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = Some(show.clone());
    }
    if let Ok(mut anchor) = CURRENT_ANCHOR.lock() {
        *anchor = Some(window.label().to_string());
    }
    app.emit_to("flyout", "flyout-show", show).map_err(|e| e.to_string())?;
    fly.show().map_err(|e| e.to_string())?;

    // Bumping POLL_GEN here is also what makes a second open_flyout from a different
    // strip safe: any loop from a prior open reads its stale gen next tick and exits,
    // so only ever one loop runs, watching whichever strip is current.
    let gen = POLL_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    spawn_poll_loop(app, gen, window.label().to_string());
    Ok(())
}

/// The panel to show right now, for a page that loaded after the `flyout-show` emit.
#[tauri::command]
pub fn get_current_flyout() -> Option<FlyoutShow> {
    CURRENT.lock().ok().and_then(|c| c.clone())
}

/// True while a flyout is open (its close animation does not count).
pub fn is_open() -> bool {
    CURRENT.lock().map(|c| c.is_some()).unwrap_or(false)
}

#[tauri::command]
pub fn flyout_is_open() -> bool {
    is_open()
}

/// True while the flyout is open and anchored to strip window `label`.
pub fn is_anchored_to(label: &str) -> bool {
    CURRENT_ANCHOR.lock().map(|c| c.as_deref() == Some(label)).unwrap_or(false)
}

/// Starts closing: the page animates the panel away, then reports `flyout_hide_done`. A hidden
/// webview keeps running its JS, so the page's own unmount on `flyout-hide` is what stops the
/// widget's polling; see flyout-mount.ts.
fn hide(app: &AppHandle) {
    if !is_open() {
        return;
    }
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = None;
    }
    if let Ok(mut anchor) = CURRENT_ANCHOR.lock() {
        *anchor = None;
    }
    let _ = app.emit_to("flyout", "flyout-hide", ());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(FORCE_HIDE_MS));
        finish_hide(&app);
    });
}

/// The only path that takes the window off screen. A no-op while a flyout is open again, so a
/// late `flyout_hide_done` or the backstop timer never hides a panel the user reopened.
fn finish_hide(app: &AppHandle) {
    if is_open() {
        return;
    }
    if let Ok(mut p) = PANEL.lock() {
        *p = None;
    }
    if let Some(fly) = app.get_webview_window("flyout") {
        let _ = fly.hide();
        crate::window_park::park(&fly.as_ref().window());
    }
}

#[tauri::command]
pub fn flyout_hide_done(app: AppHandle) {
    finish_hide(&app);
}

#[tauri::command]
pub fn close_flyout(app: AppHandle) {
    // Invalidate any running poll loop's captured gen so it exits on its next tick.
    POLL_GEN.fetch_add(1, Ordering::SeqCst);
    hide(&app);
}

/// Cursor-polls while the flyout is open: stays open as long as the pointer is
/// within the strip or the panel (padded so the strip-to-flyout gap doesn't
/// flicker-close), closes once cold for COLD_CLOSE_MS. Also makes the window
/// click-through except over the panel, since the window is far bigger than it.
#[cfg(target_os = "windows")]
fn spawn_poll_loop(app: AppHandle, gen: u64, strip_label: String) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    fn cursor_pos() -> Option<(i32, i32)> {
        unsafe {
            let mut pt: POINT = std::mem::zeroed();
            if GetCursorPos(&mut pt) == 0 {
                return None;
            }
            Some((pt.x, pt.y))
        }
    }

    std::thread::spawn(move || {
        let mut cold_since: Option<Instant> = None;
        let mut click_through: Option<bool> = None;
        loop {
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            if POLL_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let (Some(strip), Some(fly)) =
                (app.get_webview_window(&strip_label), app.get_webview_window("flyout"))
            else {
                return;
            };
            let panel = PANEL.lock().ok().and_then(|p| *p).map(|p| p.hot);
            let cursor = cursor_pos();
            let over_panel = cursor
                .zip(panel)
                .is_some_and(|((x, y), r)| r.padded(CLICK_PAD_PX).contains(x, y));
            if click_through != Some(!over_panel) {
                click_through = Some(!over_panel);
                let _ = fly.set_ignore_cursor_events(!over_panel);
            }
            let hot = cursor.is_some_and(|(x, y)| {
                window_rect(&strip).is_some_and(|r| r.padded(HOVER_PAD_PX).contains(x, y))
                    || panel.is_some_and(|r| r.padded(HOVER_PAD_PX).contains(x, y))
            });
            if hot {
                cold_since = None;
                continue;
            }
            let since = *cold_since.get_or_insert(Instant::now());
            if since.elapsed() >= Duration::from_millis(COLD_CLOSE_MS) {
                if POLL_GEN.load(Ordering::SeqCst) == gen {
                    hide(&app);
                }
                return;
            }
        }
    });
}

// No cursor-polling API available off-Windows: just close on a flat timer so the
// flyout doesn't stay open forever.
#[cfg(not(target_os = "windows"))]
fn spawn_poll_loop(app: AppHandle, gen: u64, _strip_label: String) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        if POLL_GEN.load(Ordering::SeqCst) == gen {
            hide(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect { left: 10, top: 20, right: 110, bottom: 220 }
    }

    #[test]
    fn padded_grows_every_edge_by_pad() {
        let r = rect().padded(5);
        assert_eq!((r.left, r.top, r.right, r.bottom), (5, 15, 115, 225));
    }

    #[test]
    fn padded_negative_shrinks() {
        let r = rect().padded(-5);
        assert_eq!((r.left, r.top, r.right, r.bottom), (15, 25, 105, 215));
    }

    #[test]
    fn contains_inside_point() {
        assert!(rect().contains(50, 100));
    }

    #[test]
    fn contains_left_top_edge_is_inclusive() {
        assert!(rect().contains(10, 20));
    }

    #[test]
    fn contains_right_bottom_edge_is_exclusive() {
        assert!(!rect().contains(110, 100));
        assert!(!rect().contains(50, 220));
    }

    #[test]
    fn contains_outside_point_is_false() {
        assert!(!rect().contains(0, 0));
        assert!(!rect().contains(200, 300));
    }

    #[test]
    fn union_covers_both_rects() {
        let other = Rect { left: 100, top: 0, right: 300, bottom: 50 };
        assert_eq!(rect().union(other), Rect { left: 10, top: 0, right: 300, bottom: 220 });
    }

    #[test]
    fn union_with_itself_is_unchanged() {
        assert_eq!(rect().union(rect()), rect());
    }

    #[test]
    fn window_span_reaches_past_the_strip_on_both_sides() {
        assert_eq!(window_span(0, 2560, 12, 412, 340), (0, 752));
        assert_eq!(window_span(0, 2560, 800, 1200, 340), (460, 1540));
    }

    #[test]
    fn window_span_is_clamped_to_the_monitor() {
        assert_eq!(window_span(2560, 4000, 2572, 2972, 340), (2560, 3312));
        assert_eq!(window_span(0, 600, 100, 500, 340), (0, 600));
    }

    #[test]
    fn window_span_on_a_negative_origin_monitor() {
        // Joe's portrait secondary monitor sits at a negative Y but its X can be negative too.
        assert_eq!(window_span(-1440, 0, -1428, -1100, 340), (-1440, -760));
    }
}
