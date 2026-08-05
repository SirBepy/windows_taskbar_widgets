import { invoke } from "@tauri-apps/api/core";
import { html, render } from "lit-html";
import { fmtBytes, subscribeSettings, TaskbarWidget, widgetConfig } from "../shared/widget";
import {
  barRow,
  heat,
  DiskInfo,
  fetchStatsOnce,
  readShowPercent,
  subscribeStats,
  SystemStats,
} from "./system-shared";

// Backs the dashboard's tile_drive select, which needs real drive names without
// mounting the widget. Refreshed on demand rather than by a live subscription:
// a listener here would never be disposed and would wake every window (the
// dashboard included) every poll tick just to restock a rarely-read cache.
let lastDisks: DiskInfo[] = [];
const refreshDisks = () =>
  fetchStatsOnce().then((s) => {
    if (s) lastDisks = s.disks;
  });
refreshDisks();

// Prefer the configured drive; else C: (the system drive) for the compact tile
// number; else whichever drive is closest to full, since that's worth watching.
function primaryDisk(disks: DiskInfo[], tileDrive?: string): DiskInfo | null {
  if (disks.length === 0) return null;
  if (tileDrive && tileDrive !== "auto") {
    const match = disks.find((d) => d.name === tileDrive);
    if (match) return match;
  }
  const c = disks.find((d) => /^c:\\?$/i.test(d.name));
  if (c) return c;
  return disks.reduce((min, d) => (d.free_bytes < min.free_bytes ? d : min));
}

function tileTemplate(s: SystemStats | null, tileDrive: string | undefined, showPercent: boolean) {
  if (!s) return html`<span class="muted">…</span>`;
  const d = primaryDisk(s.disks, tileDrive);
  if (!d) return html`<span class="muted">–</span>`;
  const usedPct = d.total_bytes ? ((d.total_bytes - d.free_bytes) / d.total_bytes) * 100 : 0;
  const freePct = d.total_bytes ? (d.free_bytes / d.total_bytes) * 100 : 0;
  return html`
    <div class="tile-stat ${heat(usedPct)}">
      <span class="tile-label">DISK</span>
      <span class="tile-value">
        ${showPercent
          ? html`<span class="tile-pct"><span class="num">${freePct.toFixed(0)}</span>%</span>`
          : html`
              <span class="tile-pct">${fmtBytes(d.free_bytes, 0)}</span>
              <span class="tile-unit">free</span>
            `}
      </span>
    </div>
  `;
}

function flyoutTemplate(s: SystemStats | null) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  if (s.disks.length === 0) return html`<div class="empty">No drives detected.</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-hard-drives"></i>Disks</div>
    ${s.disks.map((d) => {
      const usedPct = d.total_bytes ? ((d.total_bytes - d.free_bytes) / d.total_bytes) * 100 : 0;
      return barRow(
        "ph-hard-drives",
        d.name.replace(/\\$/, ""),
        usedPct,
        `${fmtBytes(d.free_bytes, 0)} free`,
      );
    })}
  `;
}

// Set by the tile's stats subscription; read when "Open in Explorer" fires.
let lastPrimaryDrive: string | null = null;

export const diskWidget: TaskbarWidget = {
  id: "disk",
  name: "Disk",
  // Sized for 6 drives (a generous power-user max); beyond that #flyout's
  // overflow-y:auto scrolls instead of resizing the window.
  flyout: { widthCss: 285, heightCss: 245 },
  menuItems: () => [{ id: "open-drive", label: "Open in Explorer" }],
  onMenuAction: (id) => {
    if (id === "open-drive" && lastPrimaryDrive) {
      invoke("open_explorer", { path: lastPrimaryDrive }).catch(() => {});
    }
  },
  configFields: () => {
    // Restocks for the next open; drive letters don't change while a config
    // accordion is on screen, so this render uses the cache as-is.
    refreshDisks();
    return [
      {
        key: "tile_drive",
        label: "Drive shown in tile",
        type: "select",
        options: [
          { value: "auto", label: "Auto" },
          ...lastDisks.map((d) => ({ value: d.name, label: d.name.replace(/\\$/, "") })),
        ],
        default: "auto",
      },
      { key: "show_percent", label: "Show as percentage", type: "toggle", default: true },
    ];
  },
  mountTile(root) {
    let latestStats: SystemStats | null = null;
    let tileDrive: string | undefined;
    let showPercent = true;
    const repaint = () => {
      lastPrimaryDrive = latestStats ? (primaryDisk(latestStats.disks, tileDrive)?.name ?? null) : null;
      render(tileTemplate(latestStats, tileDrive, showPercent), root);
    };
    const stopStats = subscribeStats((s) => {
      latestStats = s;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      const cfg = widgetConfig(s, "disk");
      tileDrive = cfg.tile_drive as string | undefined;
      showPercent = readShowPercent(cfg);
      repaint();
    });
    return () => {
      stopStats();
      stopSettings();
    };
  },
  mountFlyout(root) {
    render(flyoutTemplate(null), root);
    return subscribeStats((s) => render(flyoutTemplate(s), root));
  },
};
