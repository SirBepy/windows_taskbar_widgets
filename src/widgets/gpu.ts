import { invoke } from "@tauri-apps/api/core";
import { html } from "lit-html";
import { ConfigField, fmtBytes, TaskbarWidget } from "../shared/widget";
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
  if (!s?.gpu) return html``;
  return html`
    <span class="stat ${heat(s.gpu.util_pct)}">
      <i class="ph ph-graphics-card"></i>
      <span class="val">${s.gpu.util_pct}%</span>
      ${showTemp ? html`<span class="unit">${s.gpu.temp_c}°</span>` : null}
    </span>
  `;
}

function flyoutTemplate(s: SystemStats | null, showTemp: boolean, procs: ProcRow[]) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  if (!s.gpu) return html`<div class="empty">No discrete GPU detected.</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-graphics-card"></i>GPU</div>
    ${barRow(
      "ph-graphics-card",
      showTemp ? `GPU · ${s.gpu.temp_c}°C` : "GPU",
      s.gpu.util_pct,
      `${s.gpu.util_pct}%`,
    )}
    ${barRow(
      "ph-graphics-card",
      "VRAM",
      s.gpu.vram_total_bytes ? (s.gpu.vram_used_bytes / s.gpu.vram_total_bytes) * 100 : 0,
      `${fmtBytes(s.gpu.vram_used_bytes)} / ${fmtBytes(s.gpu.vram_total_bytes, 0)}`,
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
  { key: "show_temp", label: "Show GPU temperature", type: "toggle", default: true },
];

export const gpuWidget: TaskbarWidget = {
  id: "gpu",
  name: "GPU",
  flyout: { widthCss: 310, heightCss: 300 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    return mountStatsTile(root, "gpu", readShowTemp, tileTemplate);
  },
  mountFlyout(root) {
    return mountStatsFlyout(root, "gpu", readShowTemp, flyoutTemplate, "gpu");
  },
};
