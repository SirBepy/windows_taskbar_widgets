use crate::monitor_widgets::MonitorWidgets;
use crate::pending_queue::{prune_pending, Labeled, PendingQueue};
use crate::settings::SettingsState;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL_PREFIX: &str = "strip-";
/// The primary monitor's window keeps this pre-existing static label so the
/// single-monitor case has zero behaviour change.
pub const PRIMARY_LABEL: &str = "strip";

static LAST_WANTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Window label -> device_name, mirroring `overlay::OverlayState`.
#[derive(Default)]
pub struct StripState(pub Mutex<HashMap<String, String>>);

pub fn new_state() -> StripState {
    StripState::default()
}

// Same rule as overlay.rs's label_for, copied verbatim (it's private there):
// every non-alphanumeric char becomes '-'. Never yields the bare "strip" label.
pub fn label_for(device_name: &str) -> String {
    let safe: String = device_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{LABEL_PREFIX}{safe}")
}

/// Creates a strip window at runtime for a secondary monitor. Chrome flags mirror the
/// static "strip" window declared in tauri.conf.json exactly.
pub fn build(app: &AppHandle, device_name: &str, label: &str) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Widgets")
        .inner_size(320.0, 48.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .shadow(false)
        .visible(false)
        .build()?;
    if let Ok(mut map) = app.state::<StripState>().0.lock() {
        map.insert(label.to_string(), device_name.to_string());
    }
    Ok(())
}

/// Every live strip window: the bare primary label plus any runtime secondary
/// (`LABEL_PREFIX`) window `reconcile` has built so far.
pub fn live_labels(app: &AppHandle) -> Vec<String> {
    app.webview_windows()
        .into_keys()
        .filter(|l| l == PRIMARY_LABEL || l.starts_with(LABEL_PREFIX))
        .collect()
}

/// Pure: which window label a monitor's content shows on. Mirrors
/// `wanted_strip_labels`'s primary rule exactly (empty key, or a device name that
/// happens to be the live primary, both mean the bare label) so the two can never
/// disagree about which window a given device's widgets land in.
pub fn resolve_label_for_monitor(monitor: &str, primary_name: Option<&str>) -> String {
    if monitor.is_empty() || Some(monitor) == primary_name {
        PRIMARY_LABEL.to_string()
    } else {
        label_for(monitor)
    }
}

/// `resolve_label_for_monitor` with the live primary looked up from the AppHandle.
pub fn label_for_monitor(app: &AppHandle, monitor: &str) -> String {
    let primary_name = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    resolve_label_for_monitor(monitor, primary_name.as_deref())
}

/// Pure reverse of `resolve_label_for_monitor`: which `monitor_widgets` key's content
/// belongs on window `label`. Never parses `label` (sanitized, not reversible) -
/// re-runs the forward resolution over every key instead, preferring a non-empty
/// match over `""` (today's single-monitor default) when both resolve to `label`.
pub fn monitor_key_for_label(keys: &[String], primary_name: Option<&str>, label: &str) -> String {
    let matched: Vec<&String> =
        keys.iter().filter(|k| resolve_label_for_monitor(k, primary_name) == label).collect();
    matched
        .iter()
        .find(|k| !k.is_empty())
        .or(matched.first())
        .map(|k| k.to_string())
        .unwrap_or_default()
}

/// The calling strip window's `monitor_widgets` key, so the page can render only the
/// instances placed on its own monitor. See `monitor_key_for_label` for the resolution
/// rule; unmatched (a window whose monitor unplugged) falls back to "", today's key.
#[tauri::command]
pub fn strip_monitor_key(app: AppHandle, window: tauri::Window) -> String {
    let keys: Vec<String> = match app.state::<SettingsState>().0.lock() {
        Ok(s) => s.monitor_widgets.0.keys().cloned().collect(),
        Err(e) => {
            log::error!("strip_monitor_key: settings lock poisoned: {e}");
            return String::new();
        }
    };
    let primary_name = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    monitor_key_for_label(&keys, primary_name.as_deref(), window.label())
}

/// Wanted strip labels for the live monitor set. The primary always maps to the bare
/// `strip` label regardless of instances (zero change for single-monitor); every other
/// live monitor maps to `label_for(device)` only if it has instances. A `monitor_widgets`
/// key with no matching live monitor is never visited, so it produces no label.
pub fn wanted_strip_labels(widgets: &MonitorWidgets, live_monitors: &[(String, bool)]) -> Vec<String> {
    live_monitors
        .iter()
        .filter_map(|(name, is_primary)| {
            if *is_primary {
                Some(PRIMARY_LABEL.to_string())
            } else if !widgets.instances_for(name).is_empty() {
                Some(label_for(name))
            } else {
                None
            }
        })
        .collect()
}

/// Live monitor (name, is_primary) pairs for `wanted_strip_labels`. Kept local to
/// this module: `taskbar::monitors` builds the same shape for a different purpose
/// (`resolve_live_monitor_key`) and isn't touched by this step.
fn live_monitors(app: &AppHandle) -> Vec<(String, bool)> {
    let Ok(monitors) = app.available_monitors() else { return Vec::new() };
    let primary_name = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    monitors
        .into_iter()
        .filter_map(|m| m.name().cloned())
        .map(|name| {
            let is_primary = Some(&name) == primary_name.as_ref();
            (name, is_primary)
        })
        .collect()
}

/// Creates and closes strip windows so the live set matches `wanted_strip_labels`.
/// Mirrors `overlay::reconcile`. `PRIMARY_LABEL` never starts with `LABEL_PREFIX` so
/// the close loop already excludes it by construction; the explicit check below is
/// belt-and-braces since closing the static `strip` window would break single-instance.
pub fn reconcile(app: &AppHandle) {
    let widgets = match app.state::<SettingsState>().0.lock() {
        Ok(s) => s.monitor_widgets.clone(),
        Err(e) => {
            log::error!("strip reconcile skipped, settings lock poisoned: {e}");
            return;
        }
    };
    let live = live_monitors(app);
    let wanted = wanted_strip_labels(&widgets, &live);
    // Logged only on change: reconcile runs inside autohide's 250ms tick, so an
    // unconditional line here writes ~4/sec and grows app.log unbounded under
    // KeepAll rotation - measured live on 2026-08-12 at 121 of 128 lines.
    if LAST_WANTED.lock().map(|mut l| std::mem::replace(&mut *l, wanted.clone()) != wanted).unwrap_or(true) {
        log::info!("strip reconcile: {} wanted", wanted.len());
    }

    for (label, win) in app.webview_windows() {
        if label != PRIMARY_LABEL && label.starts_with(LABEL_PREFIX) && !wanted.contains(&label) {
            let _ = win.close();
            if let Ok(mut map) = app.state::<StripState>().0.lock() {
                map.remove(&label);
            }
        }
    }
    // Same pass as the close loop above, so one place decides what is wanted: a built
    // window is closed there, a build still queued is dropped here.
    prune_pending(&mut PENDING.lock(), &wanted);

    for label in &wanted {
        if label == PRIMARY_LABEL || app.get_webview_window(label).is_some() {
            continue;
        }
        let Some(device_name) =
            live.iter().find(|(name, _)| &label_for(name) == label).map(|(name, _)| name.clone())
        else {
            continue;
        };
        // Queued, not hopped through run_on_main_thread: reconcile runs off the main
        // thread (250ms poller, save_settings) and a build() dispatched into one of the
        // event loop's own dispatches never returns (todo 46). Logged on first queue
        // only - the poller would otherwise write this line 4x/sec.
        if queue_build(&device_name, label) {
            log::info!("strip {device_name}: queued build of {label}");
        }
    }
}

/// A queued build: device_name, window label.
type Pending = (String, String);

impl Labeled for Pending {
    fn label(&self) -> &str {
        &self.1
    }
}

/// Strip windows waiting to be built, drained by `drain_pending` on the event loop's own tick.
static PENDING: PendingQueue<Pending> = PendingQueue::new("strip");

/// Pure: queue `entry` unless its label is already queued. Returns whether it was added.
/// Unlike overlay's, a strip entry carries no geometry, so a re-queue has nothing to update.
fn upsert_pending(queue: &mut Vec<Pending>, entry: Pending) -> bool {
    if queue.iter().any(|e| e.label() == entry.label()) {
        return false;
    }
    queue.push(entry);
    true
}

fn queue_build(device_name: &str, label: &str) -> bool {
    upsert_pending(&mut PENDING.lock(), (device_name.to_string(), label.to_string()))
}

/// Builds the queued strip windows on the event loop's own tick, called from
/// `RunEvent::MainEventsCleared`. See `overlay::drain_pending` for why the build cannot
/// happen inside a dispatch instead.
pub fn drain_pending(app: &AppHandle) {
    for (device_name, label) in PENDING.take() {
        if app.get_webview_window(&label).is_some() {
            continue;
        }
        log::info!("strip {device_name}: building {label}");
        if let Err(e) = build(app, &device_name, &label) {
            log::error!("strip {device_name}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor_widgets::StripInstance;

    fn widgets_with(monitor: &str, widget_id: &str) -> MonitorWidgets {
        MonitorWidgets(HashMap::from([(
            monitor.to_string(),
            vec![StripInstance { instance_id: format!("{widget_id}#1"), widget_id: widget_id.to_string() }],
        )]))
    }

    #[test]
    fn label_for_sanitizes_and_prefixes() {
        let label = label_for(r"\\.\DISPLAY2");
        assert!(label.starts_with(LABEL_PREFIX));
        assert!(label.contains("DISPLAY2"));
        assert!(!label.chars().any(|c| c == '\\' || c == '.'));
    }

    #[test]
    fn label_for_never_produces_bare_primary_label() {
        assert_ne!(label_for(""), PRIMARY_LABEL);
        assert_ne!(label_for(r"\\.\DISPLAY1"), PRIMARY_LABEL);
    }

    #[test]
    fn primary_always_gets_bare_label_even_with_no_instances() {
        let widgets = MonitorWidgets::default();
        let live = [(r"\\.\DISPLAY1".to_string(), true)];
        assert_eq!(wanted_strip_labels(&widgets, &live), vec![PRIMARY_LABEL.to_string()]);
    }

    #[test]
    fn secondary_with_instances_gets_a_label() {
        let widgets = widgets_with(r"\\.\DISPLAY2", "cpu");
        let live = [(r"\\.\DISPLAY1".to_string(), true), (r"\\.\DISPLAY2".to_string(), false)];
        let labels = wanted_strip_labels(&widgets, &live);
        assert_eq!(labels, vec![PRIMARY_LABEL.to_string(), label_for(r"\\.\DISPLAY2")]);
    }

    #[test]
    fn secondary_without_instances_gets_no_label() {
        let widgets = MonitorWidgets::default();
        let live = [(r"\\.\DISPLAY1".to_string(), true), (r"\\.\DISPLAY2".to_string(), false)];
        assert_eq!(wanted_strip_labels(&widgets, &live), vec![PRIMARY_LABEL.to_string()]);
    }

    #[test]
    fn resolve_label_for_monitor_empty_key_is_primary() {
        assert_eq!(resolve_label_for_monitor("", Some(r"\\.\DISPLAY1")), PRIMARY_LABEL);
    }

    #[test]
    fn resolve_label_for_monitor_matching_live_primary_is_bare_label() {
        assert_eq!(resolve_label_for_monitor(r"\\.\DISPLAY2", Some(r"\\.\DISPLAY2")), PRIMARY_LABEL);
    }

    #[test]
    fn resolve_label_for_monitor_secondary_device_gets_label_for() {
        let got = resolve_label_for_monitor(r"\\.\DISPLAY2", Some(r"\\.\DISPLAY1"));
        assert_eq!(got, label_for(r"\\.\DISPLAY2"));
    }

    #[test]
    fn unplugged_monitor_with_instances_produces_no_label() {
        // "DISPLAY3" has saved instances but isn't in the live monitor list.
        let widgets = widgets_with(r"\\.\DISPLAY3", "cpu");
        let live = [(r"\\.\DISPLAY1".to_string(), true)];
        assert_eq!(wanted_strip_labels(&widgets, &live), vec![PRIMARY_LABEL.to_string()]);
    }

    #[test]
    fn monitor_key_for_label_empty_only_resolves_to_primary() {
        let keys = vec!["".to_string()];
        assert_eq!(monitor_key_for_label(&keys, Some(r"\\.\DISPLAY1"), PRIMARY_LABEL), "");
    }

    #[test]
    fn monitor_key_for_label_secondary_resolves_to_its_own_key() {
        let keys = vec!["".to_string(), r"\\.\DISPLAY2".to_string()];
        let label = label_for(r"\\.\DISPLAY2");
        assert_eq!(monitor_key_for_label(&keys, Some(r"\\.\DISPLAY1"), &label), r"\\.\DISPLAY2");
    }

    #[test]
    fn monitor_key_for_label_prefers_non_empty_over_empty_when_both_resolve_to_primary() {
        // "" and the live primary's own device name both resolve to PRIMARY_LABEL.
        let keys = vec!["".to_string(), r"\\.\DISPLAY1".to_string()];
        assert_eq!(monitor_key_for_label(&keys, Some(r"\\.\DISPLAY1"), PRIMARY_LABEL), r"\\.\DISPLAY1");
    }

    #[test]
    fn monitor_key_for_label_no_match_returns_empty() {
        let keys = vec![r"\\.\DISPLAY2".to_string()];
        assert_eq!(monitor_key_for_label(&keys, Some(r"\\.\DISPLAY1"), "strip-DISPLAY3"), "");
    }

    fn pending(label: &str) -> Pending {
        (r"\\.\DISPLAY2".to_string(), label.to_string())
    }

    #[test]
    fn upsert_pending_adds_a_label_not_yet_queued() {
        let mut queue = vec![];
        assert!(upsert_pending(&mut queue, pending("strip-DISPLAY2")));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn upsert_pending_reports_a_label_already_queued_as_not_added() {
        // The 250ms poller re-queues every tick until the window exists; without this
        // the queue would grow by one entry per tick.
        let mut queue = vec![pending("strip-DISPLAY2")];
        assert!(!upsert_pending(&mut queue, pending("strip-DISPLAY2")));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn prune_pending_drops_a_label_no_longer_wanted() {
        let mut queue = vec![pending("strip-DISPLAY2"), pending("strip-DISPLAY3")];
        prune_pending(&mut queue, &["strip-DISPLAY3".to_string()]);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].1, "strip-DISPLAY3");
    }

    #[test]
    fn prune_pending_with_only_the_primary_wanted_empties_the_queue() {
        let mut queue = vec![pending("strip-DISPLAY2")];
        prune_pending(&mut queue, &[PRIMARY_LABEL.to_string()]);
        assert!(queue.is_empty());
    }
}
