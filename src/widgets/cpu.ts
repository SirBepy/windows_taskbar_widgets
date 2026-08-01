import { invoke } from "@tauri-apps/api/core";
import { html, render } from "lit-html";
import { ConfigField, subscribeSettings, TaskbarWidget, widgetConfig } from "../shared/widget";
import {
  barRow,
  heat,
  procRow,
  ProcRow,
  subscribeStats,
  subscribeTopProcesses,
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

function flyoutTemplate(s: SystemStats | null, procs: ProcRow[], showTemp: boolean) {
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
  flyout: { widthCss: 320, heightCss: 300 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    let latestStats: SystemStats | null = null;
    let showTemp = true;
    const repaint = () => render(tileTemplate(latestStats, showTemp), root);
    const stopStats = subscribeStats((s) => {
      latestStats = s;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      showTemp = (widgetConfig(s, "cpu").show_temp as boolean | undefined) ?? true;
      repaint();
    });
    return () => {
      stopStats();
      stopSettings();
    };
  },
  mountFlyout(root) {
    render(flyoutTemplate(null, [], true), root);
    let latestStats: SystemStats | null = null;
    let latestProcs: ProcRow[] = [];
    let showTemp = true;
    const repaint = () => render(flyoutTemplate(latestStats, latestProcs, showTemp), root);
    const stopStats = subscribeStats((s) => {
      latestStats = s;
      repaint();
    });
    const stopProcs = subscribeTopProcesses("cpu", (rows) => {
      latestProcs = rows;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      showTemp = (widgetConfig(s, "cpu").show_temp as boolean | undefined) ?? true;
      repaint();
    });
    return () => {
      stopStats();
      stopProcs();
      stopSettings();
    };
  },
};
