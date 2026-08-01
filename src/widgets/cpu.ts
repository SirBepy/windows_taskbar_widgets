import { invoke } from "@tauri-apps/api/core";
import { html, render } from "lit-html";
import { TaskbarWidget } from "../shared/widget";
import {
  barRow,
  heat,
  procRow,
  ProcRow,
  subscribeStats,
  subscribeTopProcesses,
  SystemStats,
} from "./system-shared";

function tileTemplate(s: SystemStats | null) {
  if (!s) return html`<span class="muted">…</span>`;
  return html`
    <span class="stat ${heat(s.cpu_pct)}">
      <i class="ph ph-cpu"></i>
      <span class="val">${s.cpu_pct.toFixed(0)}%</span>
      ${s.cpu_temp_c != null
        ? html`<span class="unit">${s.cpu_temp_c.toFixed(0)}°</span>`
        : null}
    </span>
  `;
}

function flyoutTemplate(s: SystemStats | null, procs: ProcRow[]) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-cpu"></i>CPU</div>
    ${barRow(
      "ph-cpu",
      s.cpu_temp_c != null ? `CPU · ${s.cpu_temp_c.toFixed(0)}°C` : "CPU",
      s.cpu_pct,
      `${s.cpu_pct.toFixed(0)}%`,
    )}
    ${procs.length
      ? html`
          <div class="fly-subtitle">Top processes</div>
          ${procs.map((p) => procRow(p, (v) => `${v.toFixed(0)}%`))}
        `
      : null}
  `;
}

export const cpuWidget: TaskbarWidget = {
  id: "cpu",
  name: "CPU",
  flyout: { widthCss: 320, heightCss: 300 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  mountTile(root) {
    return subscribeStats((s) => render(tileTemplate(s), root));
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
    const stopProcs = subscribeTopProcesses("cpu", (rows) => {
      latestProcs = rows;
      repaint();
    });
    return () => {
      stopStats();
      stopProcs();
    };
  },
};
