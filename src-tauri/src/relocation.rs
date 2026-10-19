//! Pure fullscreen-relocation layout math. When a monitor is covered by a fullscreen
//! app, its strip widgets move onto another live monitor's strip, appended after that
//! strip's own widgets; they move back once the fullscreen app goes away. Nothing is
//! written to settings - every caller recomputes this from the raw `monitor_widgets`
//! lane and the live covered-monitor snapshot whenever either changes (see
//! `effective_from_app` and autohide's poller).

use crate::monitor_widgets::{InstanceId, MonitorWidgets};
use crate::settings::OverlaySpec;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Instance ids whose overlay was relocated as of the last `effective_overlays_from_app`
/// call (i.e. the last reconcile tick). `save_overlay_geometry` consults this snapshot
/// instead of recomputing "is this overlay covered right now" from live state at save
/// time: a drag/resize that ends exactly as the covering fullscreen app closes would
/// otherwise see an already-empty covered set and persist the relocated target-monitor
/// position as the overlay's new home.
static RELOCATED_OVERLAYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether `id`'s overlay was relocated as of the last reconcile tick.
pub(crate) fn is_overlay_relocated(id: &str) -> bool {
    RELOCATED_OVERLAYS.lock().map(|g| g.iter().any(|s| s == id)).unwrap_or(false)
}

fn store_relocated_overlays(ids: Vec<InstanceId>) {
    if let Ok(mut guard) = RELOCATED_OVERLAYS.lock() {
        *guard = ids;
    }
}

/// Pure: the subset of `overlays` whose stored monitor is currently relocated, per the
/// same `relocation_target_for_overlay` decision `effective_overlays` uses to move them.
pub(crate) fn relocated_overlay_ids(
    overlays: &[(InstanceId, String, OverlaySpec)],
    covered: &[String],
    connected: &[String],
    primary: &str,
    enabled: bool,
) -> Vec<InstanceId> {
    overlays
        .iter()
        .filter(|(_, _, spec)| {
            relocation_target_for_overlay(&spec.monitor, covered, connected, primary, enabled).is_some()
        })
        .map(|(id, _, _)| id.clone())
        .collect()
}

/// Picks which connected monitor a covered monitor's widgets land on: the primary if it
/// is connected and itself uncovered, else the first uncovered connected monitor in
/// sorted order. `None` when no uncovered connected monitor exists (every monitor is
/// covered) - the caller leaves that lane alone in that case.
fn pick_target(covered: &[String], connected: &[String], primary: &str) -> Option<String> {
    if !primary.is_empty()
        && connected.iter().any(|d| d == primary)
        && !covered.iter().any(|d| d == primary)
    {
        return Some(primary.to_string());
    }
    let mut sorted: Vec<&String> = connected.iter().collect();
    sorted.sort();
    sorted.into_iter().find(|d| !covered.iter().any(|c| c == *d)).cloned()
}

/// The runtime strip layout after fullscreen relocation. Disabled or nothing covered
/// returns `widgets` unchanged, byte for byte. Otherwise the `""` primary sentinel folds
/// into `primary`'s own key first (see `monitor_swap::fold_primary_sentinel`), then every
/// covered connected monitor with a non-empty lane moves its instances onto
/// `pick_target`'s pick - appended after that target's own instances, same order, same
/// ids - leaving the covered lane present-and-empty. A covered monitor with an already
/// empty lane is left untouched (nothing to move, nothing to mark). A covered monitor
/// with no target (every monitor covered) is also left untouched, which is how "every
/// monitor covered, nothing moves" falls out with no special case.
pub(crate) fn effective_lanes(
    widgets: &MonitorWidgets,
    covered: &[String],
    connected: &[String],
    primary: &str,
    enabled: bool,
) -> MonitorWidgets {
    if !enabled || covered.is_empty() {
        return widgets.clone();
    }

    let mut result = widgets.clone();
    crate::monitor_swap::fold_primary_sentinel(&mut result, primary);

    let mut covered_sorted: Vec<&String> = covered.iter().filter(|d| connected.contains(d)).collect();
    covered_sorted.sort();

    for device in covered_sorted {
        let instances = result.0.get(device.as_str()).cloned().unwrap_or_default();
        if instances.is_empty() {
            continue;
        }
        let Some(target) = pick_target(covered, connected, primary) else { continue };
        let mut target_lane = result.0.get(&target).cloned().unwrap_or_default();
        target_lane.extend(instances);
        result.0.insert(target, target_lane);
        result.0.insert(device.clone(), Vec::new());
    }

    result
}

/// True when `monitor`'s lane in `effective` exists but holds no instances - either the
/// user emptied it in the lanes UI, or relocation just moved its widgets elsewhere. An
/// absent key is a different case (no explicit lane at all) and is not this one.
pub(crate) fn lane_is_present_and_empty(effective: &MonitorWidgets, monitor: &str) -> bool {
    effective.0.get(monitor).is_some_and(|v| v.is_empty())
}

/// Builds the effective layout from live app state: settings' `monitor_widgets` lane and
/// `move_on_fullscreen` flag, autohide's latest covered-monitor snapshot, and the live
/// connected-monitor set. A poisoned settings lock fails closed to an empty map; an
/// unreadable monitor list fails closed to no connected monitors, which `effective_lanes`
/// already treats as nothing to relocate.
/// Live connected-monitor device names, shared by every `*_from_app` helper below so
/// they all see the same set within one call.
pub(crate) fn connected_monitor_devices(app: &AppHandle) -> Vec<String> {
    app.available_monitors()
        .map(|monitors| monitors.into_iter().filter_map(|m| m.name().cloned()).collect())
        .unwrap_or_default()
}

/// Live OS-primary device name, or `""` if it can't be read - same fail-closed shape
/// `pick_target` already treats as "no primary preference".
pub(crate) fn primary_monitor_device(app: &AppHandle) -> String {
    app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned()).unwrap_or_default()
}

/// Each connected monitor's real work area in CSS px, keyed by device name - the unit
/// `clamp_to_monitor` expects. `monitor_swap::swap_monitors` reuses this at click time
/// instead of its own inline per-monitor computation. A monitor with no name is
/// skipped; nothing can target it.
pub(crate) fn connected_work_areas(app: &AppHandle) -> HashMap<String, (f64, f64)> {
    connected_devices_and_work_areas(app).1
}

/// `connected_monitor_devices` and `connected_work_areas` together, from one
/// `available_monitors()` read - `effective_overlays_from_app` needs both every poll
/// tick, and the two standalone helpers above would otherwise cost two Win32
/// enumeration passes per tick instead of one.
fn connected_devices_and_work_areas(app: &AppHandle) -> (Vec<String>, HashMap<String, (f64, f64)>) {
    let Ok(monitors) = app.available_monitors() else { return (Vec::new(), HashMap::new()) };
    let mut devices = Vec::new();
    let mut work_areas = HashMap::new();
    for m in &monitors {
        let Some(name) = m.name().cloned() else { continue };
        let scale = m.scale_factor();
        let wa = m.work_area();
        work_areas.insert(name.clone(), (wa.size.width as f64 / scale, wa.size.height as f64 / scale));
        devices.push(name);
    }
    (devices, work_areas)
}

pub(crate) fn effective_from_app(app: &AppHandle) -> MonitorWidgets {
    let (widgets, enabled) = match app.state::<crate::settings::SettingsState>().0.lock() {
        Ok(s) => (s.monitor_widgets.clone(), s.move_on_fullscreen),
        Err(e) => {
            log::error!("effective_from_app: settings lock poisoned: {e}");
            return MonitorWidgets::default();
        }
    };
    let connected = connected_monitor_devices(app);
    let primary = primary_monitor_device(app);
    let covered = crate::autohide::covered_devices();
    effective_lanes(&widgets, &covered, &connected, &primary, enabled)
}

/// `Some(target)` when `stored_monitor` (`""` reads as primary, same convention as
/// `OverlaySpec::monitor`) is covered right now and `pick_target` has somewhere to
/// send it; `None` covers every other case - not covered, disabled, nothing covered,
/// or every monitor covered - in one return value, so a caller never has to re-derive
/// "did this actually move" from a before/after spec diff.
pub(crate) fn relocation_target_for_overlay(
    stored_monitor: &str,
    covered: &[String],
    connected: &[String],
    primary: &str,
    enabled: bool,
) -> Option<String> {
    if !enabled || covered.is_empty() {
        return None;
    }
    let resolved = if stored_monitor.is_empty() { primary } else { stored_monitor };
    if !covered.iter().any(|c| c == resolved) {
        return None;
    }
    pick_target(covered, connected, primary)
}

/// The overlay-placement analogue of `effective_lanes`. Unlike strip lanes, overlays
/// never merge into each other's list, so each one transforms independently: a
/// covered overlay is re-pointed to `relocation_target_for_overlay`'s pick (the same
/// monitor the strip on that lane lands on) and clamped into that monitor's real work
/// area via `monitor_swap::clamp_to_monitor`, using the overlay's own stored size.
/// Everything else - uncovered, disabled, both-covered, no entry in `work_areas` -
/// passes through unchanged, byte for byte.
pub(crate) fn effective_overlays(
    overlays: &[(InstanceId, String, OverlaySpec)],
    covered: &[String],
    connected: &[String],
    primary: &str,
    enabled: bool,
    work_areas: &HashMap<String, (f64, f64)>,
) -> Vec<(InstanceId, String, OverlaySpec)> {
    overlays
        .iter()
        .map(|(id, widget_id, spec)| {
            match relocation_target_for_overlay(&spec.monitor, covered, connected, primary, enabled) {
                None => (id.clone(), widget_id.clone(), spec.clone()),
                Some(target) => {
                    let mut moved = spec.clone();
                    moved.monitor = target.clone();
                    if let Some(&(w, h)) = work_areas.get(&target) {
                        moved = crate::monitor_swap::clamp_to_monitor(&moved, w, h);
                    }
                    (id.clone(), widget_id.clone(), moved)
                }
            }
        })
        .collect()
}

/// Builds the effective overlay placement list from live app state, mirroring
/// `effective_from_app` for strip lanes. Nothing here is written back to settings -
/// the saved spec still reflects the overlay's real home, which is why the next call
/// after the covered set clears restores it verbatim with no extra "move it back"
/// code path.
pub(crate) fn effective_overlays_from_app(app: &AppHandle) -> Vec<(InstanceId, String, OverlaySpec)> {
    let (overlays, enabled) = match app.state::<crate::settings::SettingsState>().0.lock() {
        Ok(s) => (s.overlays(), s.move_on_fullscreen),
        Err(e) => {
            log::error!("effective_overlays_from_app: settings lock poisoned: {e}");
            return Vec::new();
        }
    };
    let (connected, work_areas) = connected_devices_and_work_areas(app);
    let primary = primary_monitor_device(app);
    let covered = crate::autohide::covered_devices();
    store_relocated_overlays(relocated_overlay_ids(&overlays, &covered, &connected, &primary, enabled));
    effective_overlays(&overlays, &covered, &connected, &primary, enabled, &work_areas)
}

/// The frontend's handle onto the effective layout: same shape as
/// `get_settings().monitor_widgets`, so a caller can swap one for the other when
/// deciding which tiles render on which strip.
#[tauri::command]
pub fn get_effective_monitor_widgets(app: AppHandle) -> MonitorWidgets {
    effective_from_app(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor_widgets::StripInstance;
    use std::collections::HashMap;

    fn instance(id: &str, widget_id: &str) -> StripInstance {
        StripInstance { instance_id: id.to_string(), widget_id: widget_id.to_string() }
    }

    fn lane_ids(w: &MonitorWidgets, monitor: &str) -> Vec<String> {
        w.instances_for(monitor).iter().map(|si| si.instance_id.clone()).collect()
    }

    fn two_monitor_widgets() -> MonitorWidgets {
        MonitorWidgets(HashMap::from([
            ("A".to_string(), vec![instance("cpu#1", "cpu")]),
            ("B".to_string(), vec![instance("ram#1", "ram")]),
        ]))
    }

    #[test]
    fn primary_covered_moves_onto_secondary() {
        let widgets = two_monitor_widgets();
        let effective = effective_lanes(
            &widgets,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert!(lane_ids(&effective, "A").is_empty());
        assert_eq!(lane_ids(&effective, "B"), ["ram#1", "cpu#1"]);
    }

    #[test]
    fn secondary_covered_moves_onto_primary() {
        let widgets = two_monitor_widgets();
        let effective = effective_lanes(
            &widgets,
            &["B".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert!(lane_ids(&effective, "B").is_empty());
        assert_eq!(lane_ids(&effective, "A"), ["cpu#1", "ram#1"]);
    }

    #[test]
    fn both_monitors_covered_is_unchanged() {
        let widgets = two_monitor_widgets();
        let covered = vec!["A".to_string(), "B".to_string()];
        let connected = covered.clone();
        let effective = effective_lanes(&widgets, &covered, &connected, "A", true);
        assert_eq!(effective, widgets);
    }

    #[test]
    fn disabled_is_unchanged() {
        let widgets = two_monitor_widgets();
        let effective = effective_lanes(
            &widgets,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            false,
        );
        assert_eq!(effective, widgets);
    }

    #[test]
    fn nothing_covered_is_unchanged() {
        let widgets = two_monitor_widgets();
        let effective = effective_lanes(&widgets, &[], &["A".to_string(), "B".to_string()], "A", true);
        assert_eq!(effective, widgets);
    }

    #[test]
    fn primary_sentinel_lane_moves_like_a_real_device_key() {
        // Pre-lanes-UI install: the primary's widgets still sit under "".
        let widgets = MonitorWidgets(HashMap::from([
            ("".to_string(), vec![instance("cpu#1", "cpu")]),
            ("B".to_string(), vec![instance("ram#1", "ram")]),
        ]));
        let effective = effective_lanes(
            &widgets,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert!(lane_ids(&effective, "").is_empty());
        assert!(lane_ids(&effective, "A").is_empty());
        assert_eq!(lane_ids(&effective, "B"), ["ram#1", "cpu#1"]);
    }

    #[test]
    fn a_covered_monitor_with_an_empty_lane_changes_nothing() {
        let widgets = MonitorWidgets(HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec![instance("ram#1", "ram")]),
        ]));
        let effective = effective_lanes(
            &widgets,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert_eq!(effective, widgets);
    }

    #[test]
    fn target_with_an_absent_key_gets_a_lane_holding_only_the_moved_instances() {
        let widgets = MonitorWidgets(HashMap::from([("A".to_string(), vec![instance("cpu#1", "cpu")])]));
        let effective = effective_lanes(
            &widgets,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert_eq!(lane_ids(&effective, "B"), ["cpu#1"]);
    }

    #[test]
    fn lane_is_present_and_empty_true_only_for_an_explicit_empty_vec() {
        let w = MonitorWidgets(HashMap::from([("A".to_string(), vec![])]));
        assert!(lane_is_present_and_empty(&w, "A"));
        assert!(!lane_is_present_and_empty(&w, "B")); // absent key, not this case
    }

    #[test]
    fn lane_is_present_and_empty_false_when_it_has_instances() {
        let w = MonitorWidgets(HashMap::from([("A".to_string(), vec![instance("cpu#1", "cpu")])]));
        assert!(!lane_is_present_and_empty(&w, "A"));
    }

    fn overlay_entry(id: &str, monitor: &str, x: f64, y: f64) -> (InstanceId, String, OverlaySpec) {
        (id.to_string(), "cpu".to_string(), OverlaySpec { monitor: monitor.to_string(), x, y, ..Default::default() })
    }

    fn monitor_of(effective: &[(InstanceId, String, OverlaySpec)], id: &str) -> String {
        effective.iter().find(|(i, ..)| i == id).unwrap().2.monitor.clone()
    }

    #[test]
    fn relocation_target_for_overlay_is_none_when_not_covered() {
        assert_eq!(
            relocation_target_for_overlay("A", &["B".to_string()], &["A".to_string(), "B".to_string()], "A", true),
            None
        );
    }

    #[test]
    fn relocation_target_for_overlay_is_none_when_disabled() {
        assert_eq!(
            relocation_target_for_overlay("A", &["A".to_string()], &["A".to_string(), "B".to_string()], "A", false),
            None
        );
    }

    #[test]
    fn relocation_target_for_overlay_resolves_the_primary_sentinel() {
        assert_eq!(
            relocation_target_for_overlay("", &["A".to_string()], &["A".to_string(), "B".to_string()], "A", true),
            Some("B".to_string())
        );
    }

    #[test]
    fn relocation_target_for_overlay_is_none_when_every_monitor_covered() {
        let covered = vec!["A".to_string(), "B".to_string()];
        assert_eq!(relocation_target_for_overlay("A", &covered, &covered, "A", true), None);
    }

    #[test]
    fn effective_overlays_moves_and_clamps_a_covered_overlay() {
        let overlays = vec![overlay_entry("cpu#1", "A", 2000.0, 100.0)];
        let work_areas = HashMap::from([("B".to_string(), (1920.0, 1080.0))]);
        let effective = effective_overlays(
            &overlays,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
            &work_areas,
        );
        assert_eq!(monitor_of(&effective, "cpu#1"), "B");
        let spec = &effective[0].2;
        assert!(spec.x + crate::overlay::FALLBACK_W <= 1920.0, "x={} not clamped into 1920", spec.x);
    }

    #[test]
    fn effective_overlays_leaves_an_uncovered_overlay_unchanged() {
        let overlays = vec![overlay_entry("cpu#1", "B", 10.0, 10.0)];
        let work_areas = HashMap::new();
        let effective = effective_overlays(
            &overlays,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
            &work_areas,
        );
        assert_eq!(effective, overlays);
    }

    #[test]
    fn effective_overlays_leaves_overlays_unchanged_when_every_monitor_covered() {
        let overlays = vec![overlay_entry("cpu#1", "A", 10.0, 10.0)];
        let covered = vec!["A".to_string(), "B".to_string()];
        let effective = effective_overlays(&overlays, &covered, &covered, "A", true, &HashMap::new());
        assert_eq!(effective, overlays);
    }

    #[test]
    fn effective_overlays_leaves_overlays_unchanged_when_disabled() {
        let overlays = vec![overlay_entry("cpu#1", "A", 10.0, 10.0)];
        let work_areas = HashMap::from([("B".to_string(), (1920.0, 1080.0))]);
        let effective = effective_overlays(
            &overlays,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            false,
            &work_areas,
        );
        assert_eq!(effective, overlays);
    }

    #[test]
    fn relocated_overlay_ids_includes_a_relocated_overlay_and_excludes_an_unrelocated_one() {
        let overlays = vec![
            overlay_entry("cpu#1", "A", 10.0, 10.0),
            overlay_entry("ram#1", "B", 10.0, 10.0),
        ];
        let ids = relocated_overlay_ids(
            &overlays,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
        );
        assert!(ids.contains(&"cpu#1".to_string()));
        assert!(!ids.contains(&"ram#1".to_string()));
    }

    // Reproduces the overlay-rehoming bug directly: `is_overlay_relocated` must answer
    // from the last-stored reconcile snapshot alone, never from a live covered-set
    // recompute - a drag/resize ending exactly as the covering fullscreen app closes
    // would otherwise see an already-empty covered set and let `save_overlay_geometry`
    // persist the relocated target-monitor position as the overlay's new home. This
    // test takes no `covered` argument at all, which is the fix: there is no live state
    // left to race.
    #[test]
    fn is_overlay_relocated_reflects_the_last_stored_snapshot_not_any_live_state() {
        store_relocated_overlays(vec!["cpu#1".to_string()]);
        assert!(is_overlay_relocated("cpu#1"));
        assert!(!is_overlay_relocated("ram#1"));
    }

    #[test]
    fn effective_overlays_moves_the_primary_sentinel_overlay_when_primary_is_covered() {
        let overlays = vec![overlay_entry("cpu#1", "", 10.0, 10.0)];
        let work_areas = HashMap::from([("B".to_string(), (1920.0, 1080.0))]);
        let effective = effective_overlays(
            &overlays,
            &["A".to_string()],
            &["A".to_string(), "B".to_string()],
            "A",
            true,
            &work_areas,
        );
        assert_eq!(monitor_of(&effective, "cpu#1"), "B");
    }
}
