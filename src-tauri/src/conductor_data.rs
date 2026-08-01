//! Read-only bridge into Claude Conductor's on-disk state
//! (%APPDATA%\claude-conductor): accounts.json + companion.db usage_snapshots.
//! Conductor stays the writer; this never opens the DB writable.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Account {
    id: String,
    label: String,
    colour: String,
    #[allow(dead_code)]
    icon: String,
}

#[derive(Deserialize)]
struct WindowUsage {
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct UsageSnapshot {
    captured_at: String,
    five_hour: WindowUsage,
    seven_day: WindowUsage,
}

#[derive(Clone, Serialize)]
pub struct AccountUsage {
    pub id: String,
    pub label: String,
    pub colour: String,
    pub captured_at: String,
    pub five_hour_pct: f64,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_pct: f64,
    pub seven_day_resets_at: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ConductorUsage {
    pub available: bool,
    pub accounts: Vec<AccountUsage>,
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("claude-conductor"))
}

fn latest_snapshot(
    conn: &rusqlite::Connection,
    account_id: Option<&str>,
) -> Option<UsageSnapshot> {
    let sql = "SELECT data FROM usage_snapshots \
               WHERE (?1 IS NULL AND account_id IS NULL) OR account_id = ?1 \
               ORDER BY timestamp DESC LIMIT 1";
    let data: String = conn
        .query_row(sql, rusqlite::params![account_id], |r| r.get(0))
        .ok()?;
    serde_json::from_str(&data).ok()
}

#[tauri::command]
pub fn get_conductor_usage() -> ConductorUsage {
    let empty = ConductorUsage { available: false, accounts: Vec::new() };
    let Some(dir) = data_dir() else { return empty };
    let db_path = dir.join("companion.db");
    if !db_path.exists() {
        return empty;
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return empty;
    };

    let accounts: Vec<Account> = std::fs::read_to_string(dir.join("accounts.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();

    let mut out = Vec::new();
    if accounts.is_empty() {
        if let Some(snap) = latest_snapshot(&conn, None) {
            out.push(to_usage("legacy", "Claude", "#d97757", snap));
        }
    } else {
        for a in &accounts {
            if let Some(snap) = latest_snapshot(&conn, Some(&a.id)) {
                out.push(to_usage(&a.id, &a.label, &a.colour, snap));
            }
        }
    }
    ConductorUsage { available: true, accounts: out }
}

fn to_usage(id: &str, label: &str, colour: &str, snap: UsageSnapshot) -> AccountUsage {
    let none_if_empty = |s: Option<String>| s.filter(|v| !v.is_empty());
    AccountUsage {
        id: id.to_string(),
        label: label.to_string(),
        colour: colour.to_string(),
        captured_at: snap.captured_at,
        five_hour_pct: snap.five_hour.utilization,
        five_hour_resets_at: none_if_empty(snap.five_hour.resets_at),
        seven_day_pct: snap.seven_day.utilization,
        seven_day_resets_at: none_if_empty(snap.seven_day.resets_at),
    }
}
