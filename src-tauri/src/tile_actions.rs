use crate::settings::{Settings, StripInstance};
use std::collections::HashMap;

// "Edit this widget" opens the settings strip editor, which is still keyed by
// widget kind (not per-placement), so the menu needs the kind back from the
// instance id it was built for.
pub(crate) fn widget_kind_for(s: &Settings, instance_id: &str) -> String {
    s.monitor_widgets
        .all()
        .find(|si| si.instance_id == instance_id)
        .map(|si| si.widget_id.clone())
        .unwrap_or_else(|| instance_id.to_string())
}

// Stays in monitor_widgets so re-enabling remembers its lane. The legacy widget-id
// keyed lists (still read by overlay::reconcile, poller.rs, bridge_pomodoro.rs) are
// mirrored only once no other visible instance of that kind remains, so hiding one
// placement can never hide a sibling. Returns the instance's monitor, if found.
pub(crate) fn apply_hide(s: &mut Settings, instance_id: &str) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string);
    let widget_id = s.monitor_widgets.all().find(|si| si.instance_id == instance_id).map(|si| si.widget_id.clone());

    if !s.hidden_widgets.iter().any(|w| w == instance_id) {
        s.hidden_widgets.push(instance_id.to_string());
    }
    if let Some(wid) = &widget_id {
        let sibling_visible = s.monitor_widgets.all().any(|si| {
            &si.widget_id == wid && si.instance_id != instance_id && !s.hidden_widgets.iter().any(|h| h == &si.instance_id)
        });
        if !sibling_visible {
            s.enabled_widgets.retain(|w| w != wid);
            if !s.hidden_widgets.iter().any(|w| w == wid) {
                s.hidden_widgets.push(wid.clone());
            }
        }
    }
    monitor
}

// Unlike hide, drops the instance entirely: a divider's uuid id is single-use, so
// there's nothing meaningful to re-offer later, and monitor_widgets shouldn't grow
// dead entries.
pub(crate) fn apply_remove_divider(s: &mut Settings, instance_id: &str) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string);
    if let Some(m) = &monitor {
        if let Some(instances) = s.monitor_widgets.0.get_mut(m) {
            instances.retain(|si| si.instance_id != instance_id);
        }
    }
    s.enabled_widgets.retain(|w| w != instance_id);
    monitor
}

// Reorders within the instance's own monitor lane. The swap is mirrored onto the
// legacy enabled_widgets, which is still what main.ts renders strip order from, so
// move-left/right stays visible until Phase 3 threads instance ids to the frontend.
// Mirroring only ever swaps the two kinds involved, so it cannot reorder a sibling lane.
pub(crate) fn apply_move(s: &mut Settings, instance_id: &str, dir: i32) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string)?;
    let mut swapped_kinds: Option<(String, String)> = None;
    if let Some(instances) = s.monitor_widgets.0.get_mut(&monitor) {
        if let Some(i) = instances.iter().position(|si| si.instance_id == instance_id) {
            let j = i as i32 + dir;
            if j >= 0 && (j as usize) < instances.len() {
                let j = j as usize;
                swapped_kinds =
                    Some((instances[i].widget_id.clone(), instances[j].widget_id.clone()));
                instances.swap(i, j);
            }
        }
    }
    if let Some((a, b)) = swapped_kinds {
        let ia = s.enabled_widgets.iter().position(|w| w == &a);
        let ib = s.enabled_widgets.iter().position(|w| w == &b);
        if let (Some(ia), Some(ib)) = (ia, ib) {
            s.enabled_widgets.swap(ia, ib);
        }
    }
    Some(monitor)
}

/// Replaces each named monitor's lane with `widget_ids`, in that order. Only the keys
/// given are touched, plus the `""` lane the Decision 2 block below retires, so an
/// unplugged monitor's saved lane is never mutated on absence. An instance already in
/// that lane is reused rather than re-minted, keeping its hides and overlay geometry.
pub(crate) fn apply_lanes(s: &mut Settings, lanes: &HashMap<String, Vec<String>>) {
    for (monitor, widget_ids) in lanes {
        let mut previous = s.monitor_widgets.0.remove(monitor).unwrap_or_default();
        s.monitor_widgets.0.insert(monitor.clone(), Vec::with_capacity(widget_ids.len()));
        for widget_id in widget_ids {
            // Pushed as it is built, not collected then inserted: next_instance_id scans
            // the live map, so two copies of one kind in one lane would otherwise both
            // mint the same "#n".
            let instance = match previous.iter().position(|si| &si.widget_id == widget_id) {
                Some(i) => previous.remove(i),
                None => StripInstance {
                    instance_id: s.monitor_widgets.next_instance_id(widget_id),
                    widget_id: widget_id.clone(),
                },
            };
            s.monitor_widgets.0.entry(monitor.clone()).or_default().push(instance);
        }
    }
    // Decision 2: a placement locks to a concrete device name, so the migration
    // default's lane goes away the first time the lanes UI writes a real one.
    if lanes.keys().any(|k| !k.is_empty()) {
        s.monitor_widgets.0.remove("");
    }
    sync_enabled_widgets(s);
}

/// `enabled_widgets` is still the flat list every widget-kind-keyed reader works from
/// (overlay::reconcile, poller.rs, bridge_pomodoro.rs), so it is rebuilt as the union of
/// every lane. Existing relative order is kept; a kind that left every lane is pushed to
/// hidden_widgets, or main.ts's newWidgetIds would re-adopt it on the next launch.
fn sync_enabled_widgets(s: &mut Settings) {
    let mut union: Vec<String> = Vec::new();
    for si in s.monitor_widgets.all() {
        if !union.contains(&si.widget_id) {
            union.push(si.widget_id.clone());
        }
    }
    for dropped in s.enabled_widgets.clone().into_iter().filter(|w| !union.contains(w)) {
        if !s.hidden_widgets.contains(&dropped) {
            s.hidden_widgets.push(dropped);
        }
    }
    let mut next: Vec<String> = s.enabled_widgets.drain(..).filter(|w| union.contains(w)).collect();
    for widget_id in union {
        if !next.contains(&widget_id) {
            next.push(widget_id);
        }
    }
    s.enabled_widgets = next;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::MonitorWidgets;

    fn instance(id: &str, widget_id: &str) -> StripInstance {
        StripInstance { instance_id: id.to_string(), widget_id: widget_id.to_string() }
    }

    fn two_monitor_settings() -> Settings {
        Settings {
            monitor_widgets: MonitorWidgets(HashMap::from([
                ("".to_string(), vec![instance("cpu#1", "cpu"), instance("ram#1", "ram")]),
                (r"\\.\DISPLAY2".to_string(), vec![instance("cpu#2", "cpu"), instance("gpu#1", "gpu")]),
            ])),
            ..Settings::default()
        }
    }

    #[test]
    fn hide_does_not_hide_sibling_instance_on_another_monitor() {
        let mut s = two_monitor_settings();

        apply_hide(&mut s, "cpu#1");

        assert!(s.hidden_widgets.contains(&"cpu#1".to_string()));
        assert!(!s.hidden_widgets.contains(&"cpu#2".to_string()));
        assert!(s.monitor_widgets.all().any(|si| si.instance_id == "cpu#2"));
    }

    #[test]
    fn hide_only_instance_of_a_kind_also_mirrors_into_legacy_enabled_widgets() {
        let mut s = Settings::default(); // one instance per widget id, all on ""

        apply_hide(&mut s, "cpu#1");

        assert!(!s.enabled_widgets.contains(&"cpu".to_string()));
        assert!(s.hidden_widgets.contains(&"cpu".to_string()));
        assert!(s.hidden_widgets.contains(&"cpu#1".to_string()));
    }

    #[test]
    fn hide_with_a_visible_sibling_leaves_legacy_enabled_widgets_alone() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_hide(&mut s, "cpu#1");

        assert!(s.enabled_widgets.contains(&"cpu".to_string()));
    }

    fn lanes(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(m, ids)| (m.to_string(), ids.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn lane_instance_ids(s: &Settings, monitor: &str) -> Vec<String> {
        s.monitor_widgets.instances_for(monitor).iter().map(|si| si.instance_id.clone()).collect()
    }

    #[test]
    fn apply_lanes_reuses_an_existing_instance_id_when_a_lane_is_only_reordered() {
        let mut s = two_monitor_settings();

        apply_lanes(&mut s, &lanes(&[(r"\\.\DISPLAY2", &["gpu", "cpu"])]));

        assert_eq!(lane_instance_ids(&s, r"\\.\DISPLAY2"), ["gpu#1", "cpu#2"]);
    }

    #[test]
    fn apply_lanes_mints_a_distinct_id_for_each_copy_of_one_kind_in_the_same_lane() {
        let mut s = Settings { monitor_widgets: MonitorWidgets::default(), ..Settings::default() };

        apply_lanes(&mut s, &lanes(&[(r"\\.\DISPLAY1", &["cpu", "cpu"])]));

        let ids = lane_instance_ids(&s, r"\\.\DISPLAY1");
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn apply_lanes_drops_the_migration_default_lane_once_a_real_device_is_written() {
        let mut s = two_monitor_settings();

        apply_lanes(&mut s, &lanes(&[(r"\\.\DISPLAY1", &["cpu", "ram"])]));

        assert!(s.monitor_widgets.instances_for("").is_empty());
        assert_eq!(lane_instance_ids(&s, r"\\.\DISPLAY1").len(), 2);
    }

    #[test]
    fn apply_lanes_never_touches_a_lane_it_was_not_given() {
        let mut s = two_monitor_settings();

        apply_lanes(&mut s, &lanes(&[("", &["cpu"])]));

        assert_eq!(lane_instance_ids(&s, r"\\.\DISPLAY2"), ["cpu#2", "gpu#1"]);
    }

    #[test]
    fn apply_lanes_rebuilds_enabled_widgets_as_the_union_of_every_lane() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_lanes(&mut s, &lanes(&[(r"\\.\DISPLAY1", &["ram"]), (r"\\.\DISPLAY2", &["gpu"])]));

        assert_eq!(s.enabled_widgets, ["ram", "gpu"]);
    }

    #[test]
    fn apply_lanes_hides_a_kind_that_left_every_lane_so_it_is_not_re_adopted() {
        // main.ts's newWidgetIds re-adopts anything in neither enabled nor hidden.
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_lanes(&mut s, &lanes(&[("", &["cpu"]), (r"\\.\DISPLAY2", &["cpu"])]));

        assert!(s.hidden_widgets.contains(&"ram".to_string()));
        assert!(s.hidden_widgets.contains(&"gpu".to_string()));
    }

    #[test]
    fn apply_lanes_leaves_an_already_hidden_kind_hidden_and_unlisted_once() {
        // The lanes UI never sends a hidden widget, so an unrelated drag must not
        // resurrect one, nor stack a second hidden_widgets entry for it.
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string()];
        s.hidden_widgets = vec!["gpu".to_string()];

        apply_lanes(&mut s, &lanes(&[("", &["cpu", "ram"]), (r"\\.\DISPLAY2", &["cpu"])]));

        assert!(!s.enabled_widgets.contains(&"gpu".to_string()));
        assert_eq!(s.hidden_widgets.iter().filter(|h| *h == "gpu").count(), 1);
    }

    #[test]
    fn move_reorders_within_one_monitor_lane_and_never_leaks_into_another() {
        let mut s = two_monitor_settings();

        apply_move(&mut s, "ram#1", -1);

        let primary: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(primary, ["ram#1", "cpu#1"]);
        let secondary: Vec<&str> =
            s.monitor_widgets.instances_for(r"\\.\DISPLAY2").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(secondary, ["cpu#2", "gpu#1"]);
    }

    #[test]
    fn move_past_the_lane_boundary_is_a_no_op() {
        let mut s = two_monitor_settings();

        apply_move(&mut s, "cpu#1", -1);

        let primary: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(primary, ["cpu#1", "ram#1"]);
    }

    #[test]
    fn move_mirrors_the_swap_into_legacy_enabled_widgets() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_move(&mut s, "ram#1", -1);

        assert_eq!(s.enabled_widgets, vec!["ram", "cpu", "gpu"]);
    }

    #[test]
    fn move_mirror_leaves_kinds_outside_the_swap_in_place() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["gpu".to_string(), "cpu".to_string(), "ram".to_string()];

        apply_move(&mut s, "ram#1", -1);

        assert_eq!(s.enabled_widgets[0], "gpu");
    }

    #[test]
    fn move_past_the_lane_boundary_leaves_legacy_enabled_widgets_alone() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];
        let before = s.enabled_widgets.clone();

        apply_move(&mut s, "cpu#1", -1);

        assert_eq!(s.enabled_widgets, before);
    }

    #[test]
    fn remove_divider_drops_the_instance_and_the_legacy_entry() {
        let mut s = Settings {
            enabled_widgets: vec!["cpu".to_string(), "divider:abc".to_string(), "ram".to_string()],
            monitor_widgets: MonitorWidgets(HashMap::from([(
                "".to_string(),
                vec![instance("cpu#1", "cpu"), instance("divider:abc", "divider:abc"), instance("ram#1", "ram")],
            )])),
            ..Settings::default()
        };

        apply_remove_divider(&mut s, "divider:abc");

        assert!(!s.enabled_widgets.contains(&"divider:abc".to_string()));
        assert!(!s.monitor_widgets.all().any(|si| si.instance_id == "divider:abc"));
    }

    #[test]
    fn widget_kind_for_resolves_the_owning_widget_id() {
        let s = two_monitor_settings();
        assert_eq!(widget_kind_for(&s, "gpu#1"), "gpu");
    }
}
