import { html, type TemplateResult } from "lit-html";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type { CustomField } from "../../../vendor/tauri_kit/frontend/settings/schema";

interface TaskbarMonitorOption {
  device_name: string;
  is_primary: boolean;
  width: number;
  height: number;
}

// Not part of the Settings struct, so re-render is driven by "settings-reset"
// (same trick as autostartField) rather than the kit's normal hydration.
let options: TaskbarMonitorOption[] = [];
let fetchStarted = false;

function ensureLoaded(): void {
  if (fetchStarted) return;
  fetchStarted = true;
  invoke<TaskbarMonitorOption[]>("list_taskbar_monitors")
    .then((list) => {
      options = list;
      void emit("settings-reset");
    })
    .catch(() => {});
}

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
