use crate::settings::{self, Settings, SettingsState, DIVIDER_PREFIX};
use crate::strip;
use serde::{Deserialize, Serialize};
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

// "Edit this widget" opens the settings strip editor, which is still keyed by
// widget kind (not per-placement), so the menu needs the kind back from the
// instance id it was built for.
fn widget_kind_for(s: &Settings, instance_id: &str) -> String {
    s.monitor_widgets
        .all()
        .find(|si| si.instance_id == instance_id)
        .map(|si| si.widget_id.clone())
        .unwrap_or_else(|| instance_id.to_string())
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

// Stays in monitor_widgets so re-enabling remembers its lane. The legacy widget-id
// keyed lists (still read by overlay::reconcile, poller.rs, bridge_pomodoro.rs) are
// mirrored only once no other visible instance of that kind remains, so hiding one
// placement can never hide a sibling. Returns the instance's monitor, if found.
fn apply_hide(s: &mut Settings, instance_id: &str) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string);
    let widget_id = s.monitor_widgets.all().find(|si| si.instance_id == instance_id).map(|si| si.widget_id.clone());

    if !s.hidden_widgets.iter().any(|w| w == instance_id) {
        s.hidden_widgets.push(instance_id.to_string());
    }
    if let Some(wid) = &widget_id {
        let sibling_visible = s.monitor_widgets.all().any(|si| {
            &si.widget_id == wid && si.instance_id != instance_id && !s.hidden_widgets.iter().any(|h| h == &si.instance_id)
        });
        if !sibling_visible {
            s.enabled_widgets.retain(|w| w != wid);
            if !s.hidden_widgets.iter().any(|w| w == wid) {
                s.hidden_widgets.push(wid.clone());
            }
        }
    }
    monitor
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

// Unlike hide, drops the instance entirely: a divider's uuid id is single-use, so
// there's nothing meaningful to re-offer later, and monitor_widgets shouldn't grow
// dead entries.
fn apply_remove_divider(s: &mut Settings, instance_id: &str) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string);
    if let Some(m) = &monitor {
        if let Some(instances) = s.monitor_widgets.0.get_mut(m) {
            instances.retain(|si| si.instance_id != instance_id);
        }
    }
    s.enabled_widgets.retain(|w| w != instance_id);
    monitor
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

// Reorders within the instance's own monitor lane. The swap is mirrored onto the
// legacy enabled_widgets, which is still what main.ts renders strip order from, so
// move-left/right stays visible until Phase 3 threads instance ids to the frontend.
// Mirroring only ever swaps the two kinds involved, so it cannot reorder a sibling lane.
fn apply_move(s: &mut Settings, instance_id: &str, dir: i32) -> Option<String> {
    let monitor = s.monitor_widgets.monitor_of(instance_id).map(str::to_string)?;
    let mut swapped_kinds: Option<(String, String)> = None;
    if let Some(instances) = s.monitor_widgets.0.get_mut(&monitor) {
        if let Some(i) = instances.iter().position(|si| si.instance_id == instance_id) {
            let j = i as i32 + dir;
            if j >= 0 && (j as usize) < instances.len() {
                let j = j as usize;
                swapped_kinds =
                    Some((instances[i].widget_id.clone(), instances[j].widget_id.clone()));
                instances.swap(i, j);
            }
        }
    }
    if let Some((a, b)) = swapped_kinds {
        let ia = s.enabled_widgets.iter().position(|w| w == &a);
        let ib = s.enabled_widgets.iter().position(|w| w == &b);
        if let (Some(ia), Some(ib)) = (ia, ib) {
            s.enabled_widgets.swap(ia, ib);
        }
    }
    Some(monitor)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{MonitorWidgets, StripInstance};
    use std::collections::HashMap;

    fn instance(id: &str, widget_id: &str) -> StripInstance {
        StripInstance { instance_id: id.to_string(), widget_id: widget_id.to_string() }
    }

    fn two_monitor_settings() -> Settings {
        Settings {
            monitor_widgets: MonitorWidgets(HashMap::from([
                ("".to_string(), vec![instance("cpu#1", "cpu"), instance("ram#1", "ram")]),
                (r"\\.\DISPLAY2".to_string(), vec![instance("cpu#2", "cpu"), instance("gpu#1", "gpu")]),
            ])),
            ..Settings::default()
        }
    }

    #[test]
    fn hide_does_not_hide_sibling_instance_on_another_monitor() {
        let mut s = two_monitor_settings();

        apply_hide(&mut s, "cpu#1");

        assert!(s.hidden_widgets.contains(&"cpu#1".to_string()));
        assert!(!s.hidden_widgets.contains(&"cpu#2".to_string()));
        assert!(s.monitor_widgets.all().any(|si| si.instance_id == "cpu#2"));
    }

    #[test]
    fn hide_only_instance_of_a_kind_also_mirrors_into_legacy_enabled_widgets() {
        let mut s = Settings::default(); // one instance per widget id, all on ""

        apply_hide(&mut s, "cpu#1");

        assert!(!s.enabled_widgets.contains(&"cpu".to_string()));
        assert!(s.hidden_widgets.contains(&"cpu".to_string()));
        assert!(s.hidden_widgets.contains(&"cpu#1".to_string()));
    }

    #[test]
    fn hide_with_a_visible_sibling_leaves_legacy_enabled_widgets_alone() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_hide(&mut s, "cpu#1");

        assert!(s.enabled_widgets.contains(&"cpu".to_string()));
    }

    #[test]
    fn move_reorders_within_one_monitor_lane_and_never_leaks_into_another() {
        let mut s = two_monitor_settings();

        apply_move(&mut s, "ram#1", -1);

        let primary: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(primary, ["ram#1", "cpu#1"]);
        let secondary: Vec<&str> =
            s.monitor_widgets.instances_for(r"\\.\DISPLAY2").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(secondary, ["cpu#2", "gpu#1"]);
    }

    #[test]
    fn move_past_the_lane_boundary_is_a_no_op() {
        let mut s = two_monitor_settings();

        apply_move(&mut s, "cpu#1", -1);

        let primary: Vec<&str> = s.monitor_widgets.instances_for("").iter().map(|si| si.instance_id.as_str()).collect();
        assert_eq!(primary, ["cpu#1", "ram#1"]);
    }

    #[test]
    fn move_mirrors_the_swap_into_legacy_enabled_widgets() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];

        apply_move(&mut s, "ram#1", -1);

        assert_eq!(s.enabled_widgets, vec!["ram", "cpu", "gpu"]);
    }

    #[test]
    fn move_mirror_leaves_kinds_outside_the_swap_in_place() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["gpu".to_string(), "cpu".to_string(), "ram".to_string()];

        apply_move(&mut s, "ram#1", -1);

        assert_eq!(s.enabled_widgets[0], "gpu");
    }

    #[test]
    fn move_past_the_lane_boundary_leaves_legacy_enabled_widgets_alone() {
        let mut s = two_monitor_settings();
        s.enabled_widgets = vec!["cpu".to_string(), "ram".to_string(), "gpu".to_string()];
        let before = s.enabled_widgets.clone();

        apply_move(&mut s, "cpu#1", -1);

        assert_eq!(s.enabled_widgets, before);
    }

    #[test]
    fn remove_divider_drops_the_instance_and_the_legacy_entry() {
        let mut s = Settings {
            enabled_widgets: vec!["cpu".to_string(), "divider:abc".to_string(), "ram".to_string()],
            monitor_widgets: MonitorWidgets(HashMap::from([(
                "".to_string(),
                vec![instance("cpu#1", "cpu"), instance("divider:abc", "divider:abc"), instance("ram#1", "ram")],
            )])),
            ..Settings::default()
        };

        apply_remove_divider(&mut s, "divider:abc");

        assert!(!s.enabled_widgets.contains(&"divider:abc".to_string()));
        assert!(!s.monitor_widgets.all().any(|si| si.instance_id == "divider:abc"));
    }

    #[test]
    fn widget_kind_for_resolves_the_owning_widget_id() {
        let s = two_monitor_settings();
        assert_eq!(widget_kind_for(&s, "gpu#1"), "gpu");
    }
}
