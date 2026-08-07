use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_kit_settings::KitSettings;

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
    // Fullscreen still hides the strip in both modes; this only gates taskbar_hidden().
    pub follow_taskbar: bool,
    // Opt-in only: excludes strip+flyout from screen capture, but also from the
    // user's own screenshots, so it must never default true.
    pub hide_from_capture: bool,
    // Keyed by widget id; each value is that widget's own free-form config object.
    pub widget_config: HashMap<String, serde_json::Value>,
    // One-time guard for the divider backfill in `load`; must stay false once so a
    // pre-existing settings.json (missing this key) still runs it exactly once.
    pub dividers_migrated: bool,
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
            widget_config: HashMap::new(),
            dividers_migrated: false,
            kit: KitSettings::default(),
        }
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
    fn fresh_install_skips_migration_but_is_marked_done() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let loaded = load(&path);

        assert_eq!(shape(&loaded.enabled_widgets), Settings::default().enabled_widgets);
        assert!(loaded.dividers_migrated);
    }
}
