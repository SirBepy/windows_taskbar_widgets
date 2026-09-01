import { html, render, type TemplateResult } from "lit-html";
import { fieldRow } from "../../../vendor/tauri_kit/frontend/settings/fields";
import type { Field } from "../../../vendor/tauri_kit/frontend/settings/schema";
import {
  ConfigField,
  Placement,
  Settings,
  TaskbarWidget,
  overlayDims,
  placementOf,
  widgetConfig,
} from "../../shared/widget";
import { isDividerId } from "../../shared/divider";
import { widgetById } from "../../widgets/registry";

type OverlayPlacement = Extract<Placement, { kind: "overlay" }>;

/** widget-strip-field.ts's own optimistic-update-then-invoke save, passed in rather
 * than duplicated: one writer keeps the lanes and this panel from racing each other. */
export type SaveConfig = (next: (s: Settings) => Settings) => void;

const DEFAULT_X = 40;
const DEFAULT_Y = 40;

// Re-set on every renderConfig call, so a template's handlers always close over the
// settings that were current when it was built.
let settings: Settings | null = null;
let save: SaveConfig = () => {};

function setPlacementKind(w: TaskbarWidget, kind: "strip" | "overlay"): void {
  save((s) => {
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
  save((s) => {
    const p = placementOf(s, id);
    if (p.kind !== "overlay") return s;
    return { ...s, widget_placement: { ...s.widget_placement, [id]: { ...p, opacity } } };
  });
}

function resetOverlayPosition(w: TaskbarWidget): void {
  save((s) => {
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

function toKitField(f: ConfigField): Field {
  if (f.type === "select") return { key: f.key, label: f.label, kind: "select", options: f.options ?? [] };
  return { key: f.key, label: f.label, kind: f.type };
}

function overlayRowsTemplate(w: TaskbarWidget, p: OverlayPlacement): TemplateResult {
  const inherited = p.opacity === null || p.opacity === undefined;
  return html`
    <div class="kit-row">
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
        <label class="kit-toggle">
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
        </label>
        <span class="wsf-inherit-label">Inherit</span>
      </span>
    </div>
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

function configTemplate(selectedId: string | null): TemplateResult {
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
            save((s) => ({
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

/** Renders the per-widget config panel for the selected tile into `host`. */
export function renderConfig(
  host: HTMLElement,
  selectedId: string | null,
  current: Settings | null,
  saveConfig: SaveConfig,
): void {
  settings = current;
  save = saveConfig;
  render(configTemplate(selectedId), host);
}
