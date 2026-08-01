import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { widgetsFor } from "./widgets/registry";
import { reportErrors } from "./shared/report-errors";
import { TaskbarWidget } from "./shared/widget";

reportErrors("strip");

const HOVER_OPEN_DELAY_MS = 250;

interface Settings {
  left_margin: number;
  enabled_widgets: string[];
  stats_poll_seconds: number;
}

function reportStripWidth(row: HTMLElement) {
  let last = 0;
  const push = () => {
    const w = Math.ceil(row.scrollWidth) + 4;
    if (w !== last) {
      last = w;
      invoke("set_strip_width", { widthCss: w }).catch(() => {});
    }
  };
  new ResizeObserver(push).observe(row);
  push();
}

function wireFlyoutHover(tile: HTMLElement, widget: TaskbarWidget) {
  if (!widget.flyout) return;
  let openTimer: number | undefined;
  tile.addEventListener("mouseenter", () => {
    openTimer = window.setTimeout(() => {
      const r = tile.getBoundingClientRect();
      invoke("open_flyout", {
        widgetId: widget.id,
        anchorXCss: r.left + r.width / 2,
        widthCss: widget.flyout!.widthCss,
        heightCss: widget.flyout!.heightCss,
      }).catch(() => {});
    }, HOVER_OPEN_DELAY_MS);
  });
  tile.addEventListener("mouseleave", () => window.clearTimeout(openTimer));
}

async function main() {
  const settings = await invoke<Settings>("get_settings").catch(() => null);
  const enabled = settings?.enabled_widgets ?? ["cpu", "ram", "gpu", "disk", "conductor"];
  const row = document.getElementById("strip")!;

  for (const widget of widgetsFor(enabled)) {
    const tile = document.createElement("div");
    tile.className = "tile";
    tile.dataset.widget = widget.id;
    row.appendChild(tile);
    widget.mountTile(tile);
    wireFlyoutHover(tile, widget);
  }

  document.documentElement.addEventListener("mouseenter", () => {
    invoke("flyout_zone", { zone: "strip", inside: true }).catch(() => {});
  });
  document.documentElement.addEventListener("mouseleave", () => {
    invoke("flyout_zone", { zone: "strip", inside: false }).catch(() => {});
  });

  reportStripWidth(row);
}

main();
