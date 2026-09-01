use crate::monitor_widgets::{InstanceId, StripInstance};
use crate::settings::{Settings, DIVIDER_PREFIX};
use std::collections::HashMap;

const STAT_WIDGET_IDS: [&str; 4] = ["cpu", "ram", "gpu", "disk"];

// Matches the pre-0c4e816 `.tile-stat` border-right: a divider between each adjacent
// pair of stat tiles, never trailing (the old `:not(:has(~ .tile .tile-stat))` rule
// cleared it from the last one). No-ops if a divider is already present anywhere,
// so a partially-migrated or hand-edited list is left alone.
pub(crate) fn migrate_dividers(enabled_widgets: &mut Vec<String>) {
    if enabled_widgets.iter().any(|id| id.starts_with(DIVIDER_PREFIX)) {
        return;
    }
    let mut out = Vec::with_capacity(enabled_widgets.len());
    for (i, id) in enabled_widgets.iter().enumerate() {
        out.push(id.clone());
        let next_is_stat = enabled_widgets
            .get(i + 1)
            .is_some_and(|n| STAT_WIDGET_IDS.contains(&n.as_str()));
        if STAT_WIDGET_IDS.contains(&id.as_str()) && next_is_stat {
            out.push(format!("{DIVIDER_PREFIX}{}", uuid::Uuid::new_v4()));
        }
    }
    *enabled_widgets = out;
}

// Backfills monitor_widgets and remaps hidden_widgets from the flat, pre-instance
// model. widget_config/widget_placement deliberately stay KIND-keyed: every reader
// looks them up by kind, so remapping them here only orphans the entry. "#1" can't
// collide pre-migration; dividers keep their own id (see next_instance_id).
pub(crate) fn migrate_to_instances(settings: &mut Settings) {
    if settings.widgets_migrated_to_instances {
        return;
    }
    let monitor = settings.taskbar_monitor.clone();
    let mut id_to_instance: HashMap<String, InstanceId> = HashMap::new();
    let mut instances = Vec::with_capacity(settings.enabled_widgets.len());

    for widget_id in &settings.enabled_widgets {
        let instance_id = if widget_id.starts_with(DIVIDER_PREFIX) {
            widget_id.clone()
        } else {
            format!("{widget_id}#1")
        };
        id_to_instance.insert(widget_id.clone(), instance_id.clone());
        instances.push(StripInstance { instance_id, widget_id: widget_id.clone() });
    }
    settings.monitor_widgets.0.insert(monitor, instances);

    let remap = |old: &str| id_to_instance.get(old).cloned().unwrap_or_else(|| format!("{old}#1"));
    settings.hidden_widgets = settings.hidden_widgets.iter().map(|id| remap(id)).collect();

    settings.widgets_migrated_to_instances = true;
}

// Un-orphans widget_config/widget_placement entries that 0.1.10's since-fixed
// migrate_to_instances left instance-keyed. Resolves via monitor_widgets, never by
// stripping "#1", so a divider id or a kind containing "#" survives. Idempotent,
// and an existing kind-keyed entry always beats its instance-keyed twin.
pub(crate) fn repair_kind_keyed_maps(settings: &mut Settings) {
    let kinds: std::collections::HashSet<String> =
        settings.monitor_widgets.all().map(|si| si.widget_id.clone()).collect();
    let instance_to_kind: HashMap<String, String> = settings
        .monitor_widgets
        .all()
        .map(|si| (si.instance_id.clone(), si.widget_id.clone()))
        .collect();

    repair_map(&mut settings.widget_config, &kinds, &instance_to_kind);
    repair_map(&mut settings.widget_placement, &kinds, &instance_to_kind);
}

fn repair_map<V>(
    map: &mut HashMap<String, V>,
    kinds: &std::collections::HashSet<String>,
    instance_to_kind: &HashMap<String, String>,
) {
    // Kind-keyed entries always win: they overwrite unconditionally, while an
    // instance-keyed entry only fills a gap (or_insert), regardless of drain order.
    for (key, value) in std::mem::take(map) {
        if kinds.contains(&key) {
            map.insert(key, value);
        } else if let Some(kind) = instance_to_kind.get(&key) {
            map.entry(kind.clone()).or_insert(value);
        } else {
            map.insert(key, value);
        }
    }
}

impl Settings {
    /// Additive self-heal: any enabled_widgets id with no StripInstance anywhere in
    /// monitor_widgets (e.g. a registry widget adopted post-first-run, see main.ts's
    /// newIds backfill) gets one in the primary "" lane. Never removes an instance -
    /// one can legitimately live on a monitor lane whose kind isn't in enabled_widgets.
    pub(crate) fn ensure_instances(&mut self) {
        for widget_id in self.enabled_widgets.clone() {
            if self.monitor_widgets.all().any(|si| si.widget_id == widget_id) {
                continue;
            }
            let instance_id = self.monitor_widgets.next_instance_id(&widget_id);
            self.monitor_widgets.0.entry(String::new()).or_default().push(StripInstance {
                instance_id,
                widget_id,
            });
        }
    }

    /// A kind re-added to enabled_widgets must clear BOTH hidden_widgets shapes:
    /// apply_hide's bare-kind entry AND any orphaned "<kind>#n" it leaves behind,
    /// which removeId's bare-kind-only removal never touches. Scoped to just-
    /// reintroduced kinds so an untouched kind's per-instance hide stays intact.
    pub(crate) fn clear_hidden_for_reenabled_widgets(&mut self, previously_enabled: &[String]) {
        let reintroduced: Vec<&str> = self
            .enabled_widgets
            .iter()
            .filter(|id| !previously_enabled.contains(id))
            .map(String::as_str)
            .collect();
        if reintroduced.is_empty() {
            return;
        }
        let instances: Vec<StripInstance> = self.monitor_widgets.all().cloned().collect();
        self.hidden_widgets.retain(|h| {
            !reintroduced.iter().any(|kind| {
                h == kind || instances.iter().any(|si| &si.instance_id == h && &si.widget_id == *kind)
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{load, OverlaySpec, Placement};
    use tempfile::tempdir;

    // Collapses each divider id to "D" so assertions don't depend on its uuid suffix.
    fn shape(ids: &[String]) -> Vec<String> {
        ids.iter()
            .map(|id| if id.starts_with(DIVIDER_PREFIX) { "D".to_string() } else { id.clone() })
            .collect()
    }

    #[test]
    fn migrates_dividers_between_adjacent_stat_tiles_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let written = Settings {
            enabled_widgets: ["cpu", "ram", "gpu", "disk", "conductor"].map(String::from).into(),
            ..Settings::default()
        };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        let loaded = load(&path);

        assert_eq!(
            shape(&loaded.enabled_widgets),
            ["cpu", "D", "ram", "D", "gpu", "D", "disk", "conductor"]
        );
        assert!(loaded.dividers_migrated);
    }

    #[test]
    fn skips_divider_across_a_non_stat_boundary() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let written = Settings {
            enabled_widgets: ["cpu", "ram", "conductor", "gpu", "disk"].map(String::from).into(),
            ..Settings::default()
        };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        let loaded = load(&path);

        assert_eq!(shape(&loaded.enabled_widgets), ["cpu", "D", "ram", "conductor", "gpu", "D", "disk"]);
    }

    #[test]
    fn migration_does_not_rerun_once_marked() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let written = Settings {
            enabled_widgets: ["cpu", "ram"].map(String::from).into(),
            dividers_migrated: true,
            ..Settings::default()
        };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        let loaded = load(&path);

        assert_eq!(shape(&loaded.enabled_widgets), ["cpu", "ram"]);
    }

    #[test]
    fn leaves_a_list_that_already_has_a_divider_alone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let written = Settings {
            enabled_widgets: ["cpu", "divider:existing", "ram", "gpu", "disk"].map(String::from).into(),
            ..Settings::default()
        };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        let loaded = load(&path);

        assert_eq!(loaded.enabled_widgets, ["cpu", "divider:existing", "ram", "gpu", "disk"]);
        assert!(loaded.dividers_migrated);
    }

    #[test]
    fn fresh_install_skips_migration_but_is_marked_done() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let loaded = load(&path);

        assert_eq!(shape(&loaded.enabled_widgets), Settings::default().enabled_widgets);
        assert!(loaded.dividers_migrated);
    }

    // The whole migration story for instances: a settings.json written before
    // monitor_widgets existed must load with monitor_widgets backfilled by instance
    // id, while widget_config/widget_placement stay keyed by kind (every reader
    // looks them up by kind, not instance id).
    #[test]
    fn settings_without_monitor_widgets_migrates_enabled_widgets_to_instances() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let json = r#"{
            "left_margin":44,
            "enabled_widgets":["cpu","ram","conductor"],
            "opacity":65,
            "dividers_migrated":true,
            "widget_config":{"cpu":{"foo":1}},
            "widget_placement":{"ram":{"kind":"strip"}}
        }"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load(&path);

        let instances = loaded.monitor_widgets.instances_for("");
        let instance_ids: Vec<&str> = instances.iter().map(|si| si.instance_id.as_str()).collect();
        let widget_ids: Vec<&str> = instances.iter().map(|si| si.widget_id.as_str()).collect();
        assert_eq!(instance_ids, ["cpu#1", "ram#1", "conductor#1"]);
        assert_eq!(widget_ids, ["cpu", "ram", "conductor"]);
        assert_eq!(loaded.widget_config.get("cpu").and_then(|v| v.get("foo")), Some(&serde_json::json!(1)));
        assert_eq!(loaded.widget_placement.get("ram"), Some(&Placement::Strip));
        assert_eq!(loaded.left_margin, 44);
        assert_eq!(loaded.opacity, 65);
        assert_eq!(loaded.enabled_widgets, ["cpu", "ram", "conductor"]);
        assert!(loaded.widgets_migrated_to_instances);
    }

    // Bug 3: migrate_to_instances used to remap widget_config/widget_placement keys
    // to instance ids too, orphaning both since every reader looks them up by kind.
    #[test]
    fn migrate_to_instances_leaves_config_and_placement_kind_keyed_but_remaps_hidden() {
        let mut s = Settings {
            enabled_widgets: ["cpu", "ram"].map(String::from).into(),
            hidden_widgets: vec!["ram".to_string()],
            widgets_migrated_to_instances: false,
            ..Settings::default()
        };
        s.widget_config.insert("cpu".to_string(), serde_json::json!({"foo": 1}));
        s.widget_placement.insert("ram".to_string(), Placement::Strip);

        migrate_to_instances(&mut s);

        assert_eq!(s.widget_config.get("cpu").and_then(|v| v.get("foo")), Some(&serde_json::json!(1)));
        assert_eq!(s.widget_placement.get("ram"), Some(&Placement::Strip));
        assert_eq!(s.hidden_widgets, ["ram#1"]);
    }

    // Regression: a pre-migration settings.json with a Floating widget must still be
    // reported by overlays() after migrate_to_instances runs. Before the fix, the
    // remap orphaned "conductor" to "conductor#1" and the overlay silently vanished.
    #[test]
    fn floating_overlay_survives_migration_to_instances() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let json = r#"{
            "left_margin":12,
            "enabled_widgets":["cpu","conductor"],
            "dividers_migrated":true,
            "widget_placement":{"conductor":{"kind":"overlay","monitor":"","x":10.0,"y":20.0}}
        }"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load(&path);

        let overlay_ids: Vec<String> = loaded.overlays().into_iter().map(|(_, widget_id, _)| widget_id).collect();
        assert_eq!(overlay_ids, ["conductor"]);
    }

    #[test]
    fn repair_rekeys_an_instance_keyed_widget_placement_entry() {
        let mut s = Settings::default();
        s.monitor_widgets.0.entry(String::new()).or_default().push(StripInstance {
            instance_id: "pomodoro#1".into(),
            widget_id: "pomodoro".into(),
        });
        let spec = OverlaySpec { x: 5.0, y: 6.0, ..Default::default() };
        s.widget_placement.insert("pomodoro#1".to_string(), Placement::Overlay(spec.clone()));

        repair_kind_keyed_maps(&mut s);

        assert_eq!(s.widget_placement.get("pomodoro"), Some(&Placement::Overlay(spec)));
        assert!(!s.widget_placement.contains_key("pomodoro#1"));
    }

    #[test]
    fn repair_rekeys_an_instance_keyed_widget_config_entry() {
        let mut s = Settings::default();
        s.widget_config.insert("cpu#1".to_string(), serde_json::json!({"show_percent": true}));

        repair_kind_keyed_maps(&mut s);

        assert_eq!(s.widget_config.get("cpu").and_then(|v| v.get("show_percent")), Some(&serde_json::json!(true)));
        assert!(!s.widget_config.contains_key("cpu#1"));
    }

    #[test]
    fn repair_leaves_a_divider_id_key_untouched() {
        let mut s = Settings::default();
        let divider_id = format!("{DIVIDER_PREFIX}abc-123");
        s.widget_config.insert(divider_id.clone(), serde_json::json!({"unused": true}));

        repair_kind_keyed_maps(&mut s);

        assert_eq!(s.widget_config.get(&divider_id), Some(&serde_json::json!({"unused": true})));
    }

    #[test]
    fn repair_keeps_the_kind_keyed_value_on_collision() {
        let mut s = Settings::default();
        s.widget_placement.insert("cpu".to_string(), Placement::Strip);
        let overlay = Placement::Overlay(OverlaySpec { x: 1.0, y: 1.0, ..Default::default() });
        s.widget_placement.insert("cpu#1".to_string(), overlay);

        repair_kind_keyed_maps(&mut s);

        assert_eq!(s.widget_placement.get("cpu"), Some(&Placement::Strip));
        assert!(!s.widget_placement.contains_key("cpu#1"));
    }

    #[test]
    fn repair_is_idempotent() {
        let mut s = Settings::default();
        s.monitor_widgets.0.entry(String::new()).or_default().push(StripInstance {
            instance_id: "pomodoro#1".into(),
            widget_id: "pomodoro".into(),
        });
        s.widget_placement.insert("pomodoro#1".to_string(), Placement::Strip);

        repair_kind_keyed_maps(&mut s);
        let after_first = s.widget_placement.clone();
        repair_kind_keyed_maps(&mut s);

        assert_eq!(s.widget_placement, after_first);
    }

    #[test]
    fn migrate_to_instances_is_idempotent() {
        let mut s = Settings {
            enabled_widgets: ["cpu", "ram"].map(String::from).into(),
            widgets_migrated_to_instances: false,
            ..Settings::default()
        };

        migrate_to_instances(&mut s);
        let monitor_widgets = s.monitor_widgets.clone();
        let hidden_widgets = s.hidden_widgets.clone();
        let widget_config = s.widget_config.clone();
        let widget_placement = s.widget_placement.clone();

        migrate_to_instances(&mut s);

        assert_eq!(s.monitor_widgets, monitor_widgets);
        assert_eq!(s.hidden_widgets, hidden_widgets);
        assert_eq!(s.widget_config, widget_config);
        assert_eq!(s.widget_placement, widget_placement);
        assert!(s.widgets_migrated_to_instances);
    }

    // Bug 1: pomodoro/spotify are shipped registry widgets not in Settings::default's
    // monitor_widgets; main.ts adopts them into enabled_widgets via reorder_widgets,
    // which only ever wrote enabled_widgets.
    #[test]
    fn ensure_instances_adds_a_missing_instance_to_the_primary_lane_with_a_non_colliding_id() {
        let mut s = Settings { enabled_widgets: vec!["cpu".to_string(), "pomodoro".to_string()], ..Settings::default() };

        s.ensure_instances();

        let primary = s.monitor_widgets.instances_for("");
        assert!(primary.iter().any(|si| si.instance_id == "pomodoro#1" && si.widget_id == "pomodoro"));
        // cpu already had "cpu#1" from Settings::default(); no second instance appears.
        assert_eq!(primary.iter().filter(|si| si.widget_id == "cpu").count(), 1);
    }

    #[test]
    fn ensure_instances_skips_a_kind_whose_instance_lives_on_another_monitor() {
        let mut s = Settings::default();
        s.monitor_widgets.0.insert(
            r"\\.\DISPLAY2".to_string(),
            vec![StripInstance { instance_id: "pomodoro#1".into(), widget_id: "pomodoro".into() }],
        );
        s.enabled_widgets.push("pomodoro".to_string());

        s.ensure_instances();

        assert!(s.monitor_widgets.instances_for("").iter().all(|si| si.widget_id != "pomodoro"));
        assert_eq!(s.monitor_widgets.all().filter(|si| si.widget_id == "pomodoro").count(), 1);
    }

    #[test]
    fn ensure_instances_is_idempotent() {
        let mut s = Settings { enabled_widgets: vec!["cpu".to_string(), "pomodoro".to_string()], ..Settings::default() };
        s.ensure_instances();
        let after_first = s.monitor_widgets.clone();

        s.ensure_instances();

        assert_eq!(s.monitor_widgets, after_first);
    }

    #[test]
    fn ensure_instances_never_removes_an_instance_absent_from_enabled_widgets() {
        let mut s = Settings { enabled_widgets: vec!["cpu".to_string()], ..Settings::default() };
        s.monitor_widgets.0.insert(
            r"\\.\DISPLAY2".to_string(),
            vec![StripInstance { instance_id: "gpu#2".into(), widget_id: "gpu".into() }],
        );

        s.ensure_instances();

        assert!(s.monitor_widgets.all().any(|si| si.instance_id == "gpu#2"));
    }

    // Bug 2 regression: apply_hide (tile_menu.rs) orphans "cpu#1" in hidden_widgets
    // once no visible sibling remains; a settings-UI re-add only ever removed the
    // bare kind, leaving "cpu#1" behind and is_active permanently false.
    #[test]
    fn reenabling_a_widget_clears_its_orphaned_instance_hide() {
        let mut s = Settings::default();
        s.hidden_widgets = vec!["cpu#1".to_string(), "cpu".to_string()];
        s.enabled_widgets.retain(|w| w != "cpu"); // mirrors apply_hide's enabled_widgets mutation

        // save_settings diffs against the state as it was persisted just before this
        // save, i.e. still missing "cpu".
        let previously_enabled = s.enabled_widgets.clone();

        // Settings UI re-add: "cpu" comes back into enabled_widgets, bare kind
        // removed from hidden_widgets (mirrors onStripDrop's removeId), but the
        // per-instance "cpu#1" entry is left in, same as the real bug.
        s.enabled_widgets.push("cpu".to_string());
        s.hidden_widgets.retain(|h| h != "cpu");
        assert!(!s.is_active("cpu#1"), "sanity: orphaned instance id still hides it");

        s.clear_hidden_for_reenabled_widgets(&previously_enabled);

        assert!(s.is_active("cpu#1"));
    }

    #[test]
    fn clear_hidden_for_reenabled_widgets_leaves_an_untouched_kinds_instance_hide_alone() {
        let mut s = two_monitor_settings_for_hidden_test();
        let previously_enabled = s.enabled_widgets.clone();
        s.hidden_widgets = vec!["cpu#1".to_string()];
        s.enabled_widgets.push("pomodoro".to_string());

        s.clear_hidden_for_reenabled_widgets(&previously_enabled);

        assert!(s.hidden_widgets.contains(&"cpu#1".to_string()));
    }

    fn two_monitor_settings_for_hidden_test() -> Settings {
        let mut s = Settings::default();
        s.monitor_widgets.0.insert(
            r"\\.\DISPLAY2".to_string(),
            vec![StripInstance { instance_id: "cpu#2".into(), widget_id: "cpu".into() }],
        );
        s
    }
}
