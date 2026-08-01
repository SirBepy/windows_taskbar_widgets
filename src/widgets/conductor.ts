import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render, svg } from "lit-html";
import { fmtCountdown, isDragging, TaskbarWidget } from "../shared/widget";

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

// Mirrors conductor's own dual-ring dial: outer = 5h window, inner = 7d window.
// Clicking re-triggers a live claude.ai poll (fire-and-forget).
function miniDial(a: AccountUsage) {
  const ring = (r: number, w: number, pct: number, opacity: number) => {
    const c = 2 * Math.PI * r;
    const filled = (Math.min(100, Math.max(0, pct)) / 100) * c;
    return svg`
      <circle cx="16" cy="16" r="${r}" fill="none" stroke="${a.colour}"
        stroke-opacity="0.18" stroke-width="${w}" />
      <circle cx="16" cy="16" r="${r}" fill="none" stroke="${a.colour}"
        stroke-opacity="${opacity}" stroke-width="${w}"
        stroke-dasharray="${filled} ${c - filled}"
        stroke-linecap="round" transform="rotate(-90 16 16)" />
    `;
  };
  return svg`
    <svg class="dial" width="30" height="30" viewBox="0 0 32 32" @click=${requestRefresh}>
      <title>${a.label}: 5h ${a.five_hour_pct.toFixed(0)}% · 7d ${a.seven_day_pct.toFixed(0)}%</title>
      ${ring(13, 3.4, a.five_hour_pct, 0.95)}
      ${ring(8, 2.6, a.seven_day_pct, 0.7)}
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
      <div class="bar ${pct >= 90 ? "hot" : pct >= 70 ? "warn" : ""}">
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
  flyout: { widthCss: 320, heightCss: 300 },
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
    // 1Hz repaint keeps the reset countdowns live between 30s polls.
    const tick = setInterval(() => {
      if (latest) render(flyoutTemplate(latest), root);
    }, 1000);
    return () => {
      stop();
      clearInterval(tick);
    };
  },
};
