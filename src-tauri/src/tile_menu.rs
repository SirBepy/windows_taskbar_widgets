use crate::settings::{self, SettingsState};
use serde::{Deserialize, Serialize};
use tauri::menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Window};

#[derive(Deserialize)]
pub struct MenuItemSpec {
    id: String,
    label: String,
}

#[derive(Clone, Serialize)]
struct TileMenuAction {
    widget_id: String,
    item_id: String,
}

fn mk_item(app: &AppHandle, id: String, label: &str) -> Result<MenuItem<tauri::Wry>, String> {
    MenuItem::with_id(app, id, label, true, None::<&str>).map_err(|e| e.to_string())
}

/// Shared by the tray menu and the tile menu's "App options" submenu so the
/// two item lists can't drift apart; ids stay "dashboard"/"quit" either way.
pub fn app_menu_items(
    app: &AppHandle,
) -> tauri::Result<(MenuItem<tauri::Wry>, MenuItem<tauri::Wry>)> {
    let dashboard = MenuItem::with_id(app, "dashboard", "Dashboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Ok((dashboard, quit))
}

#[tauri::command]
pub fn show_tile_menu(
    app: AppHandle,
    window: Window,
    widget_id: String,
    items: Vec<MenuItemSpec>,
) -> Result<(), String> {
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("edit:{widget_id}"), "Edit this widget")?)
        .map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("hide:{widget_id}"), "Hide this widget")?)
        .map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("move-left:{widget_id}"), "Move left")?)
        .map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("move-right:{widget_id}"), "Move right")?)
        .map_err(|e| e.to_string())?;
    if !items.is_empty() {
        menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for it in &items {
            let id = format!("action:{widget_id}:{}", it.id);
            menu.append(&mk_item(&app, id, &it.label)?).map_err(|e| e.to_string())?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let (dashboard, quit) = app_menu_items(&app).map_err(|e| e.to_string())?;
    let app_options = Submenu::with_items(&app, "App options", true, &[&dashboard, &quit])
        .map_err(|e| e.to_string())?;
    menu.append(&app_options).map_err(|e| e.to_string())?;
    menu.popup(window).map_err(|e| e.to_string())
}

// Hide moves the id into hidden_widgets rather than dropping it, so re-enabling
// from the dashboard doesn't need to guess at where a widget used to sit.
fn hide_widget(app: &AppHandle, widget_id: &str) {
    let state = app.state::<SettingsState>();
    let Ok(mut s) = state.0.lock() else { return };
    s.enabled_widgets.retain(|w| w != widget_id);
    if !s.hidden_widgets.iter().any(|w| w == widget_id) {
        s.hidden_widgets.push(widget_id.to_string());
    }
    let _ = settings::persist(app, &s);
    let _ = app.emit_to("strip", "widgets-changed", ());
}

fn move_widget(app: &AppHandle, widget_id: &str, dir: i32) {
    let state = app.state::<SettingsState>();
    let Ok(mut s) = state.0.lock() else { return };
    if let Some(i) = s.enabled_widgets.iter().position(|w| w == widget_id) {
        let j = i as i32 + dir;
        if j >= 0 && (j as usize) < s.enabled_widgets.len() {
            s.enabled_widgets.swap(i, j as usize);
        }
    }
    let _ = settings::persist(app, &s);
    let _ = app.emit_to("strip", "widgets-changed", ());
}

/// Global menu-event listeners fire for every Menu on the app (tray + popups
/// share one list), so ids are namespaced and anything unrecognized (e.g. tray's
/// "quit") is ignored here rather than colliding with the tray's own handler.
pub fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(wid) = id.strip_prefix("edit:") {
        open_dashboard(app.clone(), Some(wid.to_string()));
    } else if let Some(wid) = id.strip_prefix("hide:") {
        hide_widget(app, wid);
    } else if let Some(wid) = id.strip_prefix("move-left:") {
        move_widget(app, wid, -1);
    } else if let Some(wid) = id.strip_prefix("move-right:") {
        move_widget(app, wid, 1);
    } else if let Some(rest) = id.strip_prefix("action:") {
        if let Some((wid, item_id)) = rest.split_once(':') {
            let _ = app.emit_to(
                "strip",
                "tile-menu-action",
                TileMenuAction { widget_id: wid.to_string(), item_id: item_id.to_string() },
            );
        }
    }
}

#[derive(Clone, Serialize)]
struct DashboardNavigate {
    section: Option<String>,
}

/// Shows + focuses the dashboard window and tells it which section to expand.
#[tauri::command]
pub fn open_dashboard(app: AppHandle, section: Option<String>) {
    let Some(win) = app.get_webview_window("dashboard") else { return };
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit_to("dashboard", "dashboard-navigate", DashboardNavigate { section });
}

#[cfg(target_os = "windows")]
fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(target_os = "windows"))]
fn detach(_cmd: &mut std::process::Command) {}

#[tauri::command]
pub fn open_task_manager() -> Result<(), String> {
    let mut cmd = std::process::Command::new("taskmgr.exe");
    detach(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

// Drive-root only (e.g. "C:" or "C:\\"): blocks passing arbitrary paths/args to explorer.exe.
fn is_drive_root(path: &str) -> bool {
    let b = path.as_bytes();
    matches!(b, [d, b':'] | [d, b':', b'\\'] if d.is_ascii_alphabetic())
}

#[tauri::command]
pub fn open_explorer(path: String) -> Result<(), String> {
    if !is_drive_root(&path) {
        return Err(format!("rejected non-drive-root path: {path}"));
    }
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg(&path);
    detach(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reorder_widgets(app: AppHandle, order: Vec<String>) -> Result<(), String> {
    let state = app.state::<SettingsState>();
    let mut s = state.0.lock().map_err(|_| "settings lock poisoned".to_string())?;
    let mut new_order = order;
    for id in s.enabled_widgets.iter() {
        if !new_order.contains(id) {
            new_order.push(id.clone());
        }
    }
    s.enabled_widgets = new_order;
    settings::persist(&app, &s)
}
