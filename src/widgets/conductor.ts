import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render, svg } from "lit-html";
import { fmtCountdown, isDragging, msUntil, TaskbarWidget } from "../shared/widget";
import { heat } from "./system-shared";

interface AccountUsage {
  id: string;
  label: string;
  colour: string;
  icon: string;
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

// Label/colour/icon come from the last DB poll; a snapshot for an unknown
// account (no poll yet, or a brand-new account not in that snapshot) is dropped.
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

// Geometry ported 1:1 from claude_usage_in_taskbar's usage-dial.ts. 36px because
// a default 48px taskbar minus #strip's 4px v-padding leaves 40px of tile.
const DIAL_SIZE_PX = 36;
const VIEWBOX = 44;
const CENTER = 22;
const OUTER_R = 19;
const OUTER_W = 4.5;
const INNER_R = 12;
const INNER_W = 3;
const TRACK_COLOR = "rgba(255, 255, 255, 0.13)";
const ICON_FALLBACK = "user";

// Ported from formatters.ts's getPaceColor (band=10). The only ring palette:
// account colour and heat thresholds no longer touch a stroke. Both args 0-100.
function paceColor(cur: number, safe: number): string {
  if (cur < safe - 10) return "#27ae60";
  if (cur < safe) return "#f1c40f";
  if (cur < safe + 10) return "#e67e22";
  return "#e74c3c";
}

// formatters.ts's pctColor. Only used with no safe-pace anchor, so "pace
// unknown" never renders as the pace palette's "at zero pace" green.
function flatColor(pct: number): string {
  if (pct >= 80) return "#e74c3c";
  if (pct >= 50) return "#e67e22";
  return "#27ae60";
}

// null means "no safe-pace anchor", never "assume 0" - the overlay's
// computeSafePacePct clamps, losing the "unknown" vs "at 0%" distinction.
function elapsedFraction(resetsAt: string | null, windowMs: number): number | null {
  const msLeft = msUntil(resetsAt);
  if (msLeft === null) return null;
  const frac = 1 - msLeft / windowMs;
  return frac >= 0 && frac <= 1 ? frac : null;
}

// Starts at 12 o'clock, clockwise. cap only on fills, never the base track.
function seg(r: number, w: number, lenPct: number, stroke: string, cap: boolean, opacity = 1) {
  const c = 2 * Math.PI * r;
  const dash = `${(Math.max(0, Math.min(100, lenPct)) / 100) * c} ${c}`;
  return svg`<circle cx="${CENTER}" cy="${CENTER}" r="${r}" fill="none" stroke="${stroke}"
    stroke-width="${w}" stroke-dasharray="${dash}" opacity="${opacity}"
    stroke-linecap="${cap ? "round" : "butt"}" transform="rotate(-90 ${CENTER} ${CENTER})" />`;
}

// Under pace ghosts the gap out to safe; over pace darkens back to it, so the
// overshoot is the part that reads bright.
function ringFill(r: number, w: number, cur: number, safe: number, color: string) {
  if (cur === safe) return seg(r, w, cur, color, true);
  if (cur < safe) return svg`${seg(r, w, safe, color, true, 0.3)}${seg(r, w, cur, color, true)}`;
  const darker = `color-mix(in srgb, ${color} 52%, #08060c)`;
  return svg`${seg(r, w, cur, color, true)}${seg(r, w, safe, darker, true)}`;
}

// safePct null collapses safe to cur, so pace-unknown is one solid arc in
// flatColor rather than a ghosted pace-palette one.
function ringSvg(r: number, w: number, pct: number, safePct: number | null) {
  const cur = Math.max(0, Math.min(100, pct));
  const safe = safePct !== null ? Math.max(0, Math.min(100, safePct)) : cur;
  const color = safePct !== null ? paceColor(cur, safe) : flatColor(cur);
  return svg`${seg(r, w, 100, TRACK_COLOR, false)}${ringFill(r, w, cur, safe, color)}`;
}

// Outer ring = 5h window, inner = 7d. a.colour tints the icon only: identity,
// never pace. Click re-triggers a live claude.ai poll (fire-and-forget).
function miniDial(a: AccountUsage) {
  const fiveHourFrac = elapsedFraction(a.five_hour_resets_at, FIVE_HOUR_MS);
  const sevenDayFrac = elapsedFraction(a.seven_day_resets_at, SEVEN_DAY_MS);
  const toSafePct = (f: number | null) => (f === null ? null : f * 100);
  const paceNote = (f: number | null) => (f === null ? "" : ` (safe pace ${(f * 100).toFixed(0)}%)`);
  const icon = a.icon || ICON_FALLBACK;
  return svg`
    <svg class="dial" width="${DIAL_SIZE_PX}" height="${DIAL_SIZE_PX}" viewBox="0 0 ${VIEWBOX} ${VIEWBOX}" @click=${requestRefresh}>
      <title>${a.label}: 5h ${a.five_hour_pct.toFixed(0)}%${paceNote(fiveHourFrac)} · 7d ${a.seven_day_pct.toFixed(0)}%${paceNote(sevenDayFrac)}</title>
      ${ringSvg(OUTER_R, OUTER_W, a.five_hour_pct, toSafePct(fiveHourFrac))}
      ${ringSvg(INNER_R, INNER_W, a.seven_day_pct, toSafePct(sevenDayFrac))}
      <foreignObject x="${CENTER - 9}" y="${CENTER - 9}" width="18" height="18">
        <div xmlns="http://www.w3.org/1999/xhtml" class="dial-icon" style="color:${a.colour}">
          <i class="ph ph-${icon}"></i>
        </div>
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

// Base: 30s DB poll (source of truth for account label/colour/icon). On top of
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
