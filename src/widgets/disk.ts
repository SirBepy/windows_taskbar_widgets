import { html, render } from "lit-html";
import { fmtBytes, TaskbarWidget } from "../shared/widget";
import { barRow, heat, DiskInfo, subscribeStats, SystemStats } from "./system-shared";

// Prefer C: (the system drive) for the compact tile number; else whichever
// drive is closest to full, since that's the one worth keeping an eye on.
function primaryDisk(disks: DiskInfo[]): DiskInfo | null {
  if (disks.length === 0) return null;
  const c = disks.find((d) => /^c:\\?$/i.test(d.name));
  if (c) return c;
  return disks.reduce((min, d) => (d.free_bytes < min.free_bytes ? d : min));
}

function tileTemplate(s: SystemStats | null) {
  if (!s) return html`<span class="muted">…</span>`;
  const d = primaryDisk(s.disks);
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

export const diskWidget: TaskbarWidget = {
  id: "disk",
  name: "Disk",
  flyout: { widthCss: 320, heightCss: 260 },
  mountTile(root) {
    return subscribeStats((s) => render(tileTemplate(s), root));
  },
  mountFlyout(root) {
    render(flyoutTemplate(null), root);
    return subscribeStats((s) => render(flyoutTemplate(s), root));
  },
};
