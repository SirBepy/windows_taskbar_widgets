use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

static CURRENT_WIDGET: Mutex<Option<String>> = Mutex::new(None);

// Pins each poll loop to the open_flyout/close_flyout call that (in)validated it:
// a loop reads its own captured gen each tick and exits once POLL_GEN moves past it.
static POLL_GEN: AtomicU64 = AtomicU64::new(0);

const POLL_INTERVAL_MS: u64 = 150;
const COLD_CLOSE_MS: u64 = 400;
const HOVER_PAD_PX: i32 = 10;
const FLYOUT_GAP_CSS: f64 = 8.0;

#[derive(Clone, Serialize)]
struct FlyoutShow {
    widget_id: String,
}

#[derive(Clone, Copy)]
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
    widget_id: String,
    anchor_x_css: f64,
    width_css: f64,
    height_css: f64,
) -> Result<(), String> {
    let strip = app.get_webview_window("strip").ok_or("no strip window")?;
    let fly = app.get_webview_window("flyout").ok_or("no flyout window")?;
    let scale = strip.scale_factor().map_err(|e| e.to_string())?;

    let strip_pos = strip.outer_position().map_err(|e| e.to_string())?;
    let anchor_px = strip_pos.x + (anchor_x_css * scale).round() as i32;
    let w = (width_css * scale).round() as u32;
    let h = (height_css * scale).round() as u32;

    let top_of_taskbar = crate::taskbar::taskbar_rect()
        .map(|(_, top, _, _)| top)
        .unwrap_or(strip_pos.y);
    let mut x = anchor_px - w as i32 / 2;
    if let Ok(Some(monitor)) = fly.primary_monitor() {
        let mx = monitor.position().x;
        let mw = monitor.size().width as i32;
        x = x.clamp(mx, mx + mw - w as i32);
    }
    let y = top_of_taskbar - h as i32 - (FLYOUT_GAP_CSS * scale).round() as i32;

    fly.set_size(PhysicalSize::new(w, h)).map_err(|e| e.to_string())?;
    fly.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    if let Ok(mut cur) = CURRENT_WIDGET.lock() {
        *cur = Some(widget_id.clone());
    }
    app.emit_to("flyout", "flyout-show", FlyoutShow { widget_id })
        .map_err(|e| e.to_string())?;
    fly.show().map_err(|e| e.to_string())?;

    let gen = POLL_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    spawn_poll_loop(app, gen);
    Ok(())
}

#[tauri::command]
pub fn get_current_flyout_widget() -> Option<String> {
    CURRENT_WIDGET.lock().ok().and_then(|c| c.clone())
}

#[tauri::command]
pub fn close_flyout(app: AppHandle) {
    // Invalidate any running poll loop's captured gen so it exits on its next tick.
    POLL_GEN.fetch_add(1, Ordering::SeqCst);
    if let Some(fly) = app.get_webview_window("flyout") {
        let _ = fly.hide();
    }
}

/// Cursor-polls while the flyout is open: stays open as long as the pointer is
/// within either window's rect (padded so the strip-to-flyout gap doesn't
/// flicker-close), closes once cold for COLD_CLOSE_MS.
#[cfg(target_os = "windows")]
fn spawn_poll_loop(app: AppHandle, gen: u64) {
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
        loop {
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            if POLL_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let (Some(strip), Some(fly)) =
                (app.get_webview_window("strip"), app.get_webview_window("flyout"))
            else {
                return;
            };
            let hot = cursor_pos().is_some_and(|(x, y)| {
                window_rect(&strip).is_some_and(|r| r.padded(HOVER_PAD_PX).contains(x, y))
                    || window_rect(&fly).is_some_and(|r| r.padded(HOVER_PAD_PX).contains(x, y))
            });
            if hot {
                cold_since = None;
                continue;
            }
            let since = *cold_since.get_or_insert(Instant::now());
            if since.elapsed() >= Duration::from_millis(COLD_CLOSE_MS) {
                if POLL_GEN.load(Ordering::SeqCst) == gen {
                    let _ = fly.hide();
                }
                return;
            }
        }
    });
}

// No cursor-polling API available off-Windows: just close on a flat timer so the
// flyout doesn't stay open forever.
#[cfg(not(target_os = "windows"))]
fn spawn_poll_loop(app: AppHandle, gen: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        if POLL_GEN.load(Ordering::SeqCst) == gen {
            if let Some(fly) = app.get_webview_window("flyout") {
                let _ = fly.hide();
            }
        }
    });
}
