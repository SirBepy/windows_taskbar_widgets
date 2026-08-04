import { invoke } from "@tauri-apps/api/core";
import { html, render } from "lit-html";
import { ConfigField, fmtBytes, TaskbarWidget } from "../shared/widget";
import {
  barRow,
  heat,
  mountStatsTile,
  procRow,
  ProcRow,
  readShowPercent,
  subscribeStats,
  subscribeTopProcesses,
  SystemStats,
} from "./system-shared";

function tileTemplate(s: SystemStats | null, showPercent: boolean) {
  if (!s) return html`<span class="muted">…</span>`;
  const memPct = s.mem_total_bytes ? (s.mem_used_bytes / s.mem_total_bytes) * 100 : 0;
  return html`
    <div class="tile-stat ${heat(memPct)}">
      <span class="tile-label">RAM</span>
      <span class="tile-value">
        ${showPercent
          ? html`<span class="tile-pct"><span class="num">${memPct.toFixed(0)}</span>%</span>`
          : html`
              <span class="tile-pct">${(s.mem_used_bytes / 1024 ** 3).toFixed(1)}</span>
              <span class="tile-unit">/${(s.mem_total_bytes / 1024 ** 3).toFixed(0)} GB</span>
            `}
      </span>
    </div>
  `;
}

function flyoutTemplate(s: SystemStats | null, procs: ProcRow[]) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  const memPct = s.mem_total_bytes ? (s.mem_used_bytes / s.mem_total_bytes) * 100 : 0;
  return html`
    <div class="fly-title"><i class="ph ph-memory"></i>RAM</div>
    ${barRow(
      "ph-memory",
      "RAM",
      memPct,
      `${fmtBytes(s.mem_used_bytes)} / ${fmtBytes(s.mem_total_bytes, 0)}`,
    )}
    ${procs.length
      ? html`
          <div class="fly-subtitle">Top processes</div>
          ${procs.map((p) => procRow(p, (v) => fmtBytes(v)))}
        `
      : null}
  `;
}

const configFields: ConfigField[] = [
  { key: "show_percent", label: "Show as percentage", type: "toggle", default: true },
];

export const ramWidget: TaskbarWidget = {
  id: "ram",
  name: "RAM",
  flyout: { widthCss: 320, heightCss: 270 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    return mountStatsTile(root, "ram", readShowPercent, tileTemplate);
  },
  mountFlyout(root) {
    render(flyoutTemplate(null, []), root);
    let latestStats: SystemStats | null = null;
    let latestProcs: ProcRow[] = [];
    const repaint = () => render(flyoutTemplate(latestStats, latestProcs), root);
    const stopStats = subscribeStats((s) => {
      latestStats = s;
      repaint();
    });
    const stopProcs = subscribeTopProcesses("ram", (rows) => {
      latestProcs = rows;
      repaint();
    });
    return () => {
      stopStats();
      stopProcs();
    };
  },
};
