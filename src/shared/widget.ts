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
  /** Phosphor class for the settings palette chip, e.g. "ph-cpu". */
  icon?: string;
  /** Fixed at declare-time, sized for max content; see CLAUDE.md's fixed-size rule. Omit for tile-only widgets. */
  flyout?: { widthCss: number; heightCss: number };
  /** Recomputed on every hover-open (e.g. account-count-dependent height), overriding `flyout` for that
   * open only - the size still never changes while the flyout is already showing. Falls back to `flyout`. */
  flyoutDims?(): { widthCss: number; heightCss: number };
  /** Default AND minimum size when placed as a floating overlay; the user may resize up from it. */
  overlay?: { widthCss: number; heightCss: number };
  /** Render the always-visible strip tile. Returns a cleanup fn. */
  mountTile(root: HTMLElement): () => void;
  mountFlyout?(root: HTMLElement): () => void;
  /** Render the floating overlay. Falls back to mountFlyout, then mountTile, when absent. */
  mountOverlay?(root: HTMLElement): () => void;
  /** Extra entries appended to this tile's native context menu. */
  menuItems?: () => { id: string; label: string }[];
  onMenuAction?: (id: string) => void;
  /** Rows rendered in the settings screen's per-widget config accordion. */
  configFields?: () => ConfigField[];
}

/** Mirrors settings.rs's OverlaySpec. x/y are CSS px within the monitor's work area. */
export interface OverlaySpec {
  monitor: string;
  x: number;
  y: number;
  w?: number | null;
  h?: number | null;
  opacity?: number | null;
}

export type Placement = { kind: "strip" } | ({ kind: "overlay" } & OverlaySpec);

/** Mirrors monitor_widgets.rs's StripInstance: one placed copy of a widget. */
export interface StripInstance {
  instance_id: string;
  widget_id: string;
}

export interface Settings {
  left_margin: number;
  enabled_widgets: string[];
  hidden_widgets: string[];
  stats_poll_seconds: number;
  opacity: number;
  follow_taskbar: boolean;
  hide_from_capture: boolean;
  taskbar_monitor: string;
  widget_config: Record<string, Record<string, unknown>>;
  widget_placement: Record<string, Placement>;
  dividers_migrated: boolean;
  // Keyed by monitor device name ("" = primary); mirrors settings.rs's MonitorWidgets.
  monitor_widgets: Record<string, StripInstance[]>;
}

/** The first instance placing `widgetId`, across every lane. Only for callers that know a
 * kind and nothing else (an overlay window); the strip uses `instancesForMonitor`. */
export function instanceIdFor(settings: Settings | null | undefined, widgetId: string): string {
  for (const instances of Object.values(settings?.monitor_widgets ?? {})) {
    const match = instances.find((si) => si.widget_id === widgetId);
    if (match) return match.instance_id;
  }
  return `${widgetId}#1`;
}

/** An absent entry means the strip, which is what keeps existing installs migration-free. */
export function placementOf(
  settings: Settings | null | undefined,
  id: string,
): Placement {
  return settings?.widget_placement?.[id] ?? { kind: "strip" };
}

function isOverlayPlaced(
  settings: Settings | null | undefined,
  id: string,
): boolean {
  return placementOf(settings, id).kind === "overlay";
}

/** Mirrors settings.rs's `is_active`: hidden_widgets carries instance ids and bare kinds at once. */
export function isInstanceHidden(
  settings: Settings | null | undefined,
  si: StripInstance,
): boolean {
  const hidden = settings?.hidden_widgets ?? [];
  return hidden.includes(si.instance_id) || hidden.includes(si.widget_id);
}

export function stripTileInstances(
  instances: StripInstance[],
  settings: Settings | null | undefined,
): StripInstance[] {
  return instances.filter(
    (si) => !isInstanceHidden(settings, si) && !isOverlayPlaced(settings, si.widget_id),
  );
}

/** True when the strip must tear down/remount: tile membership or order changed.
 * Config-only changes (e.g. show_temp) reach mounted tiles via subscribeSettings instead. */
export function stripNeedsRemount(prev: StripInstance[], next: StripInstance[]): boolean {
  return prev.length !== next.length || prev.some((si, i) => si.instance_id !== next[i].instance_id);
}

/** One strip window's share of `ids`, in the lane's own order. Instances, not kinds: a lane
 * may hold two copies of one kind and the tile menu must act on the one right-clicked. An
 * unresolved or empty lane returns `ids` whole - a blank strip is worse than a duplicated one. */
export function instancesForMonitor(
  ids: string[],
  settings: Settings | null | undefined,
  monitorKey: string | null,
): StripInstance[] {
  const whole = () =>
    ids.map((id) => ({ instance_id: instanceIdFor(settings, id), widget_id: id }));
  if (monitorKey === null) return whole();
  const instances = settings?.monitor_widgets?.[monitorKey];
  if (!instances || instances.length === 0) return whole();
  return instances.filter((si) => ids.includes(si.widget_id));
}

/** Registry widgets a user's settings.json has never seen, minus any they already hid. */
export function newWidgetIds(allIds: string[], enabled: string[], hidden: string[]): string[] {
  return allIds.filter((id) => !enabled.includes(id) && !hidden.includes(id));
}

/** Declared overlay size, falling back to the flyout's. Null means "cannot be an overlay". */
export function overlayDims(
  w: TaskbarWidget,
): { widthCss: number; heightCss: number } | null {
  return w.overlay ?? w.flyout ?? null;
}

/** Renderer falls back where size does not; see decision 1 in docs/widget-host-design.md. */
export function overlayRenderer(w: TaskbarWidget): (root: HTMLElement) => () => void {
  return (w.mountOverlay ?? w.mountFlyout ?? w.mountTile).bind(w);
}

/** Sets --widget-opacity, the 0-100 setting as a CSS alpha. `override` is an overlay's own value. */
export function applyOpacity(
  settings: Settings | null | undefined,
  override?: number | null,
): void {
  const pct = override ?? settings?.opacity ?? 100;
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

/** "5:30" by default; `padMinutes` zero-pads mm, `hours` adds an "h:mm:ss" branch
 * once the duration reaches an hour (otherwise minutes keep counting past 59). */
export function fmtClock(ms: number, opts?: { padMinutes?: boolean; hours?: boolean }): string {
  const totalSec = Math.max(0, Math.round(ms / 1000));
  const showHours = !!opts?.hours && totalSec >= 3600;
  const h = Math.floor(totalSec / 3600);
  const m = showHours ? Math.floor((totalSec % 3600) / 60) : Math.floor(totalSec / 60);
  const s = totalSec % 60;
  const mm = opts?.padMinutes ? String(m).padStart(2, "0") : String(m);
  const ss = String(s).padStart(2, "0");
  return showHours ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
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

/** Ms remaining until an RFC3339 instant; null when missing, unparseable, or past. */
export function msUntil(resetsAt: string | null | undefined): number | null {
  if (!resetsAt) return null;
  const ms = new Date(resetsAt).getTime() - Date.now();
  return Number.isFinite(ms) && ms > 0 ? ms : null;
}

/** "2h 14m" until an RFC3339 instant; empty when past or missing. */
export function fmtCountdown(resetsAt: string | null | undefined): string {
  const ms = msUntil(resetsAt);
  if (ms === null) return "";
  const totalMin = Math.ceil(ms / 60_000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}
