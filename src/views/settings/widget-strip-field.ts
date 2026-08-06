import { html, render, type TemplateResult } from "lit-html";
import { ref } from "lit-html/directives/ref.js";
import { invoke } from "@tauri-apps/api/core";
import { fieldRow } from "../../../vendor/tauri_kit/frontend/settings/fields";
import type { CustomField, Field } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { ConfigField, Settings, setDragging, subscribeSettings, widgetConfig } from "../../shared/widget";
import { isDividerId, makeDividerId } from "../../shared/divider";
import { allWidgetIds, allWidgets, widgetById } from "../../widgets/registry";
import { fetchStatsOnce } from "../../widgets/system-shared";
import { placeAt, removeId } from "./widget-strip-dnd";

const NEW_DIVIDER = "new-divider";
const DRAG_THRESHOLD_PX = 6;
const SLIDE_MS = 190;

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
  refs.strip.addEventListener("pointerdown", onPointerDown);
  refs.palette.addEventListener("pointerdown", onPointerDown);
  root.querySelector(".wsf-reset")!.addEventListener("click", resetLayout);
  // Warms system-shared's snapshot cache so tiles paint real numbers instead of
  // their "…" placeholder, which would also measure a too-narrow drop gap.
  void fetchStatsOnce();
  stopSettings = subscribeSettings((s) => {
    settings = s;
    if (!drag) syncAll();
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
  refs!.palette.replaceChildren(
    ...off.map((w) => chip(w.id, w.name, w.icon)),
    chip(NEW_DIVIDER, "Divider", "ph-minus"),
  );
}

function toKitField(f: ConfigField): Field {
  if (f.type === "select") return { key: f.key, label: f.label, kind: "select", options: f.options ?? [] };
  return { key: f.key, label: f.label, kind: f.type };
}

function configTemplate(): TemplateResult {
  const w = selectedId && !isDividerId(selectedId) ? widgetById(selectedId) : undefined;
  if (!w || !settings) {
    return html`<div class="wsf-config-empty">Click a tile above to configure it.</div>`;
  }
  const fields = w.configFields?.() ?? [];
  return html`
    <div class="wsf-config-head"><i class="ph ph-sliders-horizontal"></i>${w.name}</div>
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

// ---------- drag ----------

interface Drag {
  id: string;
  dropId: string;
  source: HTMLElement;
  fromStrip: boolean;
  started: boolean;
  ox: number;
  oy: number;
  dx: number;
  dy: number;
  w: number;
  h: number;
  clone?: HTMLElement;
  cloneDispose?: () => void;
  slot?: HTMLElement;
}

let drag: Drag | null = null;

/** Animates every tile from where it sat before `mutate` to where it lands after,
 * so the row visibly slides apart around the drop slot instead of jumping. */
function flip(mutate: () => void): void {
  const els = [...refs!.strip.children] as HTMLElement[];
  const before = els.map((el) => el.getBoundingClientRect().left);
  mutate();
  els.forEach((el, i) => {
    const shift = before[i] - el.getBoundingClientRect().left;
    if (!shift) return;
    el.style.transition = "none";
    el.style.transform = `translateX(${shift}px)`;
  });
  requestAnimationFrame(() => {
    for (const el of els) {
      el.style.transition = `transform ${SLIDE_MS}ms cubic-bezier(.2, .8, .3, 1)`;
      el.style.transform = "";
    }
  });
}

/** Width a palette entry will occupy once it's a real tile - a chip is narrower,
 * so measuring the chip would open a gap that doesn't match what lands. */
function measureTile(id: string): number {
  const probe = document.createElement("div");
  probe.className = "tile";
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none";
  refs!.strip.appendChild(probe);
  const stop = widgetById(id)?.mountTile(probe);
  const width = probe.getBoundingClientRect().width;
  stop?.();
  probe.remove();
  return width;
}

function begin(): void {
  const d = drag!;
  const clone = document.createElement("div");
  clone.className = "tile wsf-clone";
  d.cloneDispose = widgetById(d.dropId)?.mountTile(clone);
  const baseW = d.fromStrip ? d.w : measureTile(d.dropId);
  clone.style.width = `${baseW}px`;
  clone.style.height = `${d.fromStrip ? d.h : refs!.strip.getBoundingClientRect().height}px`;
  document.body.appendChild(clone);
  d.clone = clone;
  if (!d.fromStrip) {
    d.dx = baseW / 2;
    d.dy = clone.getBoundingClientRect().height / 2;
  }

  // Read the painted rect, not baseW: the clone's scale(1.04) pick-up affordance
  // makes it wider than its layout box, and a narrower gap reads as a mismatch.
  d.slot = document.createElement("div");
  d.slot.className = "wsf-slot";
  d.slot.style.width = `${clone.getBoundingClientRect().width}px`;

  if (d.fromStrip) {
    flip(() => {
      refs!.strip.insertBefore(d.slot!, d.source);
      d.source.classList.add("wsf-lifted");
    });
  } else {
    d.source.classList.add("wsf-lifted");
  }
  setDragging(true);
}

function hits(el: HTMLElement, e: PointerEvent, padX: number, padY: number): boolean {
  const r = el.getBoundingClientRect();
  return (
    e.clientX >= r.left - padX &&
    e.clientX <= r.right + padX &&
    e.clientY >= r.top - padY &&
    e.clientY <= r.bottom + padY
  );
}

function moveSlot(x: number): void {
  const d = drag!;
  const tiles = ([...refs!.strip.children] as HTMLElement[]).filter(
    (t) => t !== d.slot && !t.classList.contains("wsf-lifted"),
  );
  let target: HTMLElement | null = null;
  for (const t of tiles) {
    const r = t.getBoundingClientRect();
    if (x < r.left + r.width / 2) {
      target = t;
      break;
    }
  }
  if (d.slot!.parentElement && d.slot!.nextElementSibling === target) return;
  if (!target && d.slot!.parentElement && !d.slot!.nextElementSibling) return;
  flip(() => refs!.strip.insertBefore(d.slot!, target));
}

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0 || !refs) return;
  const el = (e.target as HTMLElement).closest<HTMLElement>("[data-widget]");
  if (!el) return;
  const id = el.dataset.widget!;
  const r = el.getBoundingClientRect();
  drag = {
    id,
    dropId: id === NEW_DIVIDER ? makeDividerId() : id,
    source: el,
    fromStrip: el.parentElement === refs.strip,
    started: false,
    ox: e.clientX,
    oy: e.clientY,
    dx: e.clientX - r.left,
    dy: e.clientY - r.top,
    w: r.width,
    h: r.height,
  };
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}

function onPointerMove(e: PointerEvent): void {
  const d = drag;
  if (!d || !refs) return;
  if (!d.started) {
    if (Math.hypot(e.clientX - d.ox, e.clientY - d.oy) < DRAG_THRESHOLD_PX) return;
    d.started = true;
    begin();
  }
  d.clone!.style.left = `${e.clientX - d.dx}px`;
  d.clone!.style.top = `${e.clientY - d.dy}px`;

  const overStrip = hits(refs.strip, e, 40, 24);
  refs.strip.classList.toggle("wsf-drop", overStrip);
  refs.palette.classList.toggle("wsf-drop", !overStrip && hits(refs.palette, e, 0, 20));
  d.clone!.classList.toggle("wsf-removing", d.fromStrip && !overStrip);
  if (overStrip) moveSlot(e.clientX);
  else if (d.slot!.parentElement) flip(() => d.slot!.remove());
}

function onPointerUp(): void {
  window.removeEventListener("pointermove", onPointerMove);
  const d = drag;
  drag = null;
  if (!d) return;
  setDragging(false);

  if (!d.started) {
    if (d.fromStrip) {
      selectedId = d.id;
      syncStrip();
      syncConfig();
    }
    return;
  }

  refs?.strip.classList.remove("wsf-drop");
  refs?.palette.classList.remove("wsf-drop");
  d.cloneDispose?.();
  d.clone?.remove();
  d.source.classList.remove("wsf-lifted");

  const landed = !!d.slot!.parentElement;
  // Counted among the non-lifted children, which is exactly what placeAt expects.
  const index = landed
    ? ([...refs!.strip.children] as HTMLElement[])
        .filter((t) => !t.classList.contains("wsf-lifted"))
        .indexOf(d.slot!)
    : -1;
  d.slot!.remove();

  if (landed) {
    if (!isDividerId(d.dropId)) selectedId = d.dropId;
    void save((s) => ({
      ...s,
      enabled_widgets: placeAt(s.enabled_widgets, d.dropId, index),
      hidden_widgets: removeId(s.hidden_widgets, d.dropId),
    }));
  } else if (d.fromStrip) {
    if (selectedId === d.id) selectedId = null;
    // A divider's uuid id is single-use, so it's dropped rather than remembered.
    void save((s) => ({
      ...s,
      enabled_widgets: removeId(s.enabled_widgets, d.id),
      hidden_widgets: isDividerId(d.id) ? s.hidden_widgets : [...removeId(s.hidden_widgets, d.id), d.id],
    }));
  } else {
    syncAll();
  }
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
