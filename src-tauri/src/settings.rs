use crate::settings_migrations::{migrate_dividers, migrate_to_instances, repair_kind_keyed_maps};
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

// Mirrors DIVIDER_PREFIX in src/shared/divider.ts; both sides must agree exactly.
pub const DIVIDER_PREFIX: &str = "divider:";

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
    // Repairs any widget_config/widget_placement entry stranded instance-keyed by
    // 0.1.10's now-fixed migrate_to_instances. Unconditional and idempotent, so it's
    // safe on every load rather than gated behind another migration flag.
    repair_kind_keyed_maps(&mut settings);
    // Repairs an existing install with no waiting on a save: same self-heal persist() runs.
    settings.ensure_instances();
    settings
}

pub fn persist(app: &AppHandle, settings: &mut Settings) -> Result<(), String> {
    settings.ensure_instances();
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
    fn default_settings_has_matching_monitor_widgets_shape() {
        let s = Settings::default();

        let ids: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(ids, ["cpu#1", "ram#1", "gpu#1", "disk#1", "conductor#1"]);
        assert!(!s.widgets_migrated_to_instances);
    }
}
