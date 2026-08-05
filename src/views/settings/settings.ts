import "../../../vendor/tauri_kit/frontend/settings/styles.css";
import "../../../vendor/tauri_kit/frontend/settings/palettes/sirbepy-default.css";
import { invoke } from "@tauri-apps/api/core";
import { renderSettingsPage, type RenderOptions } from "../../../vendor/tauri_kit/frontend/settings/renderer";
import {
  SIRBEPY_PALETTES,
  SIRBEPY_DEFAULT_PALETTE,
} from "../../../vendor/tauri_kit/frontend/settings/palettes/sirbepy-default";
import { settingsSchema, systemInline } from "./schema";

interface VersionInfo {
  version: string;
  build_date: string;
  installed_at: string | null;
}

export function mountSettings(
  root: HTMLElement,
  extra?: Pick<RenderOptions, "onHeaderChange">,
): Promise<() => void> {
  return renderSettingsPage(root, {
    schema: settingsSchema,
    systemInline,
    about: {},
    palettes: SIRBEPY_PALETTES,
    theme: { defaultPalette: SIRBEPY_DEFAULT_PALETTE },
    getVersionInfo: async () => {
      const info = await invoke<VersionInfo>("get_version_info");
      return {
        buildDate: info.build_date !== "unknown" ? info.build_date : undefined,
        installedAt: info.installed_at ?? undefined,
      };
    },
    ...extra,
  });
}
