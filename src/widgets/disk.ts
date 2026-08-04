import { invoke } from "@tauri-apps/api/core";
import { html, render } from "lit-html";
import { fmtBytes, subscribeSettings, TaskbarWidget, widgetConfig } from "../shared/widget";
import { barRow, heat, DiskInfo, subscribeStats, SystemStats } from "./system-shared";

// Module-scope, never disposed: keeps the last disk list warm so the dashboard's
// tile_drive select can list real drive names without mounting the widget.
let lastDisks: DiskInfo[] = [];
subscribeStats((s) => {
  lastDisks = s.disks;
});

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

function tileTemplate(s: SystemStats | null, tileDrive: string | undefined) {
  if (!s) return html`<span class="muted">…</span>`;
  const d = primaryDisk(s.disks, tileDrive);
  if (!d) return html`<span class="muted">–</span>`;
  const usedPct = d.total_bytes ? ((d.total_bytes - d.free_bytes) / d.total_bytes) * 100 : 0;
  return html`
    <span class="stat ${heat(usedPct)}">
      <i class="ph ph-hard-drives"></i>
      <span class="val">${fmtBytes(d.free_bytes, 0)}</span>
      <span class="unit">free</span>
    </span>
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
  configFields: () => [
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
  ],
  mountTile(root) {
    let latestStats: SystemStats | null = null;
    let tileDrive: string | undefined;
    const repaint = () => {
      lastPrimaryDrive = latestStats ? (primaryDisk(latestStats.disks, tileDrive)?.name ?? null) : null;
      render(tileTemplate(latestStats, tileDrive), root);
    };
    const stopStats = subscribeStats((s) => {
      latestStats = s;
      repaint();
    });
    const stopSettings = subscribeSettings((s) => {
      tileDrive = widgetConfig(s, "disk").tile_drive as string | undefined;
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
