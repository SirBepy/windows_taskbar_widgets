import { html } from "lit-html";
import { ConfigField, TaskbarWidget } from "../shared/widget";
import { barRow, heat, mountStatsFlyout, mountStatsTile, SystemStats } from "./system-shared";

interface TempCfg {
  unit: "celsius" | "fahrenheit";
}

const readCfg = (cfg: Record<string, unknown>): TempCfg => ({
  unit: (cfg.unit as TempCfg["unit"]) ?? "celsius",
});

// heat() thresholds (70/90) stay meaningful only against the raw Celsius reading,
// so colouring always uses celsius regardless of the display unit.
function fmtTemp(celsius: number, unit: TempCfg["unit"]): string {
  const v = unit === "fahrenheit" ? (celsius * 9) / 5 + 32 : celsius;
  return Math.round(v).toString();
}

function statSpan(icon: string, celsius: number, unit: TempCfg["unit"]) {
  return html`
    <span class="stat ${heat(celsius)}">
      <i class="ph ${icon}"></i>
      <span class="val temp-val">${fmtTemp(celsius, unit)}</span><span class="unit">°</span>
    </span>
  `;
}

function tileTemplate(s: SystemStats | null, cfg: TempCfg) {
  if (!s) return html`<span class="muted">…</span>`;
  const cpuC = s.cpu_temp_c;
  const gpuC = s.gpu?.temp_c ?? null;
  if (cpuC == null && gpuC == null) return html`<span class="muted">–</span>`;
  return html`
    ${cpuC != null ? statSpan("ph-cpu", cpuC, cfg.unit) : null}
    ${gpuC != null ? statSpan("ph-graphics-card", gpuC, cfg.unit) : null}
  `;
}

function flyoutTemplate(s: SystemStats | null, cfg: TempCfg) {
  if (!s) return html`<div class="empty">Collecting stats…</div>`;
  const letter = cfg.unit === "fahrenheit" ? "F" : "C";
  const cpuC = s.cpu_temp_c;
  const gpuC = s.gpu?.temp_c ?? null;
  if (cpuC == null && gpuC == null) return html`<div class="empty">No temperature sensors detected.</div>`;
  return html`
    <div class="fly-title"><i class="ph ph-thermometer"></i>Temperatures</div>
    ${cpuC != null ? barRow("ph-cpu", "CPU", cpuC, `${fmtTemp(cpuC, cfg.unit)}°${letter}`) : null}
    ${gpuC != null ? barRow("ph-graphics-card", "GPU", gpuC, `${fmtTemp(gpuC, cfg.unit)}°${letter}`) : null}
  `;
}

const configFields: ConfigField[] = [
  {
    key: "unit",
    label: "Unit",
    type: "select",
    options: [
      { value: "celsius", label: "Celsius" },
      { value: "fahrenheit", label: "Fahrenheit" },
    ],
    default: "celsius",
  },
];

export const temperatureWidget: TaskbarWidget = {
  id: "temperature",
  name: "Temperature",
  // 28 (#flyout's 14px*2 padding) + 37 (fly-title) + 2*30 (fly-row). Those per-row
  // constants come from the measured resize in 312b1d9; disk's 245 checks out exactly.
  flyout: { widthCss: 245, heightCss: 125 },
  configFields: () => configFields,
  mountTile(root) {
    return mountStatsTile(root, "temperature", readCfg, tileTemplate);
  },
  mountFlyout(root) {
    return mountStatsFlyout(root, "temperature", readCfg, flyoutTemplate);
  },
};
