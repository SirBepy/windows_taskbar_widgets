import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { TaskbarWidget } from "../shared/widget";

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

const PHASE_TABS: { phase: "work" | "short" | "long" | "other"; label: string }[] = [
  { phase: "work", label: "Focus" },
  { phase: "short", label: "Break" },
  { phase: "long", label: "Big Break" },
  { phase: "other", label: "Other" },
];

const REFRESH_MS = 1000;

// Countdown for work/short/long/snooze while running; count-up for "other";
// frozen at anchor_remaining_sec whenever paused.
function displaySec(s: PomodoroPush, nowMs: number): number {
  const base = s.anchor_remaining_sec ?? 0;
  if (!s.running) return base;
  const elapsed = (nowMs - (s.anchor_ms ?? nowMs)) / 1000;
  return s.phase === "other" ? base + elapsed : Math.max(0, base - elapsed);
}

function fmtTime(totalSec: number): string {
  const sec = Math.max(0, Math.round(totalSec));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function tileTemplate(s: PomodoroPush | null, nowMs: number) {
  if (!s || !s.connected) return html`<i class="ph ph-timer muted"></i>`;
  return html`
    <span class="stat">
      <span class="phase-dot" style="background:${PHASE_COLOR[s.phase ?? "work"]}"></span>
      <span class="val">${fmtTime(displaySec(s, nowMs))}</span>
    </span>
  `;
}

function cmd(action: string, phase?: string) {
  invoke("pomodoro_cmd", { action, phase: phase ?? null }).catch(() => {});
}

function flyoutTemplate(s: PomodoroPush | null, nowMs: number) {
  if (!s || !s.connected) {
    return html`
      <div class="fly-title"><i class="ph ph-timer"></i>Pomodoro</div>
      <div class="empty">Pomodoro Overlay isn't running.</div>
    `;
  }
  const phase = s.phase ?? "work";
  const showSkip = !!s.running;
  const showSnooze = phase === "short" || phase === "long" || phase === "snooze";
  return html`
    <div class="fly-title"><i class="ph ph-timer"></i>Pomodoro</div>
    <div class="pomo-time" style="color:${PHASE_COLOR[phase]}">${fmtTime(displaySec(s, nowMs))}</div>
    <div class="pomo-tabs">
      ${PHASE_TABS.map(
        (t) => html`
          <button
            class="pomo-tab ${phase === t.phase ? "active" : ""}"
            @click=${() => cmd("switch-phase", t.phase)}
          >
            ${t.label}
          </button>
        `,
      )}
    </div>
    <div class="pomo-actions">
      <button class="pomo-btn" @click=${() => cmd(s.running ? "pause" : "start")}>
        ${s.running ? "PAUSE" : "START"}
      </button>
      ${showSkip ? html`<button class="pomo-btn" @click=${() => cmd("skip")}>Skip</button>` : null}
      ${showSnooze
        ? html`<button class="pomo-btn" @click=${() => cmd("snooze")}>Snooze</button>`
        : null}
    </div>
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
  flyout: { widthCss: 320, heightCss: 240 },
  menuItems: () => [{ id: "open-app", label: "Open Pomodoro Overlay" }],
  onMenuAction: (id) => {
    if (id === "open-app") invoke("focus_or_launch_app", { app: "pomodoro" }).catch(() => {});
  },
  mountTile(root) {
    let latest: PomodoroPush | null = null;
    const repaint = () => render(tileTemplate(latest, Date.now()), root);
    const stop = subscribe((s) => {
      latest = s;
      repaint();
    });
    const tick = setInterval(repaint, REFRESH_MS);
    return () => {
      stop();
      clearInterval(tick);
    };
  },
  mountFlyout(root) {
    let latest: PomodoroPush | null = null;
    const repaint = () => render(flyoutTemplate(latest, Date.now()), root);
    repaint();
    const stop = subscribe((s) => {
      latest = s;
      repaint();
    });
    const tick = setInterval(repaint, REFRESH_MS);
    return () => {
      stop();
      clearInterval(tick);
    };
  },
};
