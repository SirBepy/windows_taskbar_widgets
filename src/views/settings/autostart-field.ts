import { html, type TemplateResult } from "lit-html";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type { CustomField } from "../../../vendor/tauri_kit/frontend/settings/schema";

// Registry is the source of truth (Task Manager's Startup tab can remove the
// entry), so this never calls the kit's onChange/setField - it would fold into
// settings.json via save_settings. get_autostart/set_autostart only, repainted
// via "settings-reset" (the kit's only public re-render hook for a custom field).
let autostart = false;
let fetchStarted = false;

function ensureLoaded(): void {
  if (fetchStarted) return;
  fetchStarted = true;
  invoke<boolean>("get_autostart")
    .then((v) => {
      autostart = v;
      void emit("settings-reset");
    })
    .catch(() => {});
}

async function toggle(on: boolean): Promise<void> {
  await invoke("set_autostart", { enabled: on }).catch(() => {});
  autostart = await invoke<boolean>("get_autostart").catch(() => autostart);
  void emit("settings-reset");
}

export function autostartField(): CustomField {
  return {
    key: "__autostart_ui_only",
    label: "Start with Windows",
    kind: "custom",
    render(): TemplateResult {
      ensureLoaded();
      return html`
        <label class="kit-row">
          <span class="kit-row-label">Start with Windows</span>
          <span class="kit-toggle">
            <input
              type="checkbox"
              .checked=${autostart}
              @change=${(e: Event) => void toggle((e.target as HTMLInputElement).checked)}
            />
            <span class="kit-toggle-track"></span>
          </span>
        </label>
      `;
    },
  };
}
