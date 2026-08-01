import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { fmtBytes, TaskbarWidget } from "../shared/widget";

interface DiskInfo {
  name: string;
  free_bytes: number;
  total_bytes: number;
}

interface GpuStats {
  util_pct: number;
  temp_c: number;
  vram_used_bytes: number;
  vram_total_bytes: number;
}

interface SystemStats {
  cpu_pct: number;
  mem_used_bytes: number;
  mem_total_bytes: number;
  disks: DiskInfo[];
  gpu: GpuStats | null;
  cpu_temp_c: number | null;
}

function heat(pct: number): string {
  if (pct >= 90) return "hot";
  if (pct >= 70) return "warn";
  return "";
}

function tileTemplate(s: SystemStats | null) {
  if (!s) return html`<span class="muted">…</span>`;
  const memPct = s.mem_total_bytes ? (s.mem_used_bytes / s.mem_total_bytes) * 100 : 0;
  return html`
    <span class="stat ${heat(s.cpu_pct)}">
      <i class="ph ph-cpu"></i>
      <span class="val">${s.cpu_pct.toFixed(0)}%</span>
      ${s.cpu_temp_c != null
        ? html`<span class="unit">${s.cpu_temp_c.toFixed(0)}°</span>`
        : null}
    </span>
    <span class="stat ${heat(memPct)}">
      <i class="ph ph-memory"></i>
      <span class="val">${(s.mem_used_bytes / 1024 ** 3).toFixed(1)}</span>
      <span class="unit">/${(s.mem_total_bytes / 1024 ** 3).toFixed(0)} GB</span>
    </span>
    ${s.gpu
      ? html`<span class="stat ${heat(s.gpu.util_pct)}">
          <i class="ph ph-graphics-card"></i>
          <span class="val">${s.gpu.util_pct}%</span>
          <span class="unit">${s.gpu.temp_c}°</span>
        </span>`
      : null}
  `;
}

function barRow(icon: string, label: string, pct: number, val: string) {
  return html`
    <div class="fly-row">
      <span class="label"><i class="ph ${icon}"></i>${label}</span>
      <div class="bar ${heat(pct)}"><div style="width:${Math.min(100, pct)}%"></div></div>
      <span class="val">${val}</span>
    </div>
  `;
}

function flyoutTemplate(s: SystemStats | null) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  const memPct = s.mem_total_bytes ? (s.mem_used_bytes / s.mem_total_bytes) * 100 : 0;
  return html`
    <div class="fly-title"><i class="ph ph-gauge"></i>System</div>
    ${barRow(
      "ph-cpu",
      s.cpu_temp_c != null ? `CPU · ${s.cpu_temp_c.toFixed(0)}°C` : "CPU",
      s.cpu_pct,
      `${s.cpu_pct.toFixed(0)}%`,
    )}
    ${barRow(
      "ph-memory",
      "RAM",
      memPct,
      `${fmtBytes(s.mem_used_bytes)} / ${fmtBytes(s.mem_total_bytes, 0)}`,
    )}
    ${s.gpu
      ? html`
          ${barRow(
            "ph-graphics-card",
            `GPU · ${s.gpu.temp_c}°C`,
            s.gpu.util_pct,
            `${s.gpu.util_pct}%`,
          )}
          ${barRow(
            "ph-graphics-card",
            "VRAM",
            s.gpu.vram_total_bytes
              ? (s.gpu.vram_used_bytes / s.gpu.vram_total_bytes) * 100
              : 0,
            `${fmtBytes(s.gpu.vram_used_bytes)} / ${fmtBytes(s.gpu.vram_total_bytes, 0)}`,
          )}
        `
      : null}
    ${s.disks.map((d) => {
      const usedPct = d.total_bytes
        ? ((d.total_bytes - d.free_bytes) / d.total_bytes) * 100
        : 0;
      return barRow(
        "ph-hard-drives",
        d.name.replace(/\\$/, ""),
        usedPct,
        `${fmtBytes(d.free_bytes, 0)} free`,
      );
    })}
  `;
}

function subscribe(onStats: (s: SystemStats) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  invoke<SystemStats>("get_system_stats").then((s) => {
    if (!disposed && s.mem_total_bytes > 0) onStats(s);
  });
  listen<SystemStats>("system-stats", (e) => onStats(e.payload)).then((un) => {
    if (disposed) un();
    else unlisten = un;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export const systemWidget: TaskbarWidget = {
  id: "system",
  name: "System stats",
  flyout: { widthCss: 340, heightCss: 320 },
  mountTile(root) {
    return subscribe((s) => render(tileTemplate(s), root));
  },
  mountFlyout(root) {
    render(flyoutTemplate(null), root);
    return subscribe((s) => render(flyoutTemplate(s), root));
  },
};
