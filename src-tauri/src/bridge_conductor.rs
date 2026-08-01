//! Live-push bridge into Claude Conductor's daemon: WS usage snapshots plus
//! an on-demand refresh RPC. conductor_data.rs's 30s DB poll stays the
//! fallback; this only makes updates arrive sooner when the daemon is up.
use futures_util::StreamExt;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::Message;

const DAEMON_PORT: u16 = 27183;
const MIN_BACKOFF_SECS: u64 = 30;
const MAX_BACKOFF_SECS: u64 = 60;

fn access_token_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|d| PathBuf::from(d).join("claude-conductor").join("remote-access.json"))
}

/// remote-access.json's schema (per device_registry.rs's `ensure_desktop_device`):
/// `{"hash": <sha256 of token>, "token": <plaintext token>}`. `is_enabled` (the
/// kill switch) lives in a separate file (remote-devices.json) the daemon checks
/// server-side - we never read it ourselves, a disabled/invalid token just fails
/// the connect/RPC attempt below and we back off like any other outage.
fn read_token() -> Option<String> {
    let raw = std::fs::read_to_string(access_token_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("token")?.as_str().map(str::to_string)
}

#[derive(Deserialize)]
struct Frame {
    method: Option<String>,
    params: Option<serde_json::Value>,
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(run(app));
}

/// Reconnect loop, spawned once from lib.rs setup. Re-reads the token file on
/// every attempt (conductor can start/stop/re-pair independently of us).
async fn run(app: AppHandle) {
    let mut backoff = MIN_BACKOFF_SECS;
    loop {
        if try_connect_and_stream(&app).await.is_ok() {
            backoff = MIN_BACKOFF_SECS;
        } else {
            backoff = (backoff + 10).min(MAX_BACKOFF_SECS);
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
    }
}

async fn try_connect_and_stream(app: &AppHandle) -> Result<(), ()> {
    let token = read_token().ok_or(())?;
    let url = format!("ws://127.0.0.1:{DAEMON_PORT}/api/global/stream?token={token}");
    let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|_| ())?;
    let (_write, mut read) = ws.split();
    while let Some(msg) = read.next().await {
        let Ok(Message::Text(txt)) = msg else { continue };
        let Ok(frame) = serde_json::from_str::<Frame>(&txt) else { continue };
        if frame.method.as_deref() == Some("usage_snapshot") {
            if let Some(params) = frame.params {
                let _ = app.emit("conductor-usage-live", params);
            }
        }
    }
    Ok(())
}

/// Click-to-refresh: forwards to the daemon's allowlisted `/api/rpc` method
/// (see remote_handlers.rs's SAFE_METHODS entry + doc comment on
/// `request_live_usage_refresh`). Non-fatal by design - callers fire-and-forget.
#[tauri::command]
pub async fn conductor_refresh_now() -> Result<(), String> {
    let token = read_token().ok_or("conductor not paired")?;
    let url = format!("http://127.0.0.1:{DAEMON_PORT}/api/rpc");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "method": "request_live_usage_refresh" }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("conductor rpc failed: {}", resp.status()))
    }
}
