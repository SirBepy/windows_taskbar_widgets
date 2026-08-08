//! TCP client for pomodoro-overlay's newline-delimited JSON widget bridge
//! (see that repo's src-tauri/src/bridge.rs). std::thread + std::net, not
//! tokio: the protocol is a plain line-based loopback socket.
use crate::settings::SettingsState;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const DISCOVERY_FILENAME: &str = "widget-bridge.json";
const RECONNECT_MIN_SECS: u64 = 3;
const RECONNECT_MAX_SECS: u64 = 5;

#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}

fn discovery_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|d| PathBuf::from(d).join("com.sirbepy.pomodoro-overlay").join(DISCOVERY_FILENAME))
}

/// Re-read on every reconnect attempt: the overlay mints a fresh OS-assigned
/// port (and discovery file) on each of its own restarts.
fn read_discovery() -> Option<Discovery> {
    let raw = std::fs::read_to_string(discovery_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Holds the writer-thread's channel while connected so `pomodoro_cmd` can
/// send without touching the socket directly; `None` while disconnected.
pub struct PomodoroBridgeState(pub Mutex<Option<Sender<String>>>);

pub fn new_state() -> PomodoroBridgeState {
    PomodoroBridgeState(Mutex::new(None))
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || run(app));
}

fn run(app: AppHandle) {
    let mut backoff = RECONNECT_MIN_SECS;
    loop {
        if try_connect(&app).is_ok() {
            backoff = RECONNECT_MIN_SECS;
        } else {
            backoff = (backoff + 1).min(RECONNECT_MAX_SECS);
        }
        set_writer(&app, None);
        let _ = app.emit("pomodoro-state", json!({ "connected": false }));
        std::thread::sleep(Duration::from_secs(backoff));
    }
}

fn set_writer(app: &AppHandle, tx: Option<Sender<String>>) {
    if let Some(state) = app.try_state::<PomodoroBridgeState>() {
        if let Ok(mut cur) = state.0.lock() {
            *cur = tx;
        }
    }
}

/// Enabled anywhere in this app, taskbar or overlay alike (design doc decision 3).
fn pomodoro_hosted(app: &AppHandle) -> bool {
    app.try_state::<SettingsState>()
        .and_then(|s| s.0.lock().ok().map(|g| g.is_active("pomodoro")))
        .unwrap_or(false)
}

/// No-op while disconnected: a fresh connect resends it, so nothing is lost.
pub fn send_host_overlay(app: &AppHandle) {
    // Resolved before the bridge lock is taken: holding both at once is the one
    // lock order no other path here uses, and std Mutex gives no re-entrancy.
    let suppress_compact = pomodoro_hosted(app);
    let Some(state) = app.try_state::<PomodoroBridgeState>() else { return };
    let Ok(guard) = state.0.lock() else { return };
    let Some(tx) = guard.as_ref() else { return };
    // suppress_fullscreen stays false: this app never draws a fullscreen
    // overlay, so pomodoro-overlay's big view is never this app's to hide.
    let line = json!({
        "cmd": "host_overlay",
        "suppress_compact": suppress_compact,
        "suppress_fullscreen": false,
    })
    .to_string();
    let _ = tx.send(line);
}

/// Connects, authenticates, and forwards state lines until the socket drops.
/// Returns Err on any failure so `run`'s caller backs off and retries.
fn try_connect(app: &AppHandle) -> Result<(), ()> {
    let disc = read_discovery().ok_or(())?;
    let stream = TcpStream::connect(("127.0.0.1", disc.port)).map_err(|_| ())?;
    let mut writer = stream.try_clone().map_err(|_| ())?;
    writer
        .write_all(format!("{}\n", json!({ "auth": disc.token })).as_bytes())
        .map_err(|_| ())?;

    let (tx, rx) = channel::<String>();
    set_writer(app, Some(tx));
    send_host_overlay(app);
    let write_handle = std::thread::spawn(move || {
        for line in rx {
            if writer.write_all(format!("{line}\n").as_bytes()).is_err() {
                break;
            }
        }
    });

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(obj) = v.as_object_mut() {
            obj.insert("connected".into(), json!(true));
        }
        let _ = app.emit("pomodoro-state", v);
    }

    // Dropping our Sender (via set_writer(None) in `run`) closes the channel,
    // which unblocks the writer thread's `for line in rx` so this join returns.
    set_writer(app, None);
    let _ = write_handle.join();
    Err(())
}

/// Writes the matching command line; errors (incl. "not connected") are
/// surfaced to the caller so the flyout can no-op instead of throwing.
#[tauri::command]
pub fn pomodoro_cmd(
    state: tauri::State<PomodoroBridgeState>,
    action: String,
    phase: Option<String>,
) -> Result<(), String> {
    let line = match phase {
        Some(p) => json!({ "cmd": action, "phase": p }).to_string(),
        None => json!({ "cmd": action }).to_string(),
    };
    let guard = state.0.lock().map_err(|_| "bridge state poisoned".to_string())?;
    let tx = guard.as_ref().ok_or("pomodoro overlay not connected")?;
    tx.send(line).map_err(|_| "pomodoro overlay not connected".to_string())
}
