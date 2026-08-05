import { html, nothing, type TemplateResult } from "lit-html";
import { ref } from "lit-html/directives/ref.js";
import { repeat } from "lit-html/directives/repeat.js";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { fieldRow } from "../../../vendor/tauri_kit/frontend/settings/fields";
import type { CustomField, Field } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { ConfigField, Settings, TaskbarWidget, widgetConfig } from "../../shared/widget";
import { wireDragReorder } from "../../shared/drag-reorder";
import { allWidgets } from "../../widgets/registry";

// Spans enabled_widgets/hidden_widgets/widget_config, so the kit's one-key
// onChange can't do it atomically: every mutation reads fresh, saves directly,
// then emits "settings-reset" (the kit's only public re-render hook) so its
// own `current` cache never goes stale for the other two keys.
let cache: Settings | null = null;
let fetchStarted = false;
let expandedId: string | null = null;

function repaint(): void {
  void emit("settings-reset");
}

function ensureLoaded(): void {
  if (fetchStarted) return;
  fetchStarted = true;
  invoke<Settings>("get_settings")
    .then((s) => {
      cache = s;
      repaint();
    })
    .catch(() => {});
}

async function mutate(next: (s: Settings) => Settings): Promise<void> {
  const base = cache ?? (await invoke<Settings>("get_settings"));
  const updated = next(base);
  cache = updated;
  await invoke("save_settings", { settings: updated }).catch(() => {});
  repaint();
}

function toggleWidget(s: Settings, id: string, on: boolean): Settings {
  const enabled = s.enabled_widgets.filter((w) => w !== id);
  const hidden = s.hidden_widgets.filter((w) => w !== id);
  if (on) enabled.push(id);
  else hidden.push(id);
  return { ...s, enabled_widgets: enabled, hidden_widgets: hidden };
}

function setWidgetConfigValue(s: Settings, widgetId: string, key: string, value: unknown): Settings {
  const cfg = { ...widgetConfig(s, widgetId), [key]: value };
  return { ...s, widget_config: { ...s.widget_config, [widgetId]: cfg } };
}

function toKitField(f: ConfigField): Field {
  if (f.type === "select") return { key: f.key, label: f.label, kind: "select", options: f.options ?? [] };
  return { key: f.key, label: f.label, kind: f.type };
}

/** Scrolls a widget's row into view; called by the "Edit this widget" deep link. */
export function expandWidgetInList(id: string): void {
  expandedId = id;
  repaint();
  requestAnimationFrame(() =>
    document.getElementById(`widget-${id}`)?.scrollIntoView({ behavior: "smooth", block: "center" }),
  );
}

function widgetRow(w: TaskbarWidget, s: Settings, isEnabled: boolean): TemplateResult {
  const isExpanded = expandedId === w.id;
  const fields = w.configFields?.() ?? [];
  return html`
    <div
      class="widget-row"
      id="widget-${w.id}"
      data-widget=${w.id}
      ${isEnabled
        ? ref((el) => {
            if (!el) return;
            const container = (el as HTMLElement).closest<HTMLElement>(".widget-enabled-list");
            if (container) {
              wireDragReorder(container, el as HTMLElement, {
                onReorder: (order) => void mutate((cur) => ({ ...cur, enabled_widgets: order })),
              });
            }
          })
        : nothing}
    >
      <div class="widget-row-head">
        ${isEnabled
          ? html`<i class="ph ph-dots-six-vertical widget-drag-handle"></i>`
          : html`<span class="widget-drag-spacer"></span>`}
        <label class="kit-toggle widget-row-toggle">
          <input
            type="checkbox"
            .checked=${isEnabled}
            @change=${(e: Event) =>
              void mutate((cur) => toggleWidget(cur, w.id, (e.target as HTMLInputElement).checked))}
          />
          <span class="kit-toggle-track"></span>
        </label>
        <span class="kit-row-label widget-row-name">${w.name}</span>
        ${fields.length
          ? html`
              <button
                type="button"
                class="kit-btn-secondary widget-row-edit"
                @click=${() => {
                  expandedId = isExpanded ? null : w.id;
                  repaint();
                }}
              >
                ${isExpanded ? "Hide" : "Edit"}
              </button>
            `
          : null}
      </div>
      ${isExpanded && fields.length
        ? html`
            <div class="widget-row-config">
              ${fields.map((f) =>
                fieldRow(
                  toKitField(f),
                  widgetConfig(s, w.id)[f.key] ?? f.default,
                  (v) => void mutate((cur) => setWidgetConfigValue(cur, w.id, f.key, v)),
                ),
              )}
            </div>
          `
        : null}
    </div>
  `;
}

export function widgetListField(): CustomField {
  return {
    key: "enabled_widgets",
    label: "Widgets",
    kind: "custom",
    render(): TemplateResult {
      ensureLoaded();
      if (!cache) return html`<div class="kit-row">Loading…</div>`;
      const s = cache;
      const order = s.enabled_widgets;
      const enabled = order.map((id) => allWidgets().find((w) => w.id === id)).filter((w): w is TaskbarWidget => !!w);
      const disabled = allWidgets().filter((w) => !order.includes(w.id));
      return html`
        <div class="widget-enabled-list">
          ${repeat(enabled, (w) => w.id, (w) => widgetRow(w, s, true))}
        </div>
        ${disabled.length
          ? html`
              <div class="kit-section-title widget-list-divider">Available</div>
              <div class="widget-disabled-list">
                ${repeat(disabled, (w) => w.id, (w) => widgetRow(w, s, false))}
              </div>
            `
          : null}
      `;
    },
  };
}
