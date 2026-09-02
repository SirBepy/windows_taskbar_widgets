import { html, render, type TemplateResult } from "lit-html";
import { widgetById } from "../../widgets/registry";

export interface MonitorOption {
  device_name: string;
  is_primary: boolean;
  width: number;
  height: number;
}

interface Mounted {
  el: HTMLElement;
  dispose: () => void;
}

const mounted = new Map<string, Mounted>();
let laneChromeKey = "";

// ---------- chrome ----------

/** Windows' own numbering: the 2 in `\\.\DISPLAY2` is the "2" its Display settings
 * shows. Falls back to lane order for a device name outside that shape. */
function monitorNumber(m: MonitorOption, i: number): string {
  return /DISPLAY(\d+)$/.exec(m.device_name)?.[1] ?? String(i + 1);
}

function monitorLabel(m: MonitorOption, i: number): string {
  if (m.device_name === "") return "Taskbar";
  const n = `Monitor ${monitorNumber(m, i)}`;
  return m.is_primary ? `${n} (Primary)` : n;
}

// The raw device name stays in the dim suffix: two identical panels differ by nothing
// else, and it is the key `monitor_widgets` is actually written under.
function laneTemplate(m: MonitorOption, i: number, showHead: boolean): TemplateResult {
  return html`
    <div class="wsf-lane">
      ${showHead
        ? html`<div class="wsf-lane-head">
            <i class="ph ph-monitor"></i><b>${monitorLabel(m, i)}</b>
            <span class="wsf-lane-dims">${m.width}x${m.height} · ${m.device_name}</span>
          </div>`
        : ""}
      <div class="wsf-stage">
        <div class="wsf-desktop"></div>
        <div class="wsf-bar">
          <div class="wsf-strip" data-monitor=${m.device_name}></div>
          <div class="wsf-sys">
            <i class="ph ph-wifi-high"></i><i class="ph ph-speaker-high"></i
            ><i class="ph ph-battery-high"></i>
          </div>
        </div>
      </div>
    </div>
  `;
}

// Rebuilt only when the monitor SET changes, never on a tile move: re-rendering the
// chrome would throw away the strip elements the drag is holding mid-gesture.
function syncLaneChrome(container: HTMLElement, monitors: MonitorOption[]): void {
  const key = monitors.map((m) => m.device_name).join(" ");
  if (laneChromeKey === key) return;
  disposeLanes();
  laneChromeKey = key;
  const showHead = monitors.length > 1;
  render(
    html`${monitors.map((m, i) => laneTemplate(m, i, showHead))}`,
    container,
  );
}

function stripFor(container: HTMLElement, deviceName: string): HTMLElement | null {
  return (
    [...container.querySelectorAll<HTMLElement>(".wsf-strip")].find(
      (el) => (el.dataset.monitor ?? "") === deviceName,
    ) ?? null
  );
}

// ---------- tiles ----------

// Keyed by lane, kind and which copy - never by position, or a reorder would remount
// every tile it moved past. The same widget on two monitors is two live tiles, since
// one element cannot sit in two strips.
function tileKeys(monitor: string, ids: string[]): string[] {
  const seen = new Map<string, number>();
  return ids.map((id) => {
    const n = seen.get(id) ?? 0;
    seen.set(id, n + 1);
    return `${monitor} ${id} ${n}`;
  });
}

function mountTile(id: string): Mounted | null {
  const w = widgetById(id);
  if (!w) return null;
  const el = document.createElement("div");
  el.className = "tile";
  el.dataset.widget = id;
  return { el, dispose: w.mountTile(el) };
}

// Tiles are moved, never re-created: remounting on each change would re-run every
// widget's subscriptions and throw away the drag's DOM mid-gesture.
function syncTiles(
  container: HTMLElement,
  monitors: MonitorOption[],
  lanes: Record<string, string[]>,
  selectedId: string | null,
): void {
  const wanted = new Set<string>();
  for (const m of monitors) {
    for (const key of tileKeys(m.device_name, lanes[m.device_name] ?? [])) wanted.add(key);
  }
  for (const [key, m] of [...mounted]) {
    if (wanted.has(key)) continue;
    m.dispose();
    m.el.remove();
    mounted.delete(key);
  }
  for (const monitor of monitors) {
    const strip = stripFor(container, monitor.device_name);
    if (!strip) continue;
    const ids = lanes[monitor.device_name] ?? [];
    const keys = tileKeys(monitor.device_name, ids);
    ids.forEach((id, i) => {
      let m = mounted.get(keys[i]);
      if (!m) {
        const made = mountTile(id);
        if (!made) return;
        mounted.set(keys[i], (m = made));
      }
      m.el.classList.toggle("wsf-selected", id === selectedId);
      if (strip.children[i] !== m.el) strip.insertBefore(m.el, strip.children[i] ?? null);
    });
  }
}

// ---------- public ----------

/** Renders every monitor lane and its tiles into `container`. Safe to call on any
 * change: the chrome rebuild short-circuits unless the monitor set itself changed. */
export function renderLanes(
  container: HTMLElement,
  monitors: MonitorOption[],
  lanes: Record<string, string[]>,
  selectedId: string | null,
): void {
  syncLaneChrome(container, monitors);
  syncTiles(container, monitors, lanes, selectedId);
}

/** Disposes every mounted tile. Hiding a webview doesn't stop its JS, so this must run
 * when the settings page goes away - the same leak that cost 15x idle CPU in the flyout. */
export function disposeLanes(): void {
  for (const m of mounted.values()) {
    m.dispose();
    m.el.remove();
  }
  mounted.clear();
  laneChromeKey = "";
}
