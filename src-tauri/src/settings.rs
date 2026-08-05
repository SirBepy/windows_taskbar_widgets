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
    // Widgets a user hid via the tile menu or dashboard; kept (not dropped) so re-enabling
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

pub fn load(path: &Path) -> Settings {
    let mut settings: Settings = tauri_kit_settings::store::load(path).unwrap_or_default();
    // Pre-split "system" widget id: expand in place so existing tile order/position is kept.
    if let Some(i) = settings.enabled_widgets.iter().position(|id| id == "system") {
        settings.enabled_widgets.splice(
            i..=i,
            ["cpu", "ram", "gpu", "disk"].map(String::from),
        );
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
}
