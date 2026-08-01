import { invoke } from "@tauri-apps/api/core";
import { html, render, svg } from "lit-html";
import { fmtCountdown, TaskbarWidget } from "../shared/widget";

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

const POLL_MS = 30_000;

// Mirrors conductor's own dual-ring dial: outer = 5h window, inner = 7d window.
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
    <svg class="dial" width="30" height="30" viewBox="0 0 32 32">
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

function subscribe(onData: (u: ConductorUsage) => void): () => void {
  let disposed = false;
  const refresh = () =>
    invoke<ConductorUsage>("get_conductor_usage").then((u) => {
      if (!disposed) onData(u);
    });
  refresh();
  const poll = setInterval(refresh, POLL_MS);
  return () => {
    disposed = true;
    clearInterval(poll);
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
