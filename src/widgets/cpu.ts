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

// Afterburner can start or quit at any poll, so an arriving temperature is data,
// not a config toggle: the slot holds its width and blanks rather than collapsing.
const tempSlot = (temp: number | null) =>
  html`<span class="tile-unit ${temp == null ? "reserved" : ""}"
    ><span class="num">${temp == null ? "" : temp.toFixed(0)}</span>°</span
  >`;

function tileTemplate(s: SystemStats | null, showTemp: boolean) {
  if (!s) return html`<span class="muted">…</span>`;
  return html`
    <div class="tile-stat ${heat(s.cpu_pct)}">
      <span class="tile-label">CPU</span>
      <span class="tile-value">
        <span class="tile-pct"><span class="num">${s.cpu_pct.toFixed(0)}</span>%</span>
        ${showTemp ? tempSlot(s.cpu_temp_c) : null}
      </span>
    </div>
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
  icon: "ph-cpu",
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
