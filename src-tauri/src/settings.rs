use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
