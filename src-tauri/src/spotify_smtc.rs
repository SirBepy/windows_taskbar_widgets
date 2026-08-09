//! Spotify now-playing via Windows SMTC (GlobalSystemMediaTransportControlsSessionManager),
//! a general "now playing" surface every media app registers with - filtered here to the
//! one session whose AUMID contains "spotify" so a browser tab never gets picked up instead.
//! Polling loop, patterned after bridge_conductor.rs's spawn/run shape.
use base64::Engine;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Media::MediaPlaybackAutoRepeatMode as RepeatMode;
use windows::Storage::Streams::DataReader;

const POLL_MS: u64 = 1_000;

#[derive(Clone, Serialize)]
pub struct SpotifyNowPlaying {
    title: String,
    artist: String,
    album: String,
    playing: bool,
    position_ms: i64,
    duration_ms: i64,
    shuffle: bool,
    repeat: String,
    art_data_uri: Option<String>,
}

// windows-rs's WinRT types (Session, MediaProperties, ...) aren't Send, so they can't
// cross a tauri::async_runtime::spawn/#[tauri::command] boundary (both need Send futures).
// Real WinRT work runs on its own OS thread via block_on_local; only plain Send data
// (Option<SpotifyNowPlaying>, Result<(), String>) crosses back out.
fn block_on_local<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("spotify_smtc: failed to build a current-thread tokio runtime")
        .block_on(fut)
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || block_on_local(run(app)));
}

async fn run(app: AppHandle) {
    loop {
        let payload = fetch_now_playing().await;
        let _ = app.emit("spotify-now-playing", payload);
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// One-shot version of the poll loop's fetch, for the frontend's initial paint.
#[tauri::command]
pub async fn get_spotify_now_playing() -> Option<SpotifyNowPlaying> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(fetch_now_playing()))
        .await
        .unwrap_or(None)
}

async fn fetch_now_playing() -> Option<SpotifyNowPlaying> {
    let session = current_spotify_session().await?;
    build_snapshot(&session).await
}

// GetSessions returns an IVectorView; walked via GetAt/Size rather than relying on
// IntoIterator support, which isn't guaranteed across windows-rs versions.
async fn current_spotify_session() -> Option<Session> {
    let manager = SessionManager::RequestAsync().ok()?.await.ok()?;
    let sessions = manager.GetSessions().ok()?;
    let count = sessions.Size().ok()?;
    for i in 0..count {
        let Ok(session) = sessions.GetAt(i) else { continue };
        let Ok(aumid) = session.SourceAppUserModelId() else { continue };
        if aumid.to_string().to_lowercase().contains("spotify") {
            return Some(session);
        }
    }
    None
}

async fn build_snapshot(session: &Session) -> Option<SpotifyNowPlaying> {
    let info = session.GetPlaybackInfo().ok()?;
    let status = info.PlaybackStatus().unwrap_or(PlaybackStatus::Closed);
    let shuffle = info.IsShuffleActive().and_then(|r| r.Value()).unwrap_or(false);
    let repeat = match info.AutoRepeatMode().and_then(|r| r.Value()) {
        Ok(RepeatMode::Track) => "track",
        Ok(RepeatMode::List) => "list",
        _ => "off",
    }
    .to_string();

    let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
    let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
    let art_data_uri = fetch_thumbnail(&props).await;

    let timeline = session.GetTimelineProperties().ok();
    // WinRT TimeSpan.Duration is 100ns ticks.
    let position_ms = timeline.as_ref().and_then(|t| t.Position().ok()).map(|d| d.Duration / 10_000).unwrap_or(0);
    let duration_ms = timeline.as_ref().and_then(|t| t.EndTime().ok()).map(|d| d.Duration / 10_000).unwrap_or(0);

    Some(SpotifyNowPlaying {
        title,
        artist,
        album,
        playing: status == PlaybackStatus::Playing,
        position_ms,
        duration_ms,
        shuffle,
        repeat,
        art_data_uri,
    })
}

async fn fetch_thumbnail(props: &MediaProperties) -> Option<String> {
    let thumb_ref = props.Thumbnail().ok()?;
    let stream = thumb_ref.OpenReadAsync().ok()?.await.ok()?;
    let mime = stream.ContentType().ok()?.to_string();
    let size = u32::try_from(stream.Size().ok()?).ok()?;
    if size == 0 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.await.ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Some(format!("data:{mime};base64,{b64}"))
}

fn werr(e: windows::core::Error) -> String {
    e.to_string()
}

// Each _impl re-resolves the current session rather than caching one: the SMTC-reported
// session can change between polls (track/app switch). Commands are non-fatal,
// fire-and-forget calls from the frontend (see conductor_refresh_now).

async fn toggle_play_pause_impl() -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    let playing =
        session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map_err(werr)? == PlaybackStatus::Playing;
    let op = if playing { session.TryPauseAsync() } else { session.TryPlayAsync() }.map_err(werr)?;
    op.await.map_err(werr)?;
    Ok(())
}

async fn next_impl() -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    session.TrySkipNextAsync().map_err(werr)?.await.map_err(werr)?;
    Ok(())
}

async fn previous_impl() -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    session.TrySkipPreviousAsync().map_err(werr)?.await.map_err(werr)?;
    Ok(())
}

async fn seek_impl(position_ms: i64) -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    session
        .TryChangePlaybackPositionAsync(position_ms * 10_000)
        .map_err(werr)?
        .await
        .map_err(werr)?;
    Ok(())
}

async fn toggle_shuffle_impl() -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    let current = session
        .GetPlaybackInfo()
        .map_err(werr)?
        .IsShuffleActive()
        .and_then(|r| r.Value())
        .unwrap_or(false);
    session.TryChangeShuffleActiveAsync(!current).map_err(werr)?.await.map_err(werr)?;
    Ok(())
}

async fn toggle_repeat_impl() -> Result<(), String> {
    let session = current_spotify_session().await.ok_or_else(|| "no spotify session".to_string())?;
    let current = session
        .GetPlaybackInfo()
        .map_err(werr)?
        .AutoRepeatMode()
        .and_then(|r| r.Value())
        .unwrap_or(RepeatMode::None);
    let next = match current {
        RepeatMode::None => RepeatMode::List,
        RepeatMode::List => RepeatMode::Track,
        _ => RepeatMode::None,
    };
    session.TryChangeAutoRepeatModeAsync(next).map_err(werr)?.await.map_err(werr)?;
    Ok(())
}

// Thin Send-safe shells: spawn_blocking's closure captures no windows-rs types (it
// re-resolves the session inside block_on_local), so only the plain Result<(), String>
// crosses back over the join handle.

#[tauri::command]
pub async fn spotify_toggle_play_pause() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(toggle_play_pause_impl()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spotify_next() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(next_impl())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spotify_previous() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(previous_impl())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spotify_seek(position_ms: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || block_on_local(seek_impl(position_ms)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spotify_toggle_shuffle() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(toggle_shuffle_impl()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn spotify_toggle_repeat() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| block_on_local(toggle_repeat_impl()))
        .await
        .map_err(|e| e.to_string())?
}
