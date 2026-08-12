use crate::monitor_widgets::MonitorWidgets;
use crate::settings::SettingsState;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL_PREFIX: &str = "strip-";
/// The primary monitor's window keeps this pre-existing static label so the
/// single-monitor case has zero behaviour change.
pub const PRIMARY_LABEL: &str = "strip";

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
    log::info!("strip reconcile: {} wanted", wanted.len());

    for (label, win) in app.webview_windows() {
        if label != PRIMARY_LABEL && label.starts_with(LABEL_PREFIX) && !wanted.contains(&label) {
            let _ = win.close();
            if let Ok(mut map) = app.state::<StripState>().0.lock() {
                map.remove(&label);
            }
        }
    }

    for label in &wanted {
        if label == PRIMARY_LABEL || app.get_webview_window(label).is_some() {
            continue;
        }
        let Some(device_name) =
            live.iter().find(|(name, _)| &label_for(name) == label).map(|(name, _)| name.clone())
        else {
            continue;
        };
        log::info!("strip {device_name}: building {label}");
        // build() must run on the main thread; reconcile() is also called from
        // save_settings (an IPC handler, off it) - same run_on_main_thread hop
        // overlay::reconcile uses for the identical reason.
        let (app2, device2, label2) = (app.clone(), device_name.clone(), label.clone());
        if let Err(e) = app.run_on_main_thread(move || {
            if let Err(e) = build(&app2, &device2, &label2) {
                log::error!("strip {device2}: {e}");
            }
        }) {
            log::error!("strip {device_name}: failed to dispatch build to main thread: {e}");
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
    fn unplugged_monitor_with_instances_produces_no_label() {
        // "DISPLAY3" has saved instances but isn't in the live monitor list.
        let widgets = widgets_with(r"\\.\DISPLAY3", "cpu");
        let live = [(r"\\.\DISPLAY1".to_string(), true)];
        assert_eq!(wanted_strip_labels(&widgets, &live), vec![PRIMARY_LABEL.to_string()]);
    }
}
