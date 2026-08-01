import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { reportErrors } from "./shared/report-errors";
import { ConfigField, Settings, TaskbarWidget, widgetConfig } from "./shared/widget";
import { allWidgets } from "./widgets/registry";

reportErrors("dashboard");

// No native context/inspect menu anywhere in this app's webviews.
document.addEventListener("contextmenu", (e) => e.preventDefault());

let settings: Settings | null = null;
let expandedId: string | null = null;
const root = document.getElementById("dashboard")!;

async function load() {
  settings = await invoke<Settings>("get_settings").catch(() => null);
  renderAll();
}

// Every mutation writes the full Settings object back; save_settings also emits
// "widgets-changed" so the strip (and flyout, for widget_config) pick it up.
function save(next: Settings) {
  settings = next;
  invoke("save_settings", { newSettings: next }).catch(() => {});
  renderAll();
}

function toggleWidget(id: string, on: boolean) {
  if (!settings) return;
  const enabled = settings.enabled_widgets.filter((w) => w !== id);
  const hidden = settings.hidden_widgets.filter((w) => w !== id);
  if (on) enabled.push(id);
  else hidden.push(id);
  save({ ...settings, enabled_widgets: enabled, hidden_widgets: hidden });
}

function moveWidget(id: string, dir: -1 | 1) {
  if (!settings) return;
  const order = [...settings.enabled_widgets];
  const i = order.indexOf(id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= order.length) return;
  [order[i], order[j]] = [order[j], order[i]];
  save({ ...settings, enabled_widgets: order });
}

function setFieldValue(widgetId: string, key: string, value: unknown) {
  if (!settings) return;
  const cfg = { ...widgetConfig(settings, widgetId), [key]: value };
  save({ ...settings, widget_config: { ...settings.widget_config, [widgetId]: cfg } });
}

function fieldRow(widgetId: string, field: ConfigField) {
  const current = widgetConfig(settings, widgetId)[field.key] ?? field.default;
  if (field.type === "toggle") {
    return html`
      <label class="field-row">
        <span>${field.label}</span>
        <input
          type="checkbox"
          .checked=${!!current}
          @change=${(e: Event) =>
            setFieldValue(widgetId, field.key, (e.target as HTMLInputElement).checked)}
        />
      </label>
    `;
  }
  if (field.type === "number") {
    return html`
      <label class="field-row">
        <span>${field.label}</span>
        <input
          type="number"
          .value=${String(current ?? "")}
          @change=${(e: Event) =>
            setFieldValue(widgetId, field.key, Number((e.target as HTMLInputElement).value))}
        />
      </label>
    `;
  }
  return html`
    <label class="field-row">
      <span>${field.label}</span>
      <select
        @change=${(e: Event) =>
          setFieldValue(widgetId, field.key, (e.target as HTMLSelectElement).value)}
      >
        ${(field.options ?? []).map(
          (o) => html`<option value=${o.value} ?selected=${o.value === current}>${o.label}</option>`,
        )}
      </select>
    </label>
  `;
}

function widgetRow(w: TaskbarWidget, index: number, total: number) {
  const isEnabled = settings?.enabled_widgets.includes(w.id) ?? false;
  const isExpanded = expandedId === w.id;
  const fields = w.configFields?.() ?? [];
  return html`
    <div class="widget-row" id="widget-${w.id}">
      <div class="widget-head">
        <input
          type="checkbox"
          .checked=${isEnabled}
          @change=${(e: Event) => toggleWidget(w.id, (e.target as HTMLInputElement).checked)}
        />
        <span class="name">${w.name}</span>
        <div class="widget-actions">
          <button ?disabled=${!isEnabled || index === 0} @click=${() => moveWidget(w.id, -1)}>
            <i class="ph ph-caret-up"></i>
          </button>
          <button
            ?disabled=${!isEnabled || index === total - 1}
            @click=${() => moveWidget(w.id, 1)}
          >
            <i class="ph ph-caret-down"></i>
          </button>
          ${fields.length
            ? html`<button
                @click=${() => {
                  expandedId = isExpanded ? null : w.id;
                  renderAll();
                }}
              >
                Edit
              </button>`
            : null}
        </div>
      </div>
      ${isExpanded && fields.length
        ? html`<div class="widget-config">${fields.map((f) => fieldRow(w.id, f))}</div>`
        : null}
    </div>
  `;
}

function scrollToSection(id: string) {
  document.getElementById(`section-${id}`)?.scrollIntoView({ behavior: "smooth" });
}

function renderAll() {
  if (!settings) {
    render(html`<div class="empty">Loading…</div>`, root);
    return;
  }
  const order = settings.enabled_widgets;
  const ordered = [...allWidgets()].sort((a, b) => {
    const ia = order.indexOf(a.id);
    const ib = order.indexOf(b.id);
    return (ia === -1 ? 999 : ia) - (ib === -1 ? 999 : ib);
  });
  render(
    html`
      <div class="dash">
        <nav class="dash-nav">
          <button class="nav-item" @click=${() => scrollToSection("widgets")}>Widgets</button>
          <button class="nav-item" @click=${() => scrollToSection("host")}>Host</button>
        </nav>
        <div class="dash-content">
          <section id="section-widgets">
            <h2>Widgets</h2>
            ${ordered.map((w) => widgetRow(w, order.indexOf(w.id), order.length))}
          </section>
          <section id="section-host">
            <h2>Host</h2>
            <label class="field-row">
              <span>Left margin (px)</span>
              <input
                type="number"
                min="0"
                .value=${String(settings.left_margin)}
                @change=${(e: Event) =>
                  save({
                    ...settings!,
                    left_margin: Number((e.target as HTMLInputElement).value),
                  })}
              />
            </label>
            <label class="field-row">
              <span>Stats poll interval (s)</span>
              <input
                type="number"
                min="1"
                .value=${String(settings.stats_poll_seconds)}
                @change=${(e: Event) =>
                  save({
                    ...settings!,
                    stats_poll_seconds: Math.max(
                      1,
                      Number((e.target as HTMLInputElement).value),
                    ),
                  })}
              />
            </label>
          </section>
        </div>
      </div>
    `,
    root,
  );
}

listen<{ section: string | null }>("dashboard-navigate", (e) => {
  const section = e.payload.section;
  if (!section) {
    scrollToSection("widgets");
    return;
  }
  expandedId = section;
  renderAll();
  requestAnimationFrame(() =>
    document.getElementById(`widget-${section}`)?.scrollIntoView({ behavior: "smooth", block: "center" }),
  );
});

load();
