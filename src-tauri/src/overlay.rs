use crate::settings::{OverlaySpec, Placement, SettingsState};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{
    AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

pub const LABEL_PREFIX: &str = "overlay-";

// Only reached by a hand-written placement; the settings UI always writes real dims.
const FALLBACK_W: f64 = 280.0;
const FALLBACK_H: f64 = 180.0;
// Keeps a dragged overlay grabbable: this much always stays inside the monitor.
const MIN_VISIBLE: f64 = 48.0;

/// Window label -> widget id. Labels are sanitized, so the mapping is not reversible.
#[derive(Default)]
pub struct OverlayState(pub Mutex<HashMap<String, String>>);

pub fn new_state() -> OverlayState {
    OverlayState::default()
}

// Tauri documents window labels as alphanumeric, and ids may carry a colon (divider:<uuid>).
fn label_for(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{LABEL_PREFIX}{safe}")
}

fn monitor_for(app: &AppHandle, name: &str) -> Option<Monitor> {
    if !name.is_empty() {
        if let Ok(monitors) = app.available_monitors() {
            if let Some(m) = monitors.into_iter().find(|m| m.name().is_some_and(|n| n == name)) {
                return Some(m);
            }
        }
    }
    app.primary_monitor().ok().flatten()
}

/// Physical rect for a spec, clamped so MIN_VISIBLE css px always stay on the monitor.
fn geometry(app: &AppHandle, spec: &OverlaySpec) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let m = monitor_for(app, &spec.monitor)?;
    let scale = m.scale_factor();
    let (mw, mh) = (m.size().width as f64 / scale, m.size().height as f64 / scale);
    let w = spec.w.unwrap_or(FALLBACK_W).max(1.0).min(mw);
    let h = spec.h.unwrap_or(FALLBACK_H).max(1.0).min(mh);
    let x = spec.x.clamp(MIN_VISIBLE - w, mw - MIN_VISIBLE);
    let y = spec.y.clamp(0.0, mh - MIN_VISIBLE);
    Some((
        PhysicalPosition::new(
            m.position().x + (x * scale).round() as i32,
            m.position().y + (y * scale).round() as i32,
        ),
        PhysicalSize::new((w * scale).round() as u32, (h * scale).round() as u32),
    ))
}

fn build(app: &AppHandle, id: &str, label: &str, spec: &OverlaySpec) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, label, WebviewUrl::App("overlay.html".into()))
        .title("Widget")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(true)
        .visible(false)
        .build()?;
    if let Ok(mut map) = app.state::<OverlayState>().0.lock() {
        map.insert(label.to_string(), id.to_string());
    }
    apply_geometry(app, &win, spec);
    let excluded = app
        .state::<SettingsState>()
        .0
        .lock()
        .map(|s| s.hide_from_capture)
        .unwrap_or(false);
    let _ = tauri_kit_window::exclude_from_capture(&win, excluded);
    let _ = win.show();
    Ok(())
}

fn apply_geometry(app: &AppHandle, win: &tauri::WebviewWindow, spec: &OverlaySpec) {
    if let Some((pos, size)) = geometry(app, spec) {
        let _ = win.set_size(size);
        let _ = win.set_position(pos);
    }
}

/// Creates, moves, resizes and closes overlay windows so the live set matches settings.
/// One path for startup, a settings change, and a drag, so they cannot drift apart.
pub fn reconcile(app: &AppHandle) {
    // Logged, not silent: a skipped reconcile used to look identical to "there was
    // nothing to do", which cost a full debugging session on 2026-08-08.
    let desired: Vec<(String, OverlaySpec)> = match app.state::<SettingsState>().0.lock() {
        Ok(s) => s.overlays(),
        Err(e) => {
            log::error!("overlay reconcile skipped, settings lock poisoned: {e}");
            return;
        }
    };
    log::info!("overlay reconcile: {} wanted", desired.len());
    let wanted: Vec<(String, String, OverlaySpec)> = desired
        .into_iter()
        .map(|(id, spec)| (label_for(&id), id, spec))
        .collect();

    for (label, win) in app.webview_windows() {
        if label.starts_with(LABEL_PREFIX) && !wanted.iter().any(|(l, _, _)| *l == label) {
            let _ = win.close();
            if let Ok(mut map) = app.state::<OverlayState>().0.lock() {
                map.remove(&label);
            }
        }
    }

    for (label, id, spec) in &wanted {
        match app.get_webview_window(label) {
            Some(win) => apply_geometry(app, &win, spec),
            None => {
                log::info!("overlay {id}: building {label}");
                if let Err(e) = build(app, id, label, spec) {
                    log::error!("overlay {id}: {e}");
                }
            }
        }
    }
}

/// The calling overlay window's widget id, so the page can mount the right widget.
#[tauri::command]
pub fn overlay_widget_id(window: tauri::Window, state: tauri::State<OverlayState>) -> Option<String> {
    state.0.lock().ok()?.get(window.label()).cloned()
}

/// Persists a drag or resize WITHOUT emitting widgets-changed: the window is already
/// where the user put it, and a re-render mid-drag would fight them.
#[tauri::command]
pub fn save_overlay_geometry(
    app: AppHandle,
    id: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    monitor: String,
) -> Result<(), String> {
    let settings = {
        let state = app.state::<SettingsState>();
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        let opacity = match s.widget_placement.get(&id) {
            Some(Placement::Overlay(prev)) => prev.opacity,
            _ => None,
        };
        s.widget_placement.insert(
            id,
            Placement::Overlay(OverlaySpec { monitor, x, y, w: Some(w), h: Some(h), opacity }),
        );
        s.clone()
    };
    crate::settings::persist(&app, &settings)
}

/// Origin, size and scale of a monitor, so the frontend can turn a physical window
/// position into the monitor-relative CSS coords settings stores.
#[derive(serde::Serialize)]
pub struct MonitorInfo {
    name: String,
    x: i32,
    y: i32,
    width: f64,
    height: f64,
    scale: f64,
}

#[tauri::command]
pub fn monitor_at_point(app: AppHandle, x: i32, y: i32) -> Option<MonitorInfo> {
    let m = app
        .monitor_from_point(x as f64, y as f64)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let scale = m.scale_factor();
    Some(MonitorInfo {
        name: m.name().cloned().unwrap_or_default(),
        x: m.position().x,
        y: m.position().y,
        width: m.size().width as f64 / scale,
        height: m.size().height as f64 / scale,
        scale,
    })
}
