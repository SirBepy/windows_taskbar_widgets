import { html, type TemplateResult } from "lit-html";
import type { CustomField } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { lazyIpcField } from "./lazy-ipc-field";
import type { MonitorOption } from "./widget-strip-lanes";

// Not part of the Settings struct, so re-render is driven by "settings-reset"
// (same trick as autostartField) rather than the kit's normal hydration.
let options: MonitorOption[] = [];
const ensureLoaded = lazyIpcField<MonitorOption[]>("list_taskbar_monitors", (list) => {
  options = list;
});

function primaryLabel(): string {
  const primary = options.find((o) => o.is_primary);
  return primary ? `Primary - ${primary.width}x${primary.height}` : "Primary";
}

export function taskbarMonitorField(): CustomField {
  return {
    key: "taskbar_monitor",
    label: "Taskbar",
    kind: "custom",
    render(value, onChange): TemplateResult {
      ensureLoaded();
      const current = typeof value === "string" ? value : "";
      return html`
        <label class="kit-row">
          <span class="kit-row-label">Taskbar</span>
          <select
            class="kit-select"
            @change=${(e: Event) => onChange((e.target as HTMLSelectElement).value)}
          >
            <option value="" ?selected=${current === ""}>${primaryLabel()}</option>
            ${options
              .filter((o) => !o.is_primary)
              .map(
                (o) => html`
                  <option value=${o.device_name} ?selected=${current === o.device_name}>
                    ${o.device_name} - ${o.width}x${o.height}
                  </option>
                `,
              )}
          </select>
        </label>
      `;
    },
  };
}
