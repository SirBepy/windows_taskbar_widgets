import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { fmtClock, isDragging, TaskbarWidget } from "../shared/widget";

type Phase = "work" | "short" | "long" | "other" | "snooze";

interface PomodoroPush {
  connected: boolean;
  phase?: Phase;
  running?: boolean;
  anchor_ms?: number;
  anchor_remaining_sec?: number;
}

// Exact hex values from pomodoro-overlay's src/styles/base.css .phase-* --bg.
const PHASE_COLOR: Record<Phase, string> = {
  work: "#ba4949",
  short: "#38858a",
  long: "#397097",
  other: "#4a8b3f",
  snooze: "#6b35a5",
};

const REFRESH_MS = 1000;

// Countdown for work/short/long/snooze while running; count-up for "other";
// frozen at anchor_remaining_sec whenever paused.
function displaySec(s: PomodoroPush, nowMs: number): number {
  const base = s.anchor_remaining_sec ?? 0;
  if (!s.running) return base;
  const elapsed = (nowMs - (s.anchor_ms ?? nowMs)) / 1000;
  return s.phase === "other" ? base + elapsed : Math.max(0, base - elapsed);
}

// Thin wrapper over the shared formatter, kept exported so pomodoro.test.ts's
// boundary tests still import it directly.
export function fmtTime(totalSec: number): string {
  return fmtClock(totalSec * 1000, { padMinutes: true, hours: true });
}

function cmd(action: string, phase?: string) {
  invoke("pomodoro_cmd", { action, phase: phase ?? null }).catch(() => {});
}

function toggle(live: PomodoroPush | null) {
  if (!live) return;
  if (isDragging()) return;
  cmd(live.running ? "pause" : "start");
}

// Same skeleton for connected and disconnected: only text/colour/opacity vary,
// so the two states share one box model and can never differ in footprint.
function tileTemplate(s: PomodoroPush | null, nowMs: number) {
  const live = s && s.connected ? s : null;
  const phase = live?.phase ?? "work";
  return html`
    <span class="pomo-tile ${live ? "" : "muted"}">
      <span
        class="phase-dot"
        style="background:${live ? PHASE_COLOR[phase] : "rgba(255,255,255,.3)"}"
      ></span>
      <span class="pomo-tile-time">${live ? fmtTime(displaySec(live, nowMs)) : "--:--"}</span>
      <i class="ph ${live?.running ? "ph-pause" : "ph-play"} pomo-status"></i>
    </span>
  `;
}

// Survives mount/unmount cycles (renderTiles tears down/remounts every tile on every
// settings save), so a remounted tile paints its last known state on its first frame
// instead of sitting blank/clipped until the next TCP-bridged push arrives.
let lastPomodoro: PomodoroPush | null = null;

/** Last known push synchronously, then live updates via the bridge's event. */
function subscribe(onState: (s: PomodoroPush) => void): () => void {
  let disposed = false;
  if (lastPomodoro) onState(lastPomodoro);
  const unlisten = listen<PomodoroPush>("pomodoro-state", (e) => {
    lastPomodoro = e.payload;
    if (!disposed) onState(e.payload);
  });
  return () => {
    disposed = true;
    unlisten.then((un) => un());
  };
}

export const pomodoroWidget: TaskbarWidget = {
  id: "pomodoro",
  name: "Pomodoro",
  icon: "ph-timer",
  // No flyout to inherit from, so overlay placement needs its own floor. The tile
  // renderer is what draws here, per overlayRenderer's fallback chain.
  overlay: { widthCss: 150, heightCss: 44 },
  menuItems: () => [{ id: "open-app", label: "Open Pomodoro Overlay" }],
  onMenuAction: (id) => {
    if (id === "open-app") invoke("focus_or_launch_app", { app: "pomodoro" }).catch(() => {});
  },
  mountTile(root) {
    let latest: PomodoroPush | null = null;
    let tick: number | undefined;
    const repaint = () => render(tileTemplate(latest, Date.now()), root);
    // Only a running timer changes between pushes: paused freezes at
    // anchor_remaining_sec and disconnected reads "--:--", so ticking in those
    // states would repaint the strip once a second to draw the same pixels.
    const retime = () => {
      const wanted = !!latest?.connected && !!latest.running;
      if (wanted === (tick !== undefined)) return;
      if (wanted) tick = window.setInterval(repaint, REFRESH_MS);
      else {
        clearInterval(tick);
        tick = undefined;
      }
    };
    const stop = subscribe((s) => {
      latest = s;
      repaint();
      retime();
    });
    const onClick = () => toggle(latest && latest.connected ? latest : null);
    root.addEventListener("click", onClick);
    return () => {
      root.removeEventListener("click", onClick);
      stop();
      clearInterval(tick);
    };
  },
};
