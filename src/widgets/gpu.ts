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
  readShowPercent,
  readShowTemp,
  SystemStats,
} from "./system-shared";

interface GpuCfg {
  showTemp: boolean;
  showPercent: boolean;
}

const readCfg = (cfg: Record<string, unknown>): GpuCfg => ({
  showTemp: readShowTemp(cfg),
  showPercent: readShowPercent(cfg),
});

// poller.rs::read_gpu is a chain of per-poll `.ok()?` NVML reads, so a machine that
// has a GPU can still report None on any single tick. Collapsing the whole tile then
// would resize the strip, so once a GPU has been seen the tile stays and blanks.
let gpuSeen = false;

function tileTemplate(s: SystemStats | null, cfg: GpuCfg) {
  if (s?.gpu) gpuSeen = true;
  else if (!gpuSeen) return html``;
  const g = s?.gpu ?? null;
  return html`
    <div class="tile-stat ${g ? heat(g.util_pct) : ""}">
      <span class="tile-label">GPU</span>
      <span class="tile-value ${g ? "" : "reserved"}">
        <span class="tile-pct"><span class="num">${g ? g.util_pct : ""}</span>%</span>
        ${cfg.showTemp
          ? html`<span class="tile-unit"><span class="num">${g ? g.temp_c : ""}</span>°</span>`
          : null}
      </span>
    </div>
  `;
}

function flyoutTemplate(s: SystemStats | null, cfg: GpuCfg, procs: ProcRow[]) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  if (!s.gpu) return html`<div class="empty">No discrete GPU detected.</div>`;
  const vramPct = s.gpu.vram_total_bytes ? (s.gpu.vram_used_bytes / s.gpu.vram_total_bytes) * 100 : 0;
  return html`
    <div class="fly-title"><i class="ph ph-graphics-card"></i>GPU</div>
    ${barRow(
      "ph-graphics-card",
      cfg.showTemp ? `GPU · ${s.gpu.temp_c}°C` : "GPU",
      s.gpu.util_pct,
      `${s.gpu.util_pct}%`,
    )}
    ${barRow(
      "ph-graphics-card",
      "VRAM",
      vramPct,
      cfg.showPercent
        ? `${vramPct.toFixed(0)}%`
        : `${fmtBytes(s.gpu.vram_used_bytes)} / ${fmtBytes(s.gpu.vram_total_bytes, 0)}`,
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
  { key: "show_percent", label: "Show as percentage", type: "toggle", default: true },
];

export const gpuWidget: TaskbarWidget = {
  id: "gpu",
  name: "GPU",
  icon: "ph-graphics-card",
  flyout: { widthCss: 310, heightCss: 300 },
  menuItems: () => [{ id: "task-manager", label: "Open Task Manager" }],
  onMenuAction: (id) => {
    if (id === "task-manager") invoke("open_task_manager").catch(() => {});
  },
  configFields: () => configFields,
  mountTile(root) {
    return mountStatsTile(root, "gpu", readCfg, tileTemplate);
  },
  mountFlyout(root) {
    return mountStatsFlyout(root, "gpu", readCfg, flyoutTemplate, "gpu");
  },
};
