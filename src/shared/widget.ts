import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ConfigField {
  key: string;
  label: string;
  type: "toggle" | "number" | "select";
  options?: { value: string; label: string }[];
  default?: unknown;
}

// v1 widget contract. Built-ins implement this directly; the planned external
// manifest/bundle loader will adapt third-party widgets onto this same shape.
export interface TaskbarWidget {
  id: string;
  name: string;
  /** Fixed at declare-time, sized for max content; see CLAUDE.md's fixed-size rule. Omit for tile-only widgets. */
  flyout?: { widthCss: number; heightCss: number };
  /** Render the always-visible strip tile. Returns a cleanup fn. */
  mountTile(root: HTMLElement): () => void;
  mountFlyout?(root: HTMLElement): () => void;
  /** Extra entries appended to this tile's native context menu. */
  menuItems?: () => { id: string; label: string }[];
  onMenuAction?: (id: string) => void;
  /** Rows rendered in the dashboard's per-widget config accordion. */
  configFields?: () => ConfigField[];
}

export interface Settings {
  left_margin: number;
  enabled_widgets: string[];
  hidden_widgets: string[];
  stats_poll_seconds: number;
  opacity: number;
  follow_taskbar: boolean;
  hide_from_capture: boolean;
  widget_config: Record<string, Record<string, unknown>>;
}

/** Sets --widget-opacity, the 0-100 setting as a CSS alpha. */
export function applyOpacity(settings: Settings | null | undefined): void {
  const pct = settings?.opacity ?? 100;
  document.documentElement.style.setProperty("--widget-opacity", String(pct / 100));
}

/** settings.widget_config[id], defaulting to {} when unset. */
export function widgetConfig(
  settings: Settings | null | undefined,
  id: string,
): Record<string, unknown> {
  return settings?.widget_config?.[id] ?? {};
}

/** Latest settings immediately, then live updates on "widgets-changed". */
export function subscribeSettings(onSettings: (s: Settings) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  const refresh = () =>
    invoke<Settings>("get_settings").then((s) => {
      if (!disposed) onSettings(s);
    }).catch(() => {});
  refresh();
  listen("widgets-changed", refresh).then((un) => {
    if (disposed) un();
    else unlisten = un;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export function fmtBytes(bytes: number, digits = 1): string {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1024) return `${(gb / 1024).toFixed(digits)} TB`;
  return `${gb.toFixed(digits)} GB`;
}

// Shared with main.ts's drag-to-reorder handling: widgets that wire their own
// click handlers (e.g. conductor's per-account dials) need to suppress the
// click that follows a drag's pointerup, same as wireFlyoutHover does.
let dragging = false;

export function isDragging(): boolean {
  return dragging;
}

export function setDragging(v: boolean): void {
  dragging = v;
}

/** "2h 14m" until an RFC3339 instant; empty when past or missing. */
export function fmtCountdown(resetsAt: string | null | undefined): string {
  if (!resetsAt) return "";
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const totalMin = Math.ceil(ms / 60_000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}
