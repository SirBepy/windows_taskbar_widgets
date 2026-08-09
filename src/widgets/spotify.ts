import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { ConfigField, isDragging, subscribeSettings, TaskbarWidget, widgetConfig } from "../shared/widget";

export interface SpotifyNowPlaying {
  title: string;
  artist: string;
  album: string;
  playing: boolean;
  position_ms: number;
  duration_ms: number;
  shuffle: boolean;
  repeat: "off" | "track" | "list";
  art_data_uri: string | null;
}

const FLYOUT_WIDTH = 260;
const FLYOUT_HEIGHT = 300;

// Survives mount/unmount (flyout remounted on every hover-open), so a re-hover paints
// the real track instead of flashing "Nothing playing" while the invoke round-trips.
let lastNowPlaying: SpotifyNowPlaying | null = null;

/** Last known snapshot synchronously, then live updates via the poller's event. */
export function subscribeNowPlaying(onData: (np: SpotifyNowPlaying | null) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  if (lastNowPlaying !== null) onData(lastNowPlaying);
  else
    invoke<SpotifyNowPlaying | null>("get_spotify_now_playing").then((np) => {
      lastNowPlaying = np;
      if (!disposed) onData(np);
    }).catch(() => {});
  listen<SpotifyNowPlaying | null>("spotify-now-playing", (e) => {
    lastNowPlaying = e.payload;
    onData(e.payload);
  }).then((un) => {
    if (disposed) un();
    else unlisten = un;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

const readShowArt = (cfg: Record<string, unknown>): boolean =>
  (cfg.show_album_art as boolean | undefined) ?? true;

const readShowStatusIcon = (cfg: Record<string, unknown>): boolean =>
  (cfg.show_status_icon as boolean | undefined) ?? true;

function artTemplate(np: SpotifyNowPlaying | null, showArt: boolean, cls: string, iconSize: string) {
  const uri = showArt ? np?.art_data_uri : null;
  return html`
    <span class="${cls}${uri ? "" : ` ${cls}-empty`}">
      ${uri ? html`<img src=${uri} alt="" />` : html`<i class="ph ph-music-notes" style="font-size:${iconSize}"></i>`}
    </span>
  `;
}

// ---------- tile ----------

function tileTemplate(np: SpotifyNowPlaying | null, showArt: boolean, showStatus: boolean) {
  const title = np?.title || "Nothing playing";
  return html`
    <span class="spotify-tile">
      ${artTemplate(np, showArt, "spotify-art", "11px")}
      <span class="spotify-tile-text"><span class="spotify-tile-text-inner">${title}</span></span>
      ${np && showStatus ? html`<i class="ph ${np.playing ? "ph-play" : "ph-pause"} spotify-status"></i>` : null}
    </span>
  `;
}

// Marquee-scrolls only when the fixed-width text box actually overflows - a config
// change or track update can flip this either way, so it's re-measured every repaint
// rather than latched once.
function updateTileMarquee(root: HTMLElement) {
  const box = root.querySelector<HTMLElement>(".spotify-tile-text");
  const inner = box?.querySelector<HTMLElement>(".spotify-tile-text-inner");
  if (!box || !inner) return;
  box.classList.toggle("marquee", inner.scrollWidth > box.clientWidth);
}

// ---------- flyout ----------

function fmtClock(ms: number): string {
  const totalSec = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function seekBar(np: SpotifyNowPlaying, positionMs: number, onSeek: (ms: number) => void) {
  const pct = np.duration_ms > 0 ? Math.min(100, (positionMs / np.duration_ms) * 100) : 0;
  const ratioFromEvent = (e: PointerEvent, el: HTMLElement): number => {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  };
  const onDown = (e: PointerEvent) => {
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    onSeek(ratioFromEvent(e, el) * np.duration_ms);
    const onMove = (ev: PointerEvent) => onSeek(ratioFromEvent(ev, el) * np.duration_ms);
    const onUp = () => {
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerup", onUp);
      invoke("spotify_seek", { positionMs: Math.round(ratioFromEvent(e, el) * np.duration_ms) }).catch(() => {});
    };
    el.addEventListener("pointermove", onMove);
    el.addEventListener("pointerup", onUp, { once: true });
  };
  return html`
    <div class="spotify-progress">
      <span class="spotify-time">${fmtClock(positionMs)}</span>
      <div class="bar spotify-seek" @pointerdown=${onDown}><div style="width:${pct}%"></div></div>
      <span class="spotify-time spotify-time-end">${fmtClock(Math.max(0, np.duration_ms - positionMs))}</span>
    </div>
  `;
}

function controlsRow(np: SpotifyNowPlaying) {
  const act = (id: string) => (e: Event) => {
    e.stopPropagation();
    invoke(id).catch(() => {});
  };
  return html`
    <div class="spotify-controls">
      <i
        class="ph ph-shuffle${np.shuffle ? " active" : ""}"
        @click=${act("spotify_toggle_shuffle")}
      ></i>
      <i class="ph ph-skip-back" @click=${act("spotify_previous")}></i>
      <div class="spotify-play-btn" @click=${act("spotify_toggle_play_pause")}>
        <i class="ph ${np.playing ? "ph-pause" : "ph-play"}"></i>
      </div>
      <i class="ph ph-skip-forward" @click=${act("spotify_next")}></i>
      <i
        class="ph ph-repeat${np.repeat !== "off" ? " active" : ""}"
        @click=${act("spotify_toggle_repeat")}
      ></i>
    </div>
  `;
}

function flyoutTemplate(np: SpotifyNowPlaying | null, showArt: boolean, positionMs: number, onSeek: (ms: number) => void) {
  if (!np) {
    return html`
      <div class="spotify-empty">
        <i class="ph ph-spotify-logo"></i>
        <span>Nothing playing</span>
      </div>
    `;
  }
  return html`
    ${artTemplate(np, showArt, "spotify-fly-art", "34px")}
    <div class="spotify-fly-title">${np.title || "Unknown track"}</div>
    <div class="spotify-fly-artist">${[np.artist, np.album].filter(Boolean).join(" — ")}</div>
    ${seekBar(np, positionMs, onSeek)}
    ${controlsRow(np)}
  `;
}

const configFields: ConfigField[] = [
  { key: "show_album_art", label: "Show album art", type: "toggle", default: true },
  { key: "show_status_icon", label: "Show play/pause icon", type: "toggle", default: true },
];

export const spotifyWidget: TaskbarWidget = {
  id: "spotify",
  name: "Spotify",
  icon: "ph-spotify-logo",
  flyout: { widthCss: FLYOUT_WIDTH, heightCss: FLYOUT_HEIGHT },
  overlay: { widthCss: FLYOUT_WIDTH, heightCss: FLYOUT_HEIGHT },
  menuItems: () => [{ id: "copy-info", label: "Copy track info" }],
  onMenuAction: (id) => {
    if (id !== "copy-info" || !lastNowPlaying) return;
    navigator.clipboard.writeText(`${lastNowPlaying.title} — ${lastNowPlaying.artist}`).catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    let latest: SpotifyNowPlaying | null = null;
    let showArt = true;
    let showStatus = true;
    const repaint = () => {
      render(tileTemplate(latest, showArt, showStatus), root);
      updateTileMarquee(root);
    };
    const onClick = () => {
      if (isDragging()) return;
      invoke("open_spotify").catch(() => {});
    };
    root.addEventListener("click", onClick);
    const stopData = subscribeNowPlaying((np) => {
      latest = np;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      const cfg = widgetConfig(s, "spotify");
      showArt = readShowArt(cfg);
      showStatus = readShowStatusIcon(cfg);
      repaint();
    });
    return () => {
      root.removeEventListener("click", onClick);
      stopData();
      stopSettings();
    };
  },
  mountFlyout(root) {
    let latest: SpotifyNowPlaying | null = null;
    let showArt = true;
    // Local wall-clock estimate between 1s backend pushes; resynced on every fresh
    // event and overridden live while the user is dragging the seek bar.
    let anchorPositionMs = 0;
    let anchorAtMs = Date.now();
    let dragPositionMs: number | null = null;

    const estimate = (): number => {
      if (dragPositionMs !== null) return dragPositionMs;
      if (!latest) return 0;
      if (!latest.playing) return anchorPositionMs;
      return Math.min(latest.duration_ms, anchorPositionMs + (Date.now() - anchorAtMs));
    };
    const repaint = () =>
      render(
        flyoutTemplate(latest, showArt, estimate(), (ms) => {
          dragPositionMs = Math.max(0, Math.min(latest?.duration_ms ?? 0, ms));
          repaint();
        }),
        root,
      );
    repaint();

    const stopData = subscribeNowPlaying((np) => {
      latest = np;
      anchorPositionMs = np?.position_ms ?? 0;
      anchorAtMs = Date.now();
      dragPositionMs = null;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      showArt = readShowArt(widgetConfig(s, "spotify"));
      repaint();
    });
    const tick = setInterval(() => {
      if (latest?.playing && dragPositionMs === null) repaint();
    }, 1000);
    return () => {
      stopData();
      stopSettings();
      clearInterval(tick);
    };
  },
};
