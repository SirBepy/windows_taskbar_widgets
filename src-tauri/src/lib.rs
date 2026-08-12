mod autohide;
mod autostart;
mod bridge_conductor;
mod bridge_pomodoro;
mod conductor_data;
mod flyout;
mod monitor_widgets;
mod overlay;
mod settings;
mod spotify_smtc;
mod strip;
mod system_stats;
mod taskbar;
mod tile_menu;

use settings::{Settings, SettingsState};
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WindowEvent,
};

#[tauri::command]
fn get_settings(state: tauri::State<SettingsState>) -> Settings {
    // A standard Mutex stays poisoned forever once a panic occurs while held, so every
    // reader must recover via into_inner() rather than fall back to defaults on every
    // call for the rest of the process lifetime.
    state
        .0
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
}

// Param named `settings` (not `new_settings`): the kit's renderer.ts hardcodes
// `invoke(saveCmd, { settings: current })`, so this name is the IPC contract.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    self::settings::persist(&app, &settings)?;
    apply_capture_exclusion(&app, settings.hide_from_capture);
    let state = app.state::<SettingsState>();
    // A silent skip here persists the file but leaves memory stale, so every reader
    // after it (overlay::reconcile especially) acts on the pre-save settings. Recover
    // via into_inner() instead: the mutex stays poisoned forever otherwise, so a
    // skip-on-poison would desync memory from disk for the rest of the process.
    match state.0.lock() {
        Ok(mut s) => *s = settings,
        Err(poisoned) => {
            log::error!("save_settings: settings lock poisoned, recovering and updating state");
            *poisoned.into_inner() = settings;
        }
    }
    // Broadcast (not emit_to "strip"): the flyout window also renders widget_config
    // (e.g. cpu/gpu show_temp), so it needs this too, not just the strip.
    let _ = app.emit("widgets-changed", ());
    overlay::reconcile(&app);
    // Resent unconditionally, same as overlay::reconcile above: cheap, and it
    // covers pomodoro enable/disable and placement changes alike.
    bridge_pomodoro::send_host_overlay(&app);
    Ok(())
}

/// Minimal version info for the kit's About page. No build-date/install-date
/// tracking yet (unlike pomodoro's fuller impl) - kept to what About needs today.
#[derive(serde::Serialize)]
struct VersionInfo {
    version: String,
    build_date: String,
    installed_at: Option<String>,
}

#[tauri::command]
fn get_version_info() -> VersionInfo {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: option_env!("BUILD_DATE").unwrap_or("unknown").to_string(),
        installed_at: None,
    }
}

/// The strip hugs its content: JS reports the row's CSS width after each render.
#[tauri::command]
fn set_strip_width(app: AppHandle, width_css: f64) -> Result<(), String> {
    taskbar::position_strip(&app, width_css.max(1.0)).map_err(|e| e.to_string())
}

/// Webview consoles aren't visible in supervised dev runs; both pages forward
/// window.onerror / unhandled rejections here.
#[tauri::command]
fn log_js(level: String, msg: String) {
    match level.as_str() {
        "error" => log::error!("js: {msg}"),
        _ => log::info!("js: {msg}"),
    }
}

// The settings window is excluded on purpose: it's not part of the always-visible
// surface, and excluding it would hide settings from the user's own screenshots.
// Enumerated, not a fixed list: overlay windows are created at runtime and would
// otherwise leak into a capture with hide_from_capture on.
fn apply_capture_exclusion(app: &AppHandle, excluded: bool) {
    for (label, w) in app.webview_windows() {
        if label == "strip" || label == "flyout" || label.starts_with(overlay::LABEL_PREFIX) {
            let _ = tauri_kit_window::exclude_from_capture(&w, excluded);
        }
    }
}

fn toggle_strip(app: &AppHandle) {
    let Some(w) = app.get_webview_window("strip") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        flyout::close_flyout(app.clone());
        autohide::set_user_hidden(true);
        autohide::set_user_forced_visible(false);
    } else {
        let _ = w.show();
        autohide::set_user_hidden(false);
        autohide::set_user_forced_visible(true);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Open settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &quit])?;
    let icon: Image = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => Image::from_bytes(include_bytes!("../icons/32x32.png"))?,
    };
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Widgets")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => tile_menu::open_settings(app.clone(), None),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                toggle_strip(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    let context = tauri::generate_context!();
    // Loaded before the Builder exists: config-declared windows start webview JS
    // inside Builder::build(), before even the first line of setup() runs.
    let settings_path = settings::resolve_path(&context.config().identifier)
        .expect("resolve settings path");
    let settings = settings::load(&settings_path);

    tauri::Builder::default()
        // Managed with the real, already-loaded settings so SettingsState is never
        // observably Settings::default() to any command, including ones fired by a
        // webview that starts executing before setup() runs.
        .manage(SettingsState(Mutex::new(settings)))
        .manage(system_stats::StatsState(Mutex::new(Default::default())))
        .manage(bridge_pomodoro::new_state())
        .manage(overlay::new_state())
        .manage(strip::new_state())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_kit_updater::plugin())
        // Backs the About page's "Relaunch now" button after an update installs.
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("strip") {
                let _ = w.show();
                // Without this the flag stays true after a tray-hide + relaunch, and the
                // autohide poller skips every tick for the rest of the session.
                autohide::set_user_hidden(false);
            }
        }))
        // kit_copy_logs reads <log-dir>/app.log. Capped at Info: the wmi crate TRACE-logs
        // every thermal query, which with KeepAll rotation would grow logs unbounded.
        // `targets` REPLACES Builder::new()'s defaults; `target` appends to them, which is
        // what wrote every line twice into both app.log and the productName-derived file.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("app".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepAll)
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            log::info!("app started; version={}", env!("CARGO_PKG_VERSION"));
            let hide_from_capture = handle
                .state::<SettingsState>()
                .0
                .lock()
                .map(|s| s.hide_from_capture)
                .unwrap_or_else(|poisoned| poisoned.into_inner().hide_from_capture);
            apply_capture_exclusion(&handle, hide_from_capture);

            let _ = taskbar::position_strip(&handle, 320.0);
            if let Some(win) = handle.get_webview_window("strip") {
                let _ = win.show();
            }
            overlay::reconcile(&handle);
            system_stats::spawn_poller(handle.clone());
            autohide::spawn_poller(handle.clone());
            bridge_conductor::spawn(handle.clone());
            bridge_pomodoro::spawn(handle.clone());
            spotify_smtc::spawn(handle.clone());
            if let Err(e) = build_tray(&handle) {
                eprintln!("failed to build tray: {e}");
            }
            Ok(())
        })
        // Separate registration from the tray's on_menu_event: both land in the
        // same global listener list (tauri fires every listener for every menu
        // event), so each handler namespaces its ids and ignores the rest.
        .on_menu_event(tile_menu::handle_menu_event)
        // Settings is a real window (taskbar-visible), so closing it via the X
        // must hide, not destroy, or the app would need a full window rebuild to reopen.
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_version_info,
            set_strip_width,
            log_js,
            autostart::get_autostart,
            autostart::set_autostart,
            flyout::open_flyout,
            flyout::close_flyout,
            flyout::get_current_flyout_widget,
            overlay::overlay_widget_id,
            overlay::save_overlay_geometry,
            overlay::monitor_at_point,
            system_stats::get_system_stats,
            system_stats::get_top_processes,
            conductor_data::get_conductor_usage,
            bridge_conductor::conductor_refresh_now,
            bridge_pomodoro::pomodoro_cmd,
            tile_menu::show_tile_menu,
            tile_menu::open_settings,
            tile_menu::open_task_manager,
            tile_menu::open_explorer,
            tile_menu::open_spotify,
            tile_menu::focus_or_launch_app,
            tile_menu::reorder_widgets,
            taskbar::list_taskbar_monitors,
            spotify_smtc::get_spotify_now_playing,
            spotify_smtc::spotify_toggle_play_pause,
            spotify_smtc::spotify_next,
            spotify_smtc::spotify_previous,
            spotify_smtc::spotify_seek,
            spotify_smtc::spotify_toggle_shuffle,
            spotify_smtc::spotify_toggle_repeat,
            tauri_kit_settings::kit_copy_logs,
            tauri_kit_settings::kit_reset_settings,
        ])
        .run(context)
        .expect("error while running tauri application");
}
