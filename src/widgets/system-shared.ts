import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render, TemplateResult } from "lit-html";
import { subscribeSettings, widgetConfig } from "../shared/widget";

export interface DiskInfo {
  name: string;
  free_bytes: number;
  total_bytes: number;
}

export interface GpuStats {
  util_pct: number;
  temp_c: number;
  vram_used_bytes: number;
  vram_total_bytes: number;
}

export interface SystemStats {
  cpu_pct: number;
  mem_used_bytes: number;
  mem_total_bytes: number;
  disks: DiskInfo[];
  gpu: GpuStats | null;
  cpu_temp_c: number | null;
}

export interface ProcRow {
  name: string;
  pct_or_bytes: number;
  count: number;
}

export function heat(pct: number): string {
  if (pct >= 90) return "hot";
  if (pct >= 70) return "warn";
  return "";
}

export function barRow(icon: string, label: string, pct: number, val: string) {
  return html`
    <div class="fly-row">
      <span class="label"><i class="ph ${icon}"></i>${label}</span>
      <div class="bar ${heat(pct)}"><div style="width:${Math.min(100, pct)}%"></div></div>
      <span class="val">${val}</span>
    </div>
  `;
}

export function procRow(p: ProcRow, fmtVal: (v: number) => string) {
  return html`
    <div class="fly-row">
      <span class="label proc-name"
        >${p.name}${p.count > 1 ? html`<span class="muted"> ×${p.count}</span>` : null}</span
      >
      <span class="val">${fmtVal(p.pct_or_bytes)}</span>
    </div>
  `;
}

/** Latest snapshot immediately, then live updates via the 2s poller's event. */
export function subscribeStats(onStats: (s: SystemStats) => void): () => void {
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

/** readCfg for the show_temp toggle cpu and gpu both expose. */
export const readShowTemp = (cfg: Record<string, unknown>): boolean =>
  (cfg.show_temp as boolean | undefined) ?? true;

/**
 * Stats + settings + repaint wiring, shared by the cpu/gpu tiles. readCfg maps this
 * widget's config blob to whatever its template needs, so readCfg({}) yields defaults.
 */
export function mountStatsTile<C>(
  root: HTMLElement,
  widgetId: string,
  readCfg: (cfg: Record<string, unknown>) => C,
  template: (s: SystemStats | null, cfg: C) => TemplateResult,
): () => void {
  let latest: SystemStats | null = null;
  let cfg = readCfg({});
  const repaint = () => render(template(latest, cfg), root);
  const stopStats = subscribeStats((s) => {
    latest = s;
    repaint();
  });
  const stopSettings = subscribeSettings((s) => {
    cfg = readCfg(widgetConfig(s, widgetId));
    repaint();
  });
  return () => {
    stopStats();
    stopSettings();
  };
}

/** Same, plus the empty-state paint before data lands and an optional top-process feed. */
export function mountStatsFlyout<C>(
  root: HTMLElement,
  widgetId: string,
  readCfg: (cfg: Record<string, unknown>) => C,
  template: (s: SystemStats | null, cfg: C, procs: ProcRow[]) => TemplateResult,
  procMetric?: "cpu" | "ram",
): () => void {
  let latest: SystemStats | null = null;
  let procs: ProcRow[] = [];
  let cfg = readCfg({});
  const repaint = () => render(template(latest, cfg, procs), root);
  repaint();
  const stopStats = subscribeStats((s) => {
    latest = s;
    repaint();
  });
  const stopProcs = procMetric
    ? subscribeTopProcesses(procMetric, (rows) => {
        procs = rows;
        repaint();
      })
    : null;
  const stopSettings = subscribeSettings((s) => {
    cfg = readCfg(widgetConfig(s, widgetId));
    repaint();
  });
  return () => {
    stopStats();
    stopProcs?.();
    stopSettings();
  };
}

/** Only runs while a flyout has it mounted; each call costs a process refresh on the Rust side. */
export function subscribeTopProcesses(
  metric: "cpu" | "ram",
  onRows: (rows: ProcRow[]) => void,
): () => void {
  let disposed = false;
  const refresh = () =>
    invoke<ProcRow[]>("get_top_processes", { metric }).then((rows) => {
      if (!disposed) onRows(rows);
    });
  refresh();
  const poll = setInterval(refresh, 2000);
  return () => {
    disposed = true;
    clearInterval(poll);
  };
}
