//! Pure fullscreen-relocation layout math. When a monitor is covered by a fullscreen
//! app, its strip widgets move onto another live monitor's strip, appended after that
//! strip's own widgets; they move back once the fullscreen app goes away. Nothing is
//! written to settings - every caller recomputes this from the raw `monitor_widgets`
//! lane and the live covered-monitor snapshot whenever either changes (see
//! `effective_from_app` and autohide's poller).

use crate::monitor_widgets::MonitorWidgets;
use tauri::{AppHandle, Manager};

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
pub(crate) fn effective_from_app(app: &AppHandle) -> MonitorWidgets {
    let (widgets, enabled) = match app.state::<crate::settings::SettingsState>().0.lock() {
        Ok(s) => (s.monitor_widgets.clone(), s.move_on_fullscreen),
        Err(e) => {
            log::error!("effective_from_app: settings lock poisoned: {e}");
            return MonitorWidgets::default();
        }
    };
    let connected: Vec<String> = app
        .available_monitors()
        .map(|monitors| monitors.into_iter().filter_map(|m| m.name().cloned()).collect())
        .unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned()).unwrap_or_default();
    let covered = crate::autohide::covered_devices();
    effective_lanes(&widgets, &covered, &connected, &primary, enabled)
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
}
