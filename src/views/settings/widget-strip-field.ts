import { html } from "lit-html";
import { ref } from "lit-html/directives/ref.js";
import { invoke } from "@tauri-apps/api/core";
import type { CustomField } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { isInstanceHidden, Settings, subscribeSettings } from "../../shared/widget";
import { isDividerId } from "../../shared/divider";
import { dividerWidget } from "../../widgets/divider";
import { allWidgetIds, allWidgets } from "../../widgets/registry";
import { fetchStatsOnce } from "../../widgets/system-shared";
import { renderConfig } from "./widget-strip-config";
import { disposeLanes, type MonitorOption, renderLanes } from "./widget-strip-lanes";
import { insertAt, removeFirst } from "./widget-strip-dnd";
import { isStripDragActive, NEW_DIVIDER, wireStripDrag } from "./widget-strip-drag";

interface Refs {
  root: HTMLElement;
  lanes: HTMLElement;
  palette: HTMLElement;
  config: HTMLElement;
}

let refs: Refs | null = null;
let settings: Settings | null = null;
let selectedId: string | null = null;
let stopSettings: (() => void) | null = null;
let monitors: MonitorOption[] = [];
/** Widget ids per monitor device name, the lanes UI's own copy of `monitor_widgets`.
 * Updated optimistically on a drop so the strip does not wait on the IPC round trip. */
let lanes: Record<string, string[]> = {};

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

function teardown(): void {
  stopSettings?.();
  stopSettings = null;
  disposeLanes();
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
  lanes = {};
  for (const m of monitors) {
    const lane = saved[m.device_name] ?? (m.is_primary ? saved[""] : undefined);
    lanes[m.device_name] = (lane ?? [])
      .filter((si) => !isInstanceHidden(settings, si))
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
  renderLanes(refs.lanes, monitors, lanes, selectedId);
  syncPalette();
  syncConfig();
}

function syncConfig(): void {
  renderConfig(refs!.config, selectedId, settings, (next) => void save(next));
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
  renderLanes(refs!.lanes, monitors, lanes, selectedId);
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
  renderLanes(refs.lanes, monitors, lanes, selectedId);
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
