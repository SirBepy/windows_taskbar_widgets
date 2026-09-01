use crate::pending_queue::{prune_pending, Labeled, PendingQueue};
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
    // Bracketing the builder call specifically: todo 46's failure was WebviewWindowBuilder
    // never returning, which is indistinguishable from build() never being entered.
    log::info!("overlay {label}: builder start");
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
    log::info!("overlay {label}: builder returned");
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
    if let Err(e) = win.show() {
        log::error!("overlay {label}: show failed: {e}");
    }
    log::info!("overlay {label}: built, visible={:?}", win.is_visible());
    Ok(())
}

// Every result here was discarded, so a window left at the 800x600 WebView2 default
// looked identical to one that was never asked to move (todo 46).
fn apply_geometry(app: &AppHandle, win: &tauri::WebviewWindow, spec: &OverlaySpec) {
    let label = win.label();
    let Some((pos, size)) = geometry(app, spec) else {
        log::error!("overlay {label}: no monitor for {:?}, keeping default geometry", spec.monitor);
        return;
    };
    if let Err(e) = win.set_size(size) {
        log::error!("overlay {label}: set_size {size:?} failed: {e}");
    }
    if let Err(e) = win.set_position(pos) {
        log::error!("overlay {label}: set_position {pos:?} failed: {e}");
    }
}

/// Creates, moves, resizes and closes overlay windows so the live set matches settings.
/// One path for startup, a settings change, and a drag, so they cannot drift apart.
pub fn reconcile(app: &AppHandle) {
    // Logged, not silent: a skipped reconcile used to look identical to "there was
    // nothing to do", which cost a full debugging session on 2026-08-08.
    let desired: Vec<(String, String, OverlaySpec)> = match app.state::<SettingsState>().0.lock() {
        Ok(s) => s.overlays(),
        Err(e) => {
            log::error!("overlay reconcile skipped, settings lock poisoned: {e}");
            return;
        }
    };
    log::info!("overlay reconcile: {} wanted", desired.len());
    // label_for(instance_id), not widget_id: two placements of one widget kind
    // must become two distinct windows.
    let wanted: Vec<(String, String, String, OverlaySpec)> = desired
        .into_iter()
        .map(|(instance_id, widget_id, spec)| (label_for(&instance_id), instance_id, widget_id, spec))
        .collect();

    let wanted_labels: Vec<String> = wanted.iter().map(|(l, _, _, _)| l.clone()).collect();
    for (label, win) in app.webview_windows() {
        if label.starts_with(LABEL_PREFIX) && !wanted_labels.contains(&label) {
            let _ = win.close();
            if let Ok(mut map) = app.state::<OverlayState>().0.lock() {
                map.remove(&label);
            }
        }
    }
    // Same pass, so one place decides what is wanted: a window already built is closed
    // above, a build still queued is dropped here. Without this, a placement hidden
    // before the next tick is still built, then only closed by some later reconcile.
    prune_pending(&mut PENDING.lock(), &wanted_labels);

    for (label, instance_id, widget_id, spec) in &wanted {
        match app.get_webview_window(label) {
            Some(win) => apply_geometry(app, &win, spec),
            None => {
                log::info!("overlay {instance_id}: queued build of {label}");
                queue_build(instance_id, widget_id, label, spec);
            }
        }
    }
}

/// A queued build: instance_id, widget_id, window label, geometry.
type Pending = (String, String, String, OverlaySpec);

impl Labeled for Pending {
    fn label(&self) -> &str {
        &self.2
    }
}

/// Overlay windows waiting to be built, drained by `drain_pending` on the event loop's
/// own tick. Deduped by label: reconcile can run again before a tick drains the queue.
static PENDING: PendingQueue<Pending> = PendingQueue::new("overlay");

/// Pure: queue `entry`, replacing any earlier queue of the same label. Replace, not skip:
/// an overlay moved or resized between two ticks would otherwise build at the first
/// queued geometry and stay there until an unrelated settings change re-applied it.
fn upsert_pending(queue: &mut Vec<Pending>, entry: Pending) {
    match queue.iter_mut().find(|e| e.label() == entry.label()) {
        Some(slot) => *slot = entry,
        None => queue.push(entry),
    }
}

fn queue_build(instance_id: &str, widget_id: &str, label: &str, spec: &OverlaySpec) {
    let entry =
        (instance_id.to_string(), widget_id.to_string(), label.to_string(), spec.clone());
    upsert_pending(&mut PENDING.lock(), entry);
}

/// Builds the queued windows. Called from `RunEvent::MainEventsCleared`, i.e. between the
/// event loop's dispatches. Building from inside one instead - which is where
/// `run_on_main_thread` puts a task queued by `save_settings` - never returned, left the
/// webview stranded on about:blank, and froze the whole app (todo 46, measured 2026-09-01).
pub fn drain_pending(app: &AppHandle) {
    for (instance_id, widget_id, label, spec) in PENDING.take() {
        if app.get_webview_window(&label).is_some() {
            continue;
        }
        log::info!("overlay {instance_id}: building {label}");
        if let Err(e) = build(app, &widget_id, &label, &spec) {
            log::error!("overlay {label}: {e}");
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
    let mut settings = {
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
    crate::settings::persist(&app, &mut settings)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, x: f64) -> Pending {
        let spec = OverlaySpec { x, ..OverlaySpec::default() };
        ("cpu#1".to_string(), "cpu".to_string(), label.to_string(), spec)
    }

    #[test]
    fn upsert_pending_appends_a_label_not_yet_queued() {
        let mut queue = vec![entry("overlay-a", 0.0)];
        upsert_pending(&mut queue, entry("overlay-b", 0.0));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[1].2, "overlay-b");
    }

    #[test]
    fn upsert_pending_replaces_the_spec_of_an_already_queued_label() {
        let mut queue = vec![entry("overlay-a", 10.0)];
        upsert_pending(&mut queue, entry("overlay-a", 250.0));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].3.x, 250.0);
    }

    #[test]
    fn prune_pending_drops_labels_no_longer_wanted() {
        let mut queue = vec![entry("overlay-a", 0.0), entry("overlay-b", 0.0)];
        prune_pending(&mut queue, &["overlay-b".to_string()]);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].2, "overlay-b");
    }

    #[test]
    fn prune_pending_with_nothing_wanted_empties_the_queue() {
        let mut queue = vec![entry("overlay-a", 0.0)];
        prune_pending(&mut queue, &[]);
        assert!(queue.is_empty());
    }
}
