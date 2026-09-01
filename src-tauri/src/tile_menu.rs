use crate::settings::{self, SettingsState, DIVIDER_PREFIX};
use crate::strip;
use crate::tile_actions::{apply_hide, apply_lanes, apply_move, apply_remove_divider, widget_kind_for};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
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

// Dividers have no config to edit and are removed outright rather than parked in
// hidden_widgets. A divider's instance id IS its own DIVIDER_PREFIX id (see
// MonitorWidgets::next_instance_id), so this check works on either.
fn is_divider(id: &str) -> bool {
    id.starts_with(DIVIDER_PREFIX)
}

#[tauri::command]
pub fn show_tile_menu(
    app: AppHandle,
    window: Window,
    instance_id: String,
    items: Vec<MenuItemSpec>,
) -> Result<(), String> {
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;
    if is_divider(&instance_id) {
        menu.append(&mk_item(&app, format!("remove-divider:{instance_id}"), "Remove divider")?)
            .map_err(|e| e.to_string())?;
    } else {
        let widget_id = {
            let state = app.state::<SettingsState>();
            state.0.lock().map(|s| widget_kind_for(&s, &instance_id)).unwrap_or_else(|_| instance_id.clone())
        };
        menu.append(&mk_item(&app, format!("edit:{widget_id}"), "Edit this widget")?)
            .map_err(|e| e.to_string())?;
        menu.append(&mk_item(&app, format!("hide:{instance_id}"), "Hide this widget")?)
            .map_err(|e| e.to_string())?;
    }
    menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("move-left:{instance_id}"), "Move left")?)
        .map_err(|e| e.to_string())?;
    menu.append(&mk_item(&app, format!("move-right:{instance_id}"), "Move right")?)
        .map_err(|e| e.to_string())?;
    if !items.is_empty() {
        menu.append(&PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for it in &items {
            let id = format!("action:{instance_id}:{}", it.id);
            menu.append(&mk_item(&app, id, &it.label)?).map_err(|e| e.to_string())?;
        }
    }
    menu.popup(window).map_err(|e| e.to_string())
}

fn hide_widget(app: &AppHandle, instance_id: &str) {
    let target = {
        let state = app.state::<SettingsState>();
        let Ok(mut s) = state.0.lock() else { return };
        let monitor = apply_hide(&mut s, instance_id);
        let _ = settings::persist(app, &mut s);
        strip::label_for_monitor(app, &monitor.unwrap_or_default())
    };
    let _ = app.emit_to(target.as_str(), "widgets-changed", ());
    // Hiding an overlay-placed widget is the only way to close its window from
    // its own context menu, so this path has to reconcile too.
    crate::overlay::reconcile(app);
}

fn remove_divider(app: &AppHandle, instance_id: &str) {
    let target = {
        let state = app.state::<SettingsState>();
        let Ok(mut s) = state.0.lock() else { return };
        let monitor = apply_remove_divider(&mut s, instance_id);
        let _ = settings::persist(app, &mut s);
        strip::label_for_monitor(app, &monitor.unwrap_or_default())
    };
    let _ = app.emit_to(target.as_str(), "widgets-changed", ());
}

fn move_widget(app: &AppHandle, instance_id: &str, dir: i32) {
    let target = {
        let state = app.state::<SettingsState>();
        let Ok(mut s) = state.0.lock() else { return };
        let monitor = apply_move(&mut s, instance_id, dir);
        let _ = settings::persist(app, &mut s);
        strip::label_for_monitor(app, &monitor.unwrap_or_default())
    };
    let _ = app.emit_to(target.as_str(), "widgets-changed", ());
}

fn emit_tile_menu_action(app: &AppHandle, instance_id: &str, item_id: &str) {
    let state = app.state::<SettingsState>();
    let Ok(s) = state.0.lock() else { return };
    let Some(widget_id) = s.monitor_widgets.all().find(|si| si.instance_id == instance_id).map(|si| si.widget_id.clone())
    else {
        return;
    };
    let monitor = s.monitor_widgets.monitor_of(instance_id).unwrap_or("").to_string();
    drop(s);
    let target = strip::label_for_monitor(app, &monitor);
    let _ = app.emit_to(target.as_str(), "tile-menu-action", TileMenuAction { widget_id, item_id: item_id.to_string() });
}

/// Global menu-event listeners fire for every Menu on the app (tray + popups
/// share one list), so ids are namespaced and anything unrecognized (e.g. tray's
/// "quit") is ignored here rather than colliding with the tray's own handler.
pub fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(wid) = id.strip_prefix("edit:") {
        open_settings(app.clone(), Some(wid.to_string()));
    } else if let Some(iid) = id.strip_prefix("hide:") {
        hide_widget(app, iid);
    } else if let Some(iid) = id.strip_prefix("remove-divider:") {
        remove_divider(app, iid);
    } else if let Some(iid) = id.strip_prefix("move-left:") {
        move_widget(app, iid, -1);
    } else if let Some(iid) = id.strip_prefix("move-right:") {
        move_widget(app, iid, 1);
    } else if let Some(rest) = id.strip_prefix("action:") {
        if let Some((iid, item_id)) = rest.split_once(':') {
            emit_tile_menu_action(app, iid, item_id);
        }
    }
}

#[derive(Clone, Serialize)]
struct SettingsNavigate {
    section: Option<String>,
}

/// Shows + focuses the settings window and tells it which section to expand.
#[tauri::command]
pub fn open_settings(app: AppHandle, section: Option<String>) {
    let Some(win) = app.get_webview_window("settings") else { return };
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit_to("settings", "settings-navigate", SettingsNavigate { section });
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
pub fn open_spotify() -> Result<(), String> {
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg("spotify:");
    detach(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn focus_or_launch_app(app: String) -> Result<(), String> {
    focus_or_launch_app_impl(&app)
}

// key -> (LOCALAPPDATA-relative exe path, process name to match if already running).
// Hardcoded allowlist: the frontend passes a key, never a path, so it can't ask
// this command to launch anything else.
#[cfg(target_os = "windows")]
const APP_TARGETS: &[(&str, &str, &str)] = &[
    ("conductor", r"Claude Conductor\claude-conductor.exe", "claude-conductor.exe"),
    ("pomodoro", r"Pomodoro Overlay\pomodoro-overlay.exe", "pomodoro-overlay.exe"),
];

#[cfg(target_os = "windows")]
fn focus_or_launch_app_impl(key: &str) -> Result<(), String> {
    let Some(&(_, rel_path, proc_name)) = APP_TARGETS.iter().find(|(k, _, _)| *k == key) else {
        return Err(format!("unknown app key: {key}"));
    };
    if let Some(pid) = find_pid_by_name(proc_name) {
        if focus_window_for_pid(pid) {
            return Ok(());
        }
    }
    let local_appdata = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA not set")?;
    let exe_path = std::path::Path::new(&local_appdata).join(rel_path);
    if !exe_path.exists() {
        return Err(format!("{proc_name} not found at {}", exe_path.display()));
    }
    let mut cmd = std::process::Command::new(&exe_path);
    detach(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(target_os = "windows"))]
fn focus_or_launch_app_impl(_key: &str) -> Result<(), String> {
    Err("focus_or_launch_app is Windows-only".to_string())
}

#[cfg(target_os = "windows")]
fn find_pid_by_name(proc_name: &str) -> Option<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .find(|p| p.name().to_string_lossy().eq_ignore_ascii_case(proc_name))
        .map(|p| p.pid().as_u32())
}

// EnumWindows' callback is extern "system" and can't capture state, so the target
// pid and the found hwnd travel through this struct via the raw lparam pointer.
#[cfg(target_os = "windows")]
struct FindWindowCtx {
    pid: u32,
    hwnd: isize,
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};
    let ctx = &mut *(lparam as *mut FindWindowCtx);
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == ctx.pid {
        ctx.hwnd = hwnd as isize;
        return 0;
    }
    1
}

/// Restores + foregrounds the target process's first visible top-level window.
/// Returns false (caller then spawns a fresh instance) when none is found yet.
#[cfg(target_os = "windows")]
fn focus_window_for_pid(pid: u32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let mut ctx = FindWindowCtx { pid, hwnd: 0 };
    unsafe {
        EnumWindows(Some(enum_windows_proc), &mut ctx as *mut _ as isize);
        if ctx.hwnd == 0 {
            return false;
        }
        let hwnd = ctx.hwnd as windows_sys::Win32::Foundation::HWND;
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
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
    settings::persist(&app, &mut s)
}

/// The settings lanes UI's one write path. Takes every live monitor's whole lane in one
/// call, not one lane per call, so a tile dragged from one monitor to another is never
/// briefly on both or on neither.
#[tauri::command]
pub fn set_lanes(app: AppHandle, lanes: HashMap<String, Vec<String>>) -> Result<(), String> {
    {
        let state = app.state::<SettingsState>();
        let mut s = state.0.lock().map_err(|_| "settings lock poisoned".to_string())?;
        let previously_enabled = s.enabled_widgets.clone();
        apply_lanes(&mut s, &lanes);
        // Same pairing save_settings uses: a kind dragged back in has to lose BOTH
        // hidden_widgets shapes, or an orphaned "<kind>#n" keeps it invisible.
        s.clear_hidden_for_reenabled_widgets(&previously_enabled);
        settings::persist(&app, &mut s)?;
    }
    let _ = app.emit("widgets-changed", ());
    crate::overlay::reconcile(&app);
    strip::reconcile(&app);
    Ok(())
}
