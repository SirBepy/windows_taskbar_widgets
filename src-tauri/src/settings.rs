use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_kit_settings::KitSettings;

// x/y are CSS px relative to the monitor's work area, never desktop coords, so a
// resolution change or a display rearrange doesn't strand an overlay off screen.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct OverlaySpec {
    // "" means primary; otherwise a device name, same convention as taskbar_monitor.
    #[serde(default)]
    pub monitor: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    // None means "the size the widget declared"; Some once the user has resized it.
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
    #[serde(default)]
    pub opacity: Option<u32>,
}

// Internally tagged on purpose: serde's default external tagging would write
// {"Overlay":{..}}, a shape nothing else in this settings file uses.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Strip,
    Overlay(OverlaySpec),
}

pub use crate::monitor_widgets::{InstanceId, MonitorWidgets, StripInstance};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    // CSS px between the taskbar's left edge and the strip (left side is empty on Win11).
    pub left_margin: u32,
    pub enabled_widgets: Vec<String>,
    // Widgets a user hid via the tile menu or settings; kept (not dropped) so re-enabling
    // doesn't lose the id or its position semantics.
    pub hidden_widgets: Vec<String>,
    pub stats_poll_seconds: u32,
    // 0-100, base opacity for all tiles/flyouts; hover always shows fully opaque.
    pub opacity: u32,
    // true: strip hides when the taskbar does. false: strip stays visible regardless.
    // Fullscreen still hides the strip in both modes; this only gates the taskbar check
    // in autohide.rs's strip_should_hide().
    pub follow_taskbar: bool,
    // Opt-in only: excludes strip+flyout from screen capture, but also from the
    // user's own screenshots, so it must never default true.
    pub hide_from_capture: bool,
    // "" means "whatever is primary right now" (default); otherwise a device name
    // like "\\.\DISPLAY2" pinning a specific monitor's taskbar. See taskbar::select_taskbar.
    pub taskbar_monitor: String,
    // Keyed by widget id; each value is that widget's own free-form config object.
    pub widget_config: HashMap<String, serde_json::Value>,
    // Keyed by widget id; an absent key means Placement::Strip, which is what makes
    // this migration-free for every existing install.
    pub widget_placement: HashMap<String, Placement>,
    // One-time guard for the divider backfill in `load`; must stay false once so a
    // pre-existing settings.json (missing this key) still runs it exactly once.
    pub dividers_migrated: bool,
    // Placed widget copies, per monitor. Instance-id-keyed; the successor to
    // enabled_widgets, which stays around read-only until callers migrate to this.
    pub monitor_widgets: MonitorWidgets,
    // One-time guard for the enabled_widgets -> monitor_widgets backfill in `load`;
    // same "stay false once" contract as dividers_migrated.
    pub widgets_migrated_to_instances: bool,
    #[serde(flatten)]
    pub kit: KitSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            left_margin: 12,
            enabled_widgets: vec![
                "cpu".to_string(),
                "ram".to_string(),
                "gpu".to_string(),
                "disk".to_string(),
                "conductor".to_string(),
            ],
            hidden_widgets: Vec::new(),
            stats_poll_seconds: 2,
            opacity: 100,
            follow_taskbar: true,
            hide_from_capture: false,
            taskbar_monitor: String::new(),
            widget_config: HashMap::new(),
            widget_placement: HashMap::new(),
            dividers_migrated: false,
            monitor_widgets: MonitorWidgets(HashMap::from([(
                String::new(),
                ["cpu", "ram", "gpu", "disk", "conductor"]
                    .map(|id| StripInstance { instance_id: format!("{id}#1"), widget_id: id.to_string() })
                    .into(),
            )])),
            widgets_migrated_to_instances: false,
            kit: KitSettings::default(),
        }
    }
}

impl Settings {
    /// True when this instance exists and neither its instance id (tile-menu hide)
    /// nor its widget kind (the settings UI's strip-remove still writes the bare
    /// kind, no instance) appear in hidden_widgets.
    pub fn is_active(&self, instance_id: &str) -> bool {
        self.monitor_widgets.all().any(|si| {
            si.instance_id == instance_id
                && !self.hidden_widgets.iter().any(|h| h == instance_id || h == &si.widget_id)
        })
    }

    /// Ids placed as overlays right now, paired with their widget kind (so
    /// `overlay_widget_id` can report it) and spec. widget_placement is still
    /// looked up by kind: the settings UI's placement toggle has no per-instance
    /// affordance yet, so every instance of a kind shares that kind's one spec.
    pub fn overlays(&self) -> Vec<(InstanceId, String, OverlaySpec)> {
        self.monitor_widgets
            .all()
            .filter(|si| self.is_active(&si.instance_id))
            .filter_map(|si| match self.widget_placement.get(&si.widget_id) {
                Some(Placement::Overlay(spec)) => Some((si.instance_id.clone(), si.widget_id.clone(), spec.clone())),
                _ => None,
            })
            .collect()
    }

    /// Any non-hidden placement of `widget_id` on any monitor - what
    /// bridge_pomodoro.rs needs, since it only knows a kind, never a placement.
    /// Checks both hidden_widgets shapes, same reasoning as `is_active` above.
    pub fn is_widget_active(&self, widget_id: &str) -> bool {
        self.monitor_widgets.all().any(|si| {
            si.widget_id == widget_id
                && !self.hidden_widgets.iter().any(|h| h == &si.instance_id || h == widget_id)
        })
    }
}

pub struct SettingsState(pub Mutex<Settings>);

const SETTINGS_FILENAME: &str = "settings.json";

// Mirrors tauri_kit_settings::paths::settings_path but takes the bundle identifier
// directly instead of an AppHandle, so it can resolve before any window (and its
// webview JS) is built - AppHandle doesn't exist that early.
pub fn resolve_path(identifier: &str) -> std::io::Result<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?
        .join(identifier);
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(SETTINGS_FILENAME))
}

const STAT_WIDGET_IDS: [&str; 4] = ["cpu", "ram", "gpu", "disk"];

// Mirrors DIVIDER_PREFIX in src/shared/divider.ts; both sides must agree exactly.
pub const DIVIDER_PREFIX: &str = "divider:";

// Matches the pre-0c4e816 `.tile-stat` border-right: a divider between each adjacent
// pair of stat tiles, never trailing (the old `:not(:has(~ .tile .tile-stat))` rule
// cleared it from the last one). No-ops if a divider is already present anywhere,
// so a partially-migrated or hand-edited list is left alone.
fn migrate_dividers(enabled_widgets: &mut Vec<String>) {
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

// Backfills monitor_widgets/instance-keyed maps from the flat, pre-instance model.
// Plain "#1" suffix, never the collision-avoiding next_instance_id - at most one of
// each widget id can exist pre-migration, so "#1" can't collide. Dividers keep their
// own id unchanged (same prefix rule as MonitorWidgets::next_instance_id).
fn migrate_to_instances(settings: &mut Settings) {
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
    settings.widget_config = std::mem::take(&mut settings.widget_config)
        .into_iter()
        .map(|(id, v)| (remap(&id), v))
        .collect();
    settings.widget_placement = std::mem::take(&mut settings.widget_placement)
        .into_iter()
        .map(|(id, p)| (remap(&id), p))
        .collect();

    settings.widgets_migrated_to_instances = true;
}

pub fn load(path: &Path) -> Settings {
    let existed = path.exists();
    let mut settings: Settings = tauri_kit_settings::store::load(path).unwrap_or_default();
    // Pre-split "system" widget id: expand in place so existing tile order/position is kept.
    if let Some(i) = settings.enabled_widgets.iter().position(|id| id == "system") {
        settings.enabled_widgets.splice(
            i..=i,
            ["cpu", "ram", "gpu", "disk"].map(String::from),
        );
    }
    // A fresh install (no file) has nothing to backfill; mark it migrated so first-run
    // defaults are never retro-fitted with dividers later.
    if !settings.dividers_migrated {
        if existed {
            migrate_dividers(&mut settings.enabled_widgets);
        }
        settings.dividers_migrated = true;
    }
    // Same fresh-install-skips-it shape: Settings::default() already has the right
    // monitor_widgets, so a fresh install just marks itself migrated without running
    // the backfill.
    if !settings.widgets_migrated_to_instances {
        if existed {
            migrate_to_instances(&mut settings);
        } else {
            settings.widgets_migrated_to_instances = true;
        }
    }
    settings
}

pub fn persist(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    tauri_kit_settings::save_for(app, SETTINGS_FILENAME, settings).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Proves deserialization, not the startup race (that needs a running app). Guards
    // against `load` silently falling back to Default on a file with real values.
    #[test]
    fn load_returns_file_values_not_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let written = Settings { left_margin: 44, opacity: 65, hide_from_capture: true, ..Settings::default() };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        let loaded = load(&path);

        assert_eq!(loaded.left_margin, 44);
        assert_eq!(loaded.opacity, 65);
        assert!(loaded.hide_from_capture);
        assert_ne!(loaded.left_margin, Settings::default().left_margin);
    }

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
    fn taskbar_monitor_defaults_empty_and_round_trips_a_saved_device() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(load(&path).taskbar_monitor, "");

        let written = Settings { taskbar_monitor: r"\\.\DISPLAY2".to_string(), ..Settings::default() };
        tauri_kit_settings::store::save(&path, &written).unwrap();

        assert_eq!(load(&path).taskbar_monitor, r"\\.\DISPLAY2");
    }

    // The whole migration story for overlay placement: a settings.json written before
    // widget_placement existed must load with every other value intact.
    #[test]
    fn settings_without_widget_placement_load_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let json = r#"{"left_margin":44,"enabled_widgets":["cpu","ram"],"opacity":65,"dividers_migrated":true}"#;
        std::fs::write(&path, json).unwrap();

        let loaded = load(&path);

        assert!(loaded.widget_placement.is_empty());
        assert_eq!(loaded.left_margin, 44);
        assert_eq!(loaded.enabled_widgets, ["cpu", "ram"]);
    }

    #[test]
    fn placement_round_trips_as_an_internally_tagged_map() {
        let spec = OverlaySpec { monitor: r"\\.\DISPLAY2".into(), x: 12.0, y: 40.0, w: Some(300.0), ..Default::default() };
        let json = serde_json::to_string(&Placement::Overlay(spec.clone())).unwrap();

        assert!(json.contains(r#""kind":"overlay""#), "got {json}");
        assert_eq!(serde_json::from_str::<Placement>(&json).unwrap(), Placement::Overlay(spec));
        assert_eq!(serde_json::to_string(&Placement::Strip).unwrap(), r#"{"kind":"strip"}"#);
    }

    #[test]
    fn overlays_skips_hidden_and_strip_placed_widgets() {
        let mut s = Settings { hidden_widgets: vec!["ram".to_string()], ..Settings::default() };
        let spec = OverlaySpec { x: 10.0, y: 10.0, ..Default::default() };
        s.widget_placement.insert("cpu".into(), Placement::Overlay(spec.clone()));
        s.widget_placement.insert("ram".into(), Placement::Overlay(spec));
        s.widget_placement.insert("gpu".into(), Placement::Strip);

        let ids: Vec<String> = s.overlays().into_iter().map(|(instance_id, _, _)| instance_id).collect();

        assert_eq!(ids, ["cpu#1"]);
    }

    #[test]
    fn is_widget_active_true_for_one_visible_placement() {
        assert!(Settings::default().is_widget_active("cpu"));
    }

    #[test]
    fn is_widget_active_false_when_hidden_by_instance_id() {
        let s = Settings { hidden_widgets: vec!["cpu#1".to_string()], ..Settings::default() };
        assert!(!s.is_widget_active("cpu"));
    }

    #[test]
    fn is_widget_active_false_when_hidden_by_legacy_widget_id() {
        let s = Settings { hidden_widgets: vec!["cpu".to_string()], ..Settings::default() };
        assert!(!s.is_widget_active("cpu"));
    }

    #[test]
    fn is_widget_active_true_with_one_hidden_and_one_visible_sibling() {
        let mut s = Settings { hidden_widgets: vec!["cpu#1".to_string()], ..Settings::default() };
        s.monitor_widgets.0.insert(
            r"\\.\DISPLAY2".to_string(),
            vec![StripInstance { instance_id: "cpu#2".into(), widget_id: "cpu".into() }],
        );

        assert!(s.is_widget_active("cpu"));
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
    // monitor_widgets existed must load with every widget re-keyed by instance id.
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
        assert_eq!(loaded.widget_config.get("cpu#1").and_then(|v| v.get("foo")), Some(&serde_json::json!(1)));
        assert_eq!(loaded.widget_placement.get("ram#1"), Some(&Placement::Strip));
        assert_eq!(loaded.left_margin, 44);
        assert_eq!(loaded.opacity, 65);
        assert_eq!(loaded.enabled_widgets, ["cpu", "ram", "conductor"]);
        assert!(loaded.widgets_migrated_to_instances);
    }

    #[test]
    fn default_settings_has_matching_monitor_widgets_shape() {
        let s = Settings::default();

        let ids: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(ids, ["cpu#1", "ram#1", "gpu#1", "disk#1", "conductor#1"]);
        assert!(!s.widgets_migrated_to_instances);
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
}
