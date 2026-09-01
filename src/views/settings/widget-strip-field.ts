import { html, render, type TemplateResult } from "lit-html";
import { ref } from "lit-html/directives/ref.js";
import { invoke } from "@tauri-apps/api/core";
import type { CustomField } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { Settings, subscribeSettings } from "../../shared/widget";
import { isDividerId } from "../../shared/divider";
import { dividerWidget } from "../../widgets/divider";
import { allWidgetIds, allWidgets, widgetById } from "../../widgets/registry";
import { fetchStatsOnce } from "../../widgets/system-shared";
import { renderConfig } from "./widget-strip-config";
import { insertAt, removeFirst } from "./widget-strip-dnd";
import { isStripDragActive, NEW_DIVIDER, wireStripDrag } from "./widget-strip-drag";

interface Mounted {
  el: HTMLElement;
  dispose: () => void;
}

interface Refs {
  root: HTMLElement;
  lanes: HTMLElement;
  palette: HTMLElement;
  config: HTMLElement;
}

interface MonitorOption {
  device_name: string;
  is_primary: boolean;
  width: number;
  height: number;
}

let refs: Refs | null = null;
let settings: Settings | null = null;
let selectedId: string | null = null;
let stopSettings: (() => void) | null = null;
let monitors: MonitorOption[] = [];
let laneChromeKey = "";
/** Widget ids per monitor device name, the lanes UI's own copy of `monitor_widgets`.
 * Updated optimistically on a drop so the strip does not wait on the IPC round trip. */
let lanes: Record<string, string[]> = {};
const mounted = new Map<string, Mounted>();

const SKELETON = `
  <div class="wsf-lanes"></div>
  <p class="wsf-hint">Drag tiles to reorder. Drag one down to Available to turn it off.</p>
  <div class="kit-section-title">Available</div>
  <div class="wsf-palette"></div>
  <div class="wsf-config"></div>
  <button type="button" class="kit-btn-secondary wsf-reset">
    <i class="ph ph-arrow-counter-clockwise"></i> Reset widget layout
  </button>
`;

// A monitor with no taskbar detected still has to get a lane, or its widgets would be
// unreachable; "" is also what a single-monitor install's saved lane is keyed by.
const FALLBACK_MONITOR: MonitorOption = {
  device_name: "",
  is_primary: true,
  width: 0,
  height: 0,
};

// ---------- lifecycle ----------

// Hiding a webview doesn't stop its JS, so every mounted tile must be disposed
// when this page goes away - the same leak that cost 15x idle CPU in the flyout.
function teardown(): void {
  stopSettings?.();
  stopSettings = null;
  for (const m of mounted.values()) m.dispose();
  mounted.clear();
  laneChromeKey = "";
  refs = null;
}

function attach(node?: Element): void {
  if (!node) {
    teardown();
    return;
  }
  const root = node as HTMLElement;
  if (refs?.root === root) return;
  teardown();
  root.innerHTML = SKELETON;
  refs = {
    root,
    lanes: root.querySelector<HTMLElement>(".wsf-lanes")!,
    palette: root.querySelector<HTMLElement>(".wsf-palette")!,
    config: root.querySelector<HTMLElement>(".wsf-config")!,
  };
  wireStripDrag(refs.lanes, refs.palette, {
    onSelect: onStripSelect,
    onDrop: onStripDrop,
    onRemove: onStripRemove,
    onCancel: syncAll,
  });
  root.querySelector(".wsf-reset")!.addEventListener("click", resetLayout);
  // Warms system-shared's snapshot cache so tiles paint real numbers instead of
  // their "…" placeholder, which would also measure a too-narrow drop gap.
  void fetchStatsOnce();
  void loadMonitors();
  stopSettings = subscribeSettings((s) => {
    settings = s;
    if (isStripDragActive()) return;
    seedLanes();
    syncAll();
  });
}

async function loadMonitors(): Promise<void> {
  const list = await invoke<MonitorOption[]>("list_taskbar_monitors").catch(() => []);
  monitors = list.length > 0 ? list : [FALLBACK_MONITOR];
  seedLanes();
  syncAll();
}

// ---------- lane state ----------

/** Reads each live monitor's lane out of settings. A primary with no lane of its own
 * falls back to `""`, the migration default every install still has until the first
 * drop here writes a concrete device name. Hidden instances are dropped, so a widget
 * the tile menu hid stays under Available instead of a drag writing it back visible. */
function seedLanes(): void {
  const saved = settings?.monitor_widgets ?? {};
  const hidden = settings?.hidden_widgets ?? [];
  lanes = {};
  for (const m of monitors) {
    const lane = saved[m.device_name] ?? (m.is_primary ? saved[""] : undefined);
    lanes[m.device_name] = (lane ?? [])
      .filter((si) => !hidden.includes(si.instance_id) && !hidden.includes(si.widget_id))
      .map((si) => si.widget_id);
  }
}

function inAnyLane(id: string): boolean {
  return Object.values(lanes).some((ids) => ids.includes(id));
}

async function commitLanes(next: Record<string, string[]>): Promise<void> {
  lanes = next;
  syncAll();
  await invoke("set_lanes", { lanes: next }).catch(() => {});
}

// ---------- rendering ----------

function syncAll(): void {
  if (!refs) return;
  syncLaneChrome();
  syncLanes();
  syncPalette();
  syncConfig();
}

function syncConfig(): void {
  renderConfig(refs!.config, selectedId, settings, (next) => void save(next));
}

function monitorLabel(m: MonitorOption): string {
  if (m.device_name === "") return "Taskbar";
  return m.is_primary ? `Primary - ${m.device_name}` : m.device_name;
}

function laneTemplate(m: MonitorOption): TemplateResult {
  return html`
    <div class="wsf-lane">
      ${monitors.length > 1
        ? html`<div class="wsf-lane-head">
            <i class="ph ph-monitor"></i><b>${monitorLabel(m)}</b>
            <span class="wsf-lane-dims">${m.width}x${m.height}</span>
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
function syncLaneChrome(): void {
  const key = monitors.map((m) => m.device_name).join(" ");
  if (laneChromeKey === key) return;
  laneChromeKey = key;
  for (const m of mounted.values()) {
    m.dispose();
    m.el.remove();
  }
  mounted.clear();
  render(
    html`${monitors.map((m) => laneTemplate(m))}`,
    refs!.lanes,
  );
}

function stripFor(deviceName: string): HTMLElement | null {
  return [...refs!.lanes.querySelectorAll<HTMLElement>(".wsf-strip")].find(
    (el) => (el.dataset.monitor ?? "") === deviceName,
  ) ?? null;
}

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
function syncLanes(): void {
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
    const strip = stripFor(monitor.device_name);
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

function chip(id: string, name: string, icon?: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "wsf-chip";
  el.dataset.widget = id;
  el.innerHTML = `<i class="ph ${icon ?? "ph-squares-four"}"></i><span></span>`;
  el.querySelector("span")!.textContent = name;
  return el;
}

function syncPalette(): void {
  const off = allWidgets().filter((w) => !inAnyLane(w.id));
  const divider = dividerWidget(NEW_DIVIDER);
  refs!.palette.replaceChildren(
    ...off.map((w) => chip(w.id, w.name, w.icon)),
    chip(NEW_DIVIDER, divider.name, divider.icon),
  );
}

// ---------- persistence ----------

async function save(next: (s: Settings) => Settings): Promise<void> {
  const base = settings ?? (await invoke<Settings>("get_settings"));
  const updated = next(base);
  settings = updated;
  seedLanes();
  syncAll();
  await invoke("save_settings", { settings: updated }).catch(() => {});
}

// monitor_widgets is cleared too, so persist's ensure_instances rebuilds one lane
// holding everything - a reset that left a secondary lane populated would not read
// as a reset at all.
function resetLayout(): void {
  selectedId = null;
  void save((s) => ({
    ...s,
    enabled_widgets: allWidgetIds(),
    hidden_widgets: [],
    widget_config: {},
    monitor_widgets: {},
  }));
}

// ---------- drag callbacks ----------

function onStripSelect(id: string): void {
  selectedId = id;
  syncLanes();
  syncConfig();
}

function onStripDrop(dropId: string, from: string | null, to: string, index: number): void {
  if (!isDividerId(dropId)) selectedId = dropId;
  const next = { ...lanes };
  if (from !== null) next[from] = removeFirst(next[from] ?? [], dropId);
  next[to] = insertAt(next[to] ?? [], dropId, index);
  void commitLanes(next);
}

function onStripRemove(id: string, monitor: string): void {
  if (selectedId === id) selectedId = null;
  void commitLanes({ ...lanes, [monitor]: removeFirst(lanes[monitor] ?? [], id) });
}

// ---------- public ----------

/** Selects and reveals a tile; called by the "Edit this widget" deep link. */
export function selectWidgetInStrip(id: string): void {
  selectedId = id;
  if (!refs) return;
  syncLanes();
  syncConfig();
  requestAnimationFrame(() => {
    const tile = refs?.lanes.querySelector(`.wsf-strip [data-widget="${CSS.escape(id)}"]`);
    tile?.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
  });
}

export function widgetStripField(): CustomField {
  return {
    key: "enabled_widgets",
    label: "Widgets",
    kind: "custom",
    // Stable callback on purpose: a fresh arrow each render would make lit-html
    // tear the whole strip down and remount every tile.
    render: () => html`<div class="wsf" ${ref(attach)}></div>`,
  };
}
