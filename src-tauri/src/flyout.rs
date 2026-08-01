use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

static CURRENT_WIDGET: Mutex<Option<String>> = Mutex::new(None);

// Hover state for the close choreography: the flyout stays open while the pointer
// is over EITHER the strip tile or the flyout itself; both cold for 300ms closes it.
pub static STRIP_HOT: AtomicBool = AtomicBool::new(false);
pub static FLYOUT_HOT: AtomicBool = AtomicBool::new(false);
static CLOSE_GEN: AtomicU64 = AtomicU64::new(0);

const CLOSE_GRACE_MS: u64 = 300;
const FLYOUT_GAP_CSS: f64 = 8.0;

#[derive(Clone, Serialize)]
struct FlyoutShow {
    widget_id: String,
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
    CLOSE_GEN.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn get_current_flyout_widget() -> Option<String> {
    CURRENT_WIDGET.lock().ok().and_then(|c| c.clone())
}

#[tauri::command]
pub fn close_flyout(app: AppHandle) {
    if let Some(fly) = app.get_webview_window("flyout") {
        let _ = fly.hide();
    }
}

#[tauri::command]
pub fn flyout_zone(app: AppHandle, zone: String, inside: bool) {
    match zone.as_str() {
        "strip" => STRIP_HOT.store(inside, Ordering::SeqCst),
        "flyout" => FLYOUT_HOT.store(inside, Ordering::SeqCst),
        _ => return,
    }
    if !STRIP_HOT.load(Ordering::SeqCst) && !FLYOUT_HOT.load(Ordering::SeqCst) {
        let gen = CLOSE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(CLOSE_GRACE_MS));
            let still_cold =
                !STRIP_HOT.load(Ordering::SeqCst) && !FLYOUT_HOT.load(Ordering::SeqCst);
            if still_cold && CLOSE_GEN.load(Ordering::SeqCst) == gen {
                close_flyout(app);
            }
        });
    }
}
