import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { isDragging, TaskbarWidget } from "../shared/widget";

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

// Exported so the fixed-width CSS budget can be tested against real boundary
// values without a DOM (see pomodoro.test.ts).
export function fmtTime(totalSec: number): string {
  const sec = Math.max(0, Math.round(totalSec));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function cmd(action: string, phase?: string) {
  invoke("pomodoro_cmd", { action, phase: phase ?? null }).catch(() => {});
}

function toggle(e: Event, s: PomodoroPush) {
  e.stopPropagation();
  if (isDragging()) return;
  cmd(s.running ? "pause" : "start");
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
      <button
        class="pomo-toggle"
        ?disabled=${!live}
        @click=${(e: Event) => live && toggle(e, live)}
      >
        <i class="ph ${live?.running ? "ph-pause" : "ph-play"}"></i>
      </button>
    </span>
  `;
}

function subscribe(onState: (s: PomodoroPush) => void): () => void {
  let disposed = false;
  const unlisten = listen<PomodoroPush>("pomodoro-state", (e) => {
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
    return () => {
      stop();
      clearInterval(tick);
    };
  },
};
