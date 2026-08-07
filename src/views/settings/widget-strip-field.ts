import { html, render, type TemplateResult } from "lit-html";
import { ref } from "lit-html/directives/ref.js";
import { invoke } from "@tauri-apps/api/core";
import { fieldRow } from "../../../vendor/tauri_kit/frontend/settings/fields";
import type { CustomField, Field } from "../../../vendor/tauri_kit/frontend/settings/schema";
import {
  ConfigField,
  Placement,
  Settings,
  TaskbarWidget,
  overlayDims,
  placementOf,
  subscribeSettings,
  widgetConfig,
} from "../../shared/widget";
import { isDividerId } from "../../shared/divider";
import { dividerWidget } from "../../widgets/divider";
import { allWidgetIds, allWidgets, widgetById } from "../../widgets/registry";
import { fetchStatsOnce } from "../../widgets/system-shared";
import { placeAt, removeId } from "./widget-strip-dnd";
import { isStripDragActive, NEW_DIVIDER, wireStripDrag } from "./widget-strip-drag";

interface Mounted {
  el: HTMLElement;
  dispose: () => void;
}

interface Refs {
  root: HTMLElement;
  strip: HTMLElement;
  palette: HTMLElement;
  config: HTMLElement;
}

let refs: Refs | null = null;
let settings: Settings | null = null;
let selectedId: string | null = null;
let stopSettings: (() => void) | null = null;
const mounted = new Map<string, Mounted>();

const SKELETON = `
  <div class="wsf-stage">
    <div class="wsf-desktop"></div>
    <div class="wsf-bar">
      <div class="wsf-strip"></div>
      <div class="wsf-sys">
        <i class="ph ph-wifi-high"></i><i class="ph ph-speaker-high"></i><i class="ph ph-battery-high"></i>
      </div>
    </div>
  </div>
  <p class="wsf-hint">Drag tiles to reorder. Drag one down to Available to turn it off.</p>
  <div class="kit-section-title">Available</div>
  <div class="wsf-palette"></div>
  <div class="wsf-config"></div>
  <button type="button" class="kit-btn-secondary wsf-reset">
    <i class="ph ph-arrow-counter-clockwise"></i> Reset widget layout
  </button>
`;

// ---------- lifecycle ----------

// Hiding a webview doesn't stop its JS, so every mounted tile must be disposed
// when this page goes away - the same leak that cost 15x idle CPU in the flyout.
function teardown(): void {
  stopSettings?.();
  stopSettings = null;
  for (const m of mounted.values()) m.dispose();
  mounted.clear();
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
    strip: root.querySelector<HTMLElement>(".wsf-strip")!,
    palette: root.querySelector<HTMLElement>(".wsf-palette")!,
    config: root.querySelector<HTMLElement>(".wsf-config")!,
  };
  wireStripDrag(refs.strip, refs.palette, {
    onSelect: onStripSelect,
    onDrop: onStripDrop,
    onRemove: onStripRemove,
    onCancel: syncAll,
  });
  root.querySelector(".wsf-reset")!.addEventListener("click", resetLayout);
  // Warms system-shared's snapshot cache so tiles paint real numbers instead of
  // their "…" placeholder, which would also measure a too-narrow drop gap.
  void fetchStatsOnce();
  stopSettings = subscribeSettings((s) => {
    settings = s;
    if (!isStripDragActive()) syncAll();
  });
}

// ---------- rendering ----------

function syncAll(): void {
  if (!refs) return;
  syncStrip();
  syncPalette();
  syncConfig();
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
function syncStrip(): void {
  const order = settings?.enabled_widgets ?? [];
  for (const [id, m] of [...mounted]) {
    if (order.includes(id)) continue;
    m.dispose();
    m.el.remove();
    mounted.delete(id);
  }
  order.forEach((id, i) => {
    let m = mounted.get(id);
    if (!m) {
      const made = mountTile(id);
      if (!made) return;
      mounted.set(id, (m = made));
    }
    m.el.classList.toggle("wsf-selected", id === selectedId);
    if (refs!.strip.children[i] !== m.el) {
      refs!.strip.insertBefore(m.el, refs!.strip.children[i] ?? null);
    }
  });
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
  const order = settings?.enabled_widgets ?? [];
  const off = allWidgets().filter((w) => !order.includes(w.id));
  const divider = dividerWidget(NEW_DIVIDER);
  refs!.palette.replaceChildren(
    ...off.map((w) => chip(w.id, w.name, w.icon)),
    chip(NEW_DIVIDER, divider.name, divider.icon),
  );
}

function toKitField(f: ConfigField): Field {
  if (f.type === "select") return { key: f.key, label: f.label, kind: "select", options: f.options ?? [] };
  return { key: f.key, label: f.label, kind: f.type };
}

// ---------- placement ----------

type OverlayPlacement = Extract<Placement, { kind: "overlay" }>;

const DEFAULT_X = 40;
const DEFAULT_Y = 40;

function setPlacementKind(w: TaskbarWidget, kind: "strip" | "overlay"): void {
  void save((s) => {
    if (kind === "strip") {
      return { ...s, widget_placement: { ...s.widget_placement, [w.id]: { kind: "strip" } } };
    }
    const dims = overlayDims(w);
    if (!dims) return s;
    return {
      ...s,
      widget_placement: {
        ...s.widget_placement,
        [w.id]: {
          kind: "overlay",
          monitor: "",
          x: DEFAULT_X,
          y: DEFAULT_Y,
          w: dims.widthCss,
          h: dims.heightCss,
          opacity: null,
        },
      },
    };
  });
}

function setOverlayOpacity(id: string, opacity: number | null): void {
  void save((s) => {
    const p = placementOf(s, id);
    if (p.kind !== "overlay") return s;
    return { ...s, widget_placement: { ...s.widget_placement, [id]: { ...p, opacity } } };
  });
}

function resetOverlayPosition(w: TaskbarWidget): void {
  void save((s) => {
    const p = placementOf(s, w.id);
    if (p.kind !== "overlay") return s;
    const dims = overlayDims(w);
    return {
      ...s,
      widget_placement: {
        ...s.widget_placement,
        [w.id]: {
          ...p,
          x: DEFAULT_X,
          y: DEFAULT_Y,
          monitor: "",
          w: dims?.widthCss ?? p.w,
          h: dims?.heightCss ?? p.h,
        },
      },
    };
  });
}

function overlayRowsTemplate(w: TaskbarWidget, p: OverlayPlacement): TemplateResult {
  const inherited = p.opacity === null || p.opacity === undefined;
  return html`
    <label class="kit-row">
      <span class="kit-row-label">Overlay opacity</span>
      <span class="wsf-opacity-control">
        <input
          type="range"
          class="kit-range"
          min="10"
          max="100"
          step="1"
          .value=${String(p.opacity ?? settings?.opacity ?? 100)}
          ?disabled=${inherited}
          @change=${(e: Event) =>
            setOverlayOpacity(w.id, parseInt((e.target as HTMLInputElement).value, 10))}
        />
        <span class="kit-toggle">
          <input
            type="checkbox"
            .checked=${inherited}
            @change=${(e: Event) =>
              setOverlayOpacity(
                w.id,
                (e.target as HTMLInputElement).checked ? null : settings?.opacity ?? 100,
              )}
          />
          <span class="kit-toggle-track"></span>
        </span>
        <span class="wsf-inherit-label">Inherit</span>
      </span>
    </label>
    <div class="kit-row">
      <span class="kit-row-label">Position</span>
      <button type="button" class="kit-btn-secondary" @click=${() => resetOverlayPosition(w)}>
        <i class="ph ph-arrow-counter-clockwise"></i> Reset position
      </button>
    </div>
  `;
}

function placementTemplate(w: TaskbarWidget): TemplateResult {
  const dims = overlayDims(w);
  const placement = placementOf(settings, w.id);
  return html`
    <label class="kit-row">
      <span class="kit-row-label">Placement</span>
      <select
        class="kit-select"
        ?disabled=${!dims}
        @change=${(e: Event) =>
          setPlacementKind(w, (e.target as HTMLSelectElement).value as "strip" | "overlay")}
      >
        <option value="strip" ?selected=${placement.kind !== "overlay"}>Taskbar</option>
        <option value="overlay" ?selected=${placement.kind === "overlay"}>Floating</option>
      </select>
    </label>
    ${!dims
      ? html`<div class="wsf-config-empty">No floating size declared for this widget.</div>`
      : ""}
    ${placement.kind === "overlay" ? overlayRowsTemplate(w, placement) : ""}
  `;
}

function configTemplate(): TemplateResult {
  const w = selectedId && !isDividerId(selectedId) ? widgetById(selectedId) : undefined;
  if (!w || !settings) {
    return html`<div class="wsf-config-empty">Click a tile above to configure it.</div>`;
  }
  const fields = w.configFields?.() ?? [];
  return html`
    <div class="wsf-config-head"><i class="ph ph-sliders-horizontal"></i>${w.name}</div>
    ${placementTemplate(w)}
    ${fields.length
      ? fields.map((f) =>
          fieldRow(toKitField(f), widgetConfig(settings, w.id)[f.key] ?? f.default, (v) => {
            void save((s) => ({
              ...s,
              widget_config: {
                ...s.widget_config,
                [w.id]: { ...widgetConfig(s, w.id), [f.key]: v },
              },
            }));
          }),
        )
      : html`<div class="wsf-config-empty">${w.name} has no options.</div>`}
  `;
}

function syncConfig(): void {
  render(configTemplate(), refs!.config);
}

// ---------- persistence ----------

async function save(next: (s: Settings) => Settings): Promise<void> {
  const base = settings ?? (await invoke<Settings>("get_settings"));
  const updated = next(base);
  settings = updated;
  syncAll();
  await invoke("save_settings", { settings: updated }).catch(() => {});
}

function resetLayout(): void {
  selectedId = null;
  void save((s) => ({ ...s, enabled_widgets: allWidgetIds(), hidden_widgets: [], widget_config: {} }));
}

// ---------- drag callbacks ----------

function onStripSelect(id: string): void {
  selectedId = id;
  syncStrip();
  syncConfig();
}

function onStripDrop(dropId: string, index: number): void {
  if (!isDividerId(dropId)) selectedId = dropId;
  void save((s) => ({
    ...s,
    enabled_widgets: placeAt(s.enabled_widgets, dropId, index),
    hidden_widgets: removeId(s.hidden_widgets, dropId),
  }));
}

function onStripRemove(id: string): void {
  if (selectedId === id) selectedId = null;
  // A divider's uuid id is single-use, so it's dropped rather than remembered.
  void save((s) => ({
    ...s,
    enabled_widgets: removeId(s.enabled_widgets, id),
    hidden_widgets: isDividerId(id) ? s.hidden_widgets : [...removeId(s.hidden_widgets, id), id],
  }));
}

// ---------- public ----------

/** Selects and reveals a tile; called by the "Edit this widget" deep link. */
export function selectWidgetInStrip(id: string): void {
  selectedId = id;
  if (!refs) return;
  syncStrip();
  syncConfig();
  requestAnimationFrame(() =>
    mounted.get(id)?.el.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" }),
  );
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
