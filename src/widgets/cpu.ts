import { invoke } from "@tauri-apps/api/core";
import { html } from "lit-html";
import { ConfigField, TaskbarWidget } from "../shared/widget";
import {
  barRow,
  heat,
  mountStatsFlyout,
  mountStatsTile,
  procRow,
  ProcRow,
  readShowTemp,
  SystemStats,
} from "./system-shared";

function tileTemplate(s: SystemStats | null, showTemp: boolean) {
  if (!s) return html`<span class="muted">…</span>`;
  return html`
    <span class="stat ${heat(s.cpu_pct)}">
      <i class="ph ph-cpu"></i>
      <span class="val">${s.cpu_pct.toFixed(0)}%</span>
      ${s.cpu_temp_c != null && showTemp
        ? html`<span class="unit">${s.cpu_temp_c.toFixed(0)}°</span>`
        : null}
    </span>
  `;
}

function flyoutTemplate(s: SystemStats | null, showTemp: boolean, procs: ProcRow[]) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-cpu"></i>CPU</div>
    ${barRow(
      "ph-cpu",
      s.cpu_temp_c != null && showTemp ? `CPU · ${s.cpu_temp_c.toFixed(0)}°C` : "CPU",
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

const configFields: ConfigField[] = [
  { key: "show_temp", label: "Show CPU temperature", type: "toggle", default: true },
];

export const cpuWidget: TaskbarWidget = {
  id: "cpu",
  name: "CPU",
  flyout: { widthCss: 245, heightCss: 270 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    return mountStatsTile(root, "cpu", readShowTemp, tileTemplate);
  },
  mountFlyout(root) {
    return mountStatsFlyout(root, "cpu", readShowTemp, flyoutTemplate, "cpu");
  },
};
