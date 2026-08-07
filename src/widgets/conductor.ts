import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render, svg } from "lit-html";
import { fmtCountdown, isDragging, msUntil, TaskbarWidget } from "../shared/widget";
import { heat } from "./system-shared";

interface AccountUsage {
  id: string;
  label: string;
  colour: string;
  captured_at: string;
  five_hour_pct: number;
  five_hour_resets_at: string | null;
  seven_day_pct: number;
  seven_day_resets_at: string | null;
}

interface ConductorUsage {
  available: boolean;
  accounts: AccountUsage[];
}

// conductor_data.rs's fallback id when accounts.json is empty (single legacy entry).
const LEGACY_ID = "legacy";

// Raw WS push params (bridge_conductor.rs's Frame.params, conductor's own
// UsageSnapshot shape) - null account_id means the legacy single-entry.
interface LiveUsageSnapshot {
  captured_at: string;
  five_hour: { utilization: number; resets_at: string | null };
  seven_day: { utilization: number; resets_at: string | null };
  account_id: string | null;
}

const POLL_MS = 30_000;

// Label/colour come from the last DB poll; a snapshot for an unknown account
// (no poll yet, or a brand-new account not in that snapshot) is dropped.
function mergeLive(current: ConductorUsage | null, snap: LiveUsageSnapshot): ConductorUsage | null {
  if (!current) return current;
  const targetId = snap.account_id ?? LEGACY_ID;
  let changed = false;
  const accounts = current.accounts.map((a) => {
    if (a.id !== targetId) return a;
    changed = true;
    return {
      ...a,
      captured_at: snap.captured_at,
      five_hour_pct: snap.five_hour.utilization,
      five_hour_resets_at: snap.five_hour.resets_at,
      seven_day_pct: snap.seven_day.utilization,
      seven_day_resets_at: snap.seven_day.resets_at,
    };
  });
  return changed ? { ...current, accounts } : current;
}

function requestRefresh(e: Event) {
  e.stopPropagation();
  if (isDragging()) return;
  invoke("conductor_refresh_now").catch(() => {});
}

const FIVE_HOUR_MS = 5 * 3_600_000;
const SEVEN_DAY_MS = 7 * 86_400_000;

// Same warn/hot hexes as .tile-stat's CSS classes, reused literally since this
// is an inline SVG stroke and can't reference a CSS class.
const HEAT_STROKE: Record<string, string> = { warn: "#f0b232", hot: "#f04747" };
const TRACK_COLOR = "rgba(255, 255, 255, 0.13)";
const PACE_COLOR = "rgba(148, 163, 184, 0.55)"; // slate: distinct from the white track and any warm heat/account colour

// null means "don't draw a pace arc" - not "assume 0" - covers missing resets_at,
// an already-past reset, and any out-of-window fraction (bad/stale data).
function elapsedFraction(resetsAt: string | null, windowMs: number): number | null {
  const msLeft = msUntil(resetsAt);
  if (msLeft === null) return null;
  const frac = 1 - msLeft / windowMs;
  return frac >= 0 && frac <= 1 ? frac : null;
}

// Mirrors conductor's own dual-ring dial: outer = 5h window, inner = 7d window.
// Clicking re-triggers a live claude.ai poll (fire-and-forget).
function miniDial(a: AccountUsage) {
  const ring = (r: number, w: number, pct: number, pace: number | null, opacity: number) => {
    const c = 2 * Math.PI * r;
    const usage = Math.min(1, Math.max(0, pct / 100));
    const filled = usage * c;
    const h = heat(pct);
    const progressColor = h ? HEAT_STROKE[h] : pace !== null && usage > pace ? HEAT_STROKE.warn : a.colour;
    return svg`
      <circle cx="16" cy="16" r="${r}" fill="none" stroke="${TRACK_COLOR}" stroke-width="${w}" />
      ${
        pace === null
          ? null
          : svg`<circle cx="16" cy="16" r="${r}" fill="none" stroke="${PACE_COLOR}" stroke-width="${w}"
              stroke-dasharray="${pace * c} ${c - pace * c}" stroke-linecap="round"
              transform="rotate(-90 16 16)" />`
      }
      <circle cx="16" cy="16" r="${r}" fill="none" stroke="${progressColor}"
        stroke-opacity="${opacity}" stroke-width="${w}"
        stroke-dasharray="${filled} ${c - filled}"
        stroke-linecap="round" transform="rotate(-90 16 16)" />
    `;
  };
  const fiveHourPace = elapsedFraction(a.five_hour_resets_at, FIVE_HOUR_MS);
  const sevenDayPace = elapsedFraction(a.seven_day_resets_at, SEVEN_DAY_MS);
  const paceNote = (pace: number | null) => (pace === null ? "" : ` (pace ${(pace * 100).toFixed(0)}%)`);
  return svg`
    <svg class="dial" width="30" height="30" viewBox="0 0 32 32" @click=${requestRefresh}>
      <title>${a.label}: 5h ${a.five_hour_pct.toFixed(0)}%${paceNote(fiveHourPace)} · 7d ${a.seven_day_pct.toFixed(0)}%${paceNote(sevenDayPace)}</title>
      ${ring(13, 3.4, a.five_hour_pct, fiveHourPace, 0.95)}
      ${ring(8, 2.6, a.seven_day_pct, sevenDayPace, 0.7)}
      <foreignObject x="11" y="11" width="10" height="10">
        <div xmlns="http://www.w3.org/1999/xhtml" class="dial-icon"><i class="ph ph-robot"></i></div>
      </foreignObject>
    </svg>
  `;
}

function tileTemplate(u: ConductorUsage | null) {
  if (!u) return html`<span class="muted">…</span>`;
  if (!u.available || u.accounts.length === 0)
    return html`<i class="ph ph-robot"></i><span class="muted">–</span>`;
  return html`<span class="dials">${u.accounts.map(miniDial)}</span>`;
}

function windowRow(name: string, pct: number, resetsAt: string | null) {
  const cd = fmtCountdown(resetsAt);
  return html`
    <div class="fly-row">
      <span class="label">${name}</span>
      <div class="bar ${heat(pct)}">
        <div style="width:${Math.min(100, pct)}%"></div>
      </div>
      <span class="val">${pct.toFixed(0)}%${cd ? html` <span class="muted">· ${cd}</span>` : ""}</span>
    </div>
  `;
}

function flyoutTemplate(u: ConductorUsage | null) {
  const body = !u
    ? html`<div class="empty">Loading…</div>`
    : !u.available
      ? html`<div class="empty">Claude Conductor data not found.</div>`
      : u.accounts.length === 0
        ? html`<div class="empty">No usage snapshots yet.</div>`
        : u.accounts.map(
            (a) => html`
              <div class="acct">
                <div class="acct-head">
                  <span class="acct-dot" style="background:${a.colour}"></span>
                  ${a.label}
                </div>
                ${windowRow("Session (5h)", a.five_hour_pct, a.five_hour_resets_at)}
                ${windowRow("Weekly (7d)", a.seven_day_pct, a.seven_day_resets_at)}
              </div>
            `,
          );
  return html`
    <div class="fly-title"><i class="ph ph-robot"></i>Claude usage</div>
    ${body}
  `;
}

// Base: 30s DB poll (source of truth for account label/colour). On top of
// that, "conductor-usage-live" WS pushes merge in and repaint immediately -
// the poll interval stays as the fallback when the daemon bridge is down.
function subscribe(onData: (u: ConductorUsage) => void): () => void {
  let disposed = false;
  let current: ConductorUsage | null = null;
  const emit = () => {
    if (!disposed && current) onData(current);
  };
  const refresh = () =>
    invoke<ConductorUsage>("get_conductor_usage").then((u) => {
      current = u;
      emit();
    });
  refresh();
  const poll = setInterval(refresh, POLL_MS);
  const unlisten = listen<LiveUsageSnapshot>("conductor-usage-live", (e) => {
    const merged = mergeLive(current, e.payload);
    if (merged) {
      current = merged;
      emit();
    }
  });
  return () => {
    disposed = true;
    clearInterval(poll);
    unlisten.then((un) => un());
  };
}

export const conductorWidget: TaskbarWidget = {
  id: "conductor",
  name: "Claude usage",
  icon: "ph-robot",
  flyout: { widthCss: 320, heightCss: 300 },
  menuItems: () => [{ id: "open-app", label: "Open Claude Conductor" }],
  onMenuAction: (id) => {
    if (id === "open-app") invoke("focus_or_launch_app", { app: "conductor" }).catch(() => {});
  },
  mountTile(root) {
    return subscribe((u) => render(tileTemplate(u), root));
  },
  mountFlyout(root) {
    render(flyoutTemplate(null), root);
    let latest: ConductorUsage | null = null;
    const stop = subscribe((u) => {
      latest = u;
      render(flyoutTemplate(u), root);
    });
    // Keeps the reset countdowns live between 30s polls. 10s, not 1s: fmtCountdown
    // renders whole minutes, so a faster tick just redraws identical text.
    const tick = setInterval(() => {
      if (latest) render(flyoutTemplate(latest), root);
    }, 10_000);
    return () => {
      stop();
      clearInterval(tick);
    };
  },
};
