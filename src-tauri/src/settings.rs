use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_kit_settings::KitSettings;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    // CSS px between the taskbar's left edge and the strip (left side is empty on Win11).
    pub left_margin: u32,
    pub enabled_widgets: Vec<String>,
    pub stats_poll_seconds: u32,
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
            stats_poll_seconds: 2,
            kit: KitSettings::default(),
        }
    }
}

pub struct SettingsState(pub Mutex<Settings>);

const SETTINGS_FILENAME: &str = "settings.json";

pub fn load(app: &AppHandle) -> Settings {
    let mut settings =
        tauri_kit_settings::load_for::<_, Settings>(app, SETTINGS_FILENAME).unwrap_or_default();
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
