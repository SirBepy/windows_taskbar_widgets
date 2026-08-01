import { html, render } from "lit-html";
import { fmtBytes, TaskbarWidget } from "../shared/widget";
import { barRow, heat, subscribeStats, SystemStats } from "./system-shared";

function tileTemplate(s: SystemStats | null) {
  if (!s?.gpu) return html``;
  return html`
    <span class="stat ${heat(s.gpu.util_pct)}">
      <i class="ph ph-graphics-card"></i>
      <span class="val">${s.gpu.util_pct}%</span>
      <span class="unit">${s.gpu.temp_c}°</span>
    </span>
  `;
}

function flyoutTemplate(s: SystemStats | null) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  if (!s.gpu) return html`<div class="empty">No discrete GPU detected.</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-graphics-card"></i>GPU</div>
    ${barRow("ph-graphics-card", `GPU · ${s.gpu.temp_c}°C`, s.gpu.util_pct, `${s.gpu.util_pct}%`)}
    ${barRow(
      "ph-graphics-card",
      "VRAM",
      s.gpu.vram_total_bytes ? (s.gpu.vram_used_bytes / s.gpu.vram_total_bytes) * 100 : 0,
      `${fmtBytes(s.gpu.vram_used_bytes)} / ${fmtBytes(s.gpu.vram_total_bytes, 0)}`,
    )}
  `;
}

export const gpuWidget: TaskbarWidget = {
  id: "gpu",
  name: "GPU",
  flyout: { widthCss: 300, heightCss: 160 },
  mountTile(root) {
    return subscribeStats((s) => render(tileTemplate(s), root));
  },
  mountFlyout(root) {
    render(flyoutTemplate(null), root);
    return subscribeStats((s) => render(flyoutTemplate(s), root));
  },
};
