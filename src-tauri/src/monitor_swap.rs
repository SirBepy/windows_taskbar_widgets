use crate::overlay;
use crate::settings::{MonitorWidgets, OverlaySpec, Placement, Settings, SettingsState};
use tauri::{AppHandle, Emitter, Manager};

/// Folds monitor_widgets' "" sentinel lane into `primary_device`'s own key, so a swap
/// always exchanges two concrete device-name lanes, never a generic alias that would
/// silently re-resolve back to whichever monitor is OS-primary after the swap. A real
/// device key, when already present, wins over "" - the same bias
/// `strip::monitor_key_for_label` already applies when both could serve one window -
/// so "" is treated as a stale leftover from before the lanes UI ever wrote a concrete
/// key for that monitor, never as a second lane to merge in.
pub(crate) fn fold_primary_sentinel(widgets: &mut MonitorWidgets, primary_device: &str) {
    if primary_device.is_empty() {
        return;
    }
    if let Some(empty_lane) = widgets.0.remove("") {
        widgets.0.entry(primary_device.to_string()).or_insert(empty_lane);
    }
}

/// Pure core of "Swap monitors": exchanges `device_a`'s and `device_b`'s whole
/// monitor_widgets lane, then re-points every overlay living on either monitor to the
/// other one. Any other monitor's lane, and any overlay elsewhere, is untouched.
/// Presence, not just content, is swapped: a monitor with no key at all (never
/// touched by the lanes UI) stays keyless after trading with one that has an explicit
/// empty lane (`apply_lanes`'s "user emptied this monitor" signal) - collapsing the
/// two would silently turn "untouched" into "deliberately emptied".
pub fn swap_lanes_and_overlays(
    settings: &mut Settings,
    device_a: &str,
    device_b: &str,
    primary_device: &str,
) {
    fold_primary_sentinel(&mut settings.monitor_widgets, primary_device);

    let a_present = settings.monitor_widgets.0.contains_key(device_a);
    let b_present = settings.monitor_widgets.0.contains_key(device_b);
    let a_lane = settings.monitor_widgets.0.remove(device_a).unwrap_or_default();
    let b_lane = settings.monitor_widgets.0.remove(device_b).unwrap_or_default();
    if b_present {
        settings.monitor_widgets.0.insert(device_a.to_string(), b_lane);
    }
    if a_present {
        settings.monitor_widgets.0.insert(device_b.to_string(), a_lane);
    }

    for placement in settings.widget_placement.values_mut() {
        let Placement::Overlay(spec) = placement else { continue };
        let resolved = if spec.monitor.is_empty() { primary_device } else { spec.monitor.as_str() };
        if resolved == device_a {
            spec.monitor = device_b.to_string();
        } else if resolved == device_b {
            spec.monitor = device_a.to_string();
        }
    }
}

/// Overlay's real footprint in CSS px: the resize grip's stored size once set, else the
/// same FALLBACK_W/H `overlay::geometry` falls back to for a spec that has never been
/// resized - the two must agree, or a freshly-placed overlay would clamp against a
/// size different from the one it's actually drawn at.
fn overlay_footprint(spec: &OverlaySpec) -> (f64, f64) {
    (spec.w.unwrap_or(overlay::FALLBACK_W), spec.h.unwrap_or(overlay::FALLBACK_H))
}

/// Pure: pulls `spec` fully inside a `work_w` x `work_h` CSS-px work area, using the
/// overlay's own real size. Unlike `overlay::geometry`'s drag clamp (which only keeps
/// MIN_VISIBLE px on screen, so a window already being dragged never jumps), a monitor
/// swap has no user grip to respect - the destination's DPI and aspect ratio can differ
/// enough (portrait 2160x3840 @1.5 vs landscape @1.0) that the old x/y lands partly or
/// fully off the new monitor, so this clamps the whole footprint on screen instead.
pub fn clamp_to_monitor(spec: &OverlaySpec, work_w: f64, work_h: f64) -> OverlaySpec {
    let (w, h) = overlay_footprint(spec);
    let max_x = (work_w - w).max(0.0);
    let max_y = (work_h - h).max(0.0);
    OverlaySpec { x: spec.x.clamp(0.0, max_x), y: spec.y.clamp(0.0, max_y), ..spec.clone() }
}

/// Tray menu entry point: AppHandle plumbing only, so the exchange and clamp math
/// above stay unit-testable without one. Device names come from a fresh
/// `available_monitors()` read at click time, never a cached list, so a monitor
/// unplugged since the tray was last hovered can't be swapped into. Mirrors
/// `set_lanes`'s persist + emit + reconcile sequence so live strips and overlays move
/// in the same atomic step, never a moment with a widget on both monitors or neither.
pub fn swap_monitors(app: &AppHandle) {
    let connected = crate::relocation::connected_monitor_devices(app);
    if connected.len() != 2 {
        log::info!("swap monitors: need exactly 2 monitors, found {} - no-op", connected.len());
        return;
    }
    let work_areas = crate::relocation::connected_work_areas(app);
    let device_a = connected[0].clone();
    let device_b = connected[1].clone();
    let Some(&(work_w_a, work_h_a)) = work_areas.get(&device_a) else {
        log::error!("swap monitors: no work area for {device_a}");
        return;
    };
    let Some(&(work_w_b, work_h_b)) = work_areas.get(&device_b) else {
        log::error!("swap monitors: no work area for {device_b}");
        return;
    };
    let primary_device = crate::relocation::primary_monitor_device(app);

    let mut settings = {
        let state = app.state::<SettingsState>();
        let Ok(mut s) = state.0.lock() else {
            log::error!("swap monitors: settings lock poisoned");
            return;
        };
        swap_lanes_and_overlays(&mut s, &device_a, &device_b, &primary_device);
        for placement in s.widget_placement.values_mut() {
            let Placement::Overlay(spec) = placement else { continue };
            if spec.monitor == device_a {
                *spec = clamp_to_monitor(spec, work_w_a, work_h_a);
            } else if spec.monitor == device_b {
                *spec = clamp_to_monitor(spec, work_w_b, work_h_b);
            }
        }
        s.clone()
    };
    if let Err(e) = crate::settings::persist(app, &mut settings) {
        log::error!("swap monitors: persist failed: {e}");
        return;
    }
    let _ = app.emit("widgets-changed", ());
    crate::overlay::reconcile(app);
    crate::strip::reconcile(app);
    log::info!("swap monitors: swapped {device_a} <-> {device_b}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::StripInstance;
    use std::collections::HashMap;

    fn instance(id: &str, widget_id: &str) -> StripInstance {
        StripInstance { instance_id: id.to_string(), widget_id: widget_id.to_string() }
    }

    fn overlay_spec(monitor: &str, x: f64, y: f64) -> OverlaySpec {
        OverlaySpec { monitor: monitor.to_string(), x, y, ..Default::default() }
    }

    fn lane_ids(s: &Settings, monitor: &str) -> Vec<String> {
        s.monitor_widgets.instances_for(monitor).iter().map(|si| si.instance_id.clone()).collect()
    }

    #[test]
    fn swap_exchanges_the_two_named_lanes() {
        let mut s = Settings {
            monitor_widgets: MonitorWidgets(HashMap::from([
                (r"\\.\DISPLAY1".to_string(), vec![instance("cpu#1", "cpu")]),
                (r"\\.\DISPLAY2".to_string(), vec![instance("ram#1", "ram")]),
            ])),
            ..Settings::default()
        };

        swap_lanes_and_overlays(&mut s, r"\\.\DISPLAY1", r"\\.\DISPLAY2", r"\\.\DISPLAY1");

        assert_eq!(lane_ids(&s, r"\\.\DISPLAY1"), ["ram#1"]);
        assert_eq!(lane_ids(&s, r"\\.\DISPLAY2"), ["cpu#1"]);
    }

    #[test]
    fn swap_folds_the_empty_sentinel_into_the_primary_device_first() {
        // Pre-lanes-UI install: the primary's widgets still sit under "".
        let mut s = Settings {
            monitor_widgets: MonitorWidgets(HashMap::from([
                ("".to_string(), vec![instance("cpu#1", "cpu")]),
                (r"\\.\DISPLAY2".to_string(), vec![instance("ram#1", "ram")]),
            ])),
            ..Settings::default()
        };

        swap_lanes_and_overlays(&mut s, r"\\.\DISPLAY1", r"\\.\DISPLAY2", r"\\.\DISPLAY1");

        assert!(lane_ids(&s, "").is_empty());
        assert_eq!(lane_ids(&s, r"\\.\DISPLAY1"), ["ram#1"]);
        assert_eq!(lane_ids(&s, r"\\.\DISPLAY2"), ["cpu#1"]);
    }

    #[test]
    fn swap_leaves_a_third_monitors_lane_untouched() {
        let mut s = Settings {
            monitor_widgets: MonitorWidgets(HashMap::from([
                (r"\\.\DISPLAY1".to_string(), vec![instance("cpu#1", "cpu")]),
                (r"\\.\DISPLAY2".to_string(), vec![instance("ram#1", "ram")]),
                (r"\\.\DISPLAY3".to_string(), vec![instance("gpu#1", "gpu")]),
            ])),
            ..Settings::default()
        };

        swap_lanes_and_overlays(&mut s, r"\\.\DISPLAY1", r"\\.\DISPLAY2", r"\\.\DISPLAY1");

        assert_eq!(lane_ids(&s, r"\\.\DISPLAY3"), ["gpu#1"]);
    }

    #[test]
    fn swap_flips_overlays_on_either_monitor() {
        let mut s = Settings::default();
        s.widget_placement.insert(
            "cpu".to_string(),
            Placement::Overlay(overlay_spec(r"\\.\DISPLAY1", 10.0, 10.0)),
        );
        s.widget_placement.insert(
            "ram".to_string(),
            Placement::Overlay(overlay_spec(r"\\.\DISPLAY2", 20.0, 20.0)),
        );
        s.widget_placement.insert("gpu".to_string(), Placement::Overlay(overlay_spec(r"\\.\DISPLAY3", 5.0, 5.0)));

        swap_lanes_and_overlays(&mut s, r"\\.\DISPLAY1", r"\\.\DISPLAY2", r"\\.\DISPLAY1");

        let monitor_of = |id: &str| match s.widget_placement.get(id) {
            Some(Placement::Overlay(spec)) => spec.monitor.clone(),
            _ => panic!("expected overlay"),
        };
        assert_eq!(monitor_of("cpu"), r"\\.\DISPLAY2");
        assert_eq!(monitor_of("ram"), r"\\.\DISPLAY1");
        assert_eq!(monitor_of("gpu"), r"\\.\DISPLAY3");
    }

    #[test]
    fn swap_treats_an_empty_overlay_monitor_as_the_primary() {
        let mut s = Settings::default();
        s.widget_placement.insert("cpu".to_string(), Placement::Overlay(overlay_spec("", 10.0, 10.0)));

        swap_lanes_and_overlays(&mut s, r"\\.\DISPLAY1", r"\\.\DISPLAY2", r"\\.\DISPLAY1");

        match s.widget_placement.get("cpu") {
            Some(Placement::Overlay(spec)) => assert_eq!(spec.monitor, r"\\.\DISPLAY2"),
            _ => panic!("expected overlay"),
        }
    }

    #[test]
    fn clamp_pulls_an_overlay_off_the_bottom_right_back_on_screen() {
        let spec = overlay_spec(r"\\.\DISPLAY2", 2000.0, 3800.0);

        let clamped = clamp_to_monitor(&spec, 2160.0, 3840.0);

        assert!(clamped.x + overlay::FALLBACK_W <= 2160.0);
        assert!(clamped.y + overlay::FALLBACK_H <= 3840.0);
    }

    #[test]
    fn clamp_leaves_an_overlay_already_inside_unchanged() {
        let spec = overlay_spec(r"\\.\DISPLAY1", 50.0, 50.0);

        let clamped = clamp_to_monitor(&spec, 1920.0, 1080.0);

        assert_eq!(clamped.x, 50.0);
        assert_eq!(clamped.y, 50.0);
    }

    #[test]
    fn clamp_uses_the_overlays_own_stored_size_not_the_fallback() {
        let spec = OverlaySpec { monitor: r"\\.\DISPLAY2".to_string(), x: 190.0, y: 90.0, w: Some(100.0), h: Some(80.0), ..Default::default() };

        let clamped = clamp_to_monitor(&spec, 200.0, 100.0);

        assert_eq!(clamped.x, 100.0);
        assert_eq!(clamped.y, 20.0);
    }
}
