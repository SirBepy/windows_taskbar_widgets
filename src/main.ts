import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { runAutoUpdateCheck } from "../vendor/tauri_kit/frontend/updater/auto-check";
import { allWidgetIds, widgetsFor, widgetById } from "./widgets/registry";
import { reportErrors } from "./shared/report-errors";
import {
  applyOpacity,
  idsForMonitor,
  instanceIdFor,
  newWidgetIds,
  Settings,
  stripNeedsRemount,
  stripTileIds,
  TaskbarWidget,
} from "./shared/widget";

reportErrors("strip");

// Skip in dev: the dev binary lags the released version, so check() would always
// "find" an update and nag. Reads __kit_auto_update from settings for the mode.
if (!import.meta.env.DEV) runAutoUpdateCheck();

// No native context/inspect menu anywhere in this app's webviews; tiles wire
// their own native menu below via show_tile_menu.
document.addEventListener("contextmenu", (e) => e.preventDefault());

const HOVER_OPEN_DELAY_MS = 250;

let row: HTMLElement;
let tileCleanups: (() => void)[] = [];
let lastVisibleIds: string[] = [];
// Resolved once at boot from `strip_monitor_key`, then reused for every re-render:
// the window is destroyed and rebuilt if its monitor goes away, so this cannot
// change for a live window. `null` means the invoke rejected.
let monitorKey: string | null = null;
// Shared across tiles (only one can be hovered/dragged at a time) so drag-start
// can cancel a pending hover-open without each tile tracking its own timer.
let flyoutOpenTimer: number | undefined;

// Returns a "force" fn that re-reports even when the width is unchanged, so a
// changed left_margin (which set_strip_width also applies) still repositions.
function reportStripWidth(row: HTMLElement): () => void {
  let last = -1;
  const push = () => {
    const w = Math.ceil(row.scrollWidth) + 4;
    if (w !== last) {
      last = w;
      invoke("set_strip_width", { widthCss: w }).catch(() => {});
    }
  };
  new ResizeObserver(push).observe(row);
  push();
  return () => {
    last = -1;
    push();
  };
}

function wireFlyoutHover(tile: HTMLElement, widget: TaskbarWidget): () => void {
  if (!widget.flyout) return () => {};
  const onEnter = () => {
    flyoutOpenTimer = window.setTimeout(() => {
      const r = tile.getBoundingClientRect();
      const dims = widget.flyoutDims?.() ?? widget.flyout!;
      invoke("open_flyout", {
        widgetId: widget.id,
        anchorXCss: r.left + r.width / 2,
        widthCss: dims.widthCss,
        heightCss: dims.heightCss,
      }).catch(() => {});
    }, HOVER_OPEN_DELAY_MS);
  };
  const onLeave = () => window.clearTimeout(flyoutOpenTimer);
  tile.addEventListener("mouseenter", onEnter);
  tile.addEventListener("mouseleave", onLeave);
  return () => {
    window.clearTimeout(flyoutOpenTimer);
    tile.removeEventListener("mouseenter", onEnter);
    tile.removeEventListener("mouseleave", onLeave);
  };
}

function wireContextMenu(tile: HTMLElement, widget: TaskbarWidget, settings: Settings | null) {
  tile.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("show_tile_menu", {
      instanceId: instanceIdFor(settings, widget.id),
      items: widget.menuItems?.() ?? [],
    }).catch(() => {});
  });
}

const idsForThisWindow = (ids: string[], settings: Settings | null) =>
  idsForMonitor(ids, settings, monitorKey);

// A widget placed as a floating overlay gets its own window instead of a tile, so
// it is dropped here rather than filtered out of enabled_widgets (which would lose
// its position in the strip if the user moves it back).
function renderTiles(ids: string[], settings: Settings | null) {
  lastVisibleIds = stripTileIds(idsForThisWindow(ids, settings), settings);
  tileCleanups.forEach((stop) => stop());
  tileCleanups = [];
  row.replaceChildren();
  for (const widget of widgetsFor(lastVisibleIds)) {
    const tile = document.createElement("div");
    tile.className = "tile";
    tile.dataset.widget = widget.id;
    row.appendChild(tile);
    tileCleanups.push(widget.mountTile(tile));
    tileCleanups.push(wireFlyoutHover(tile, widget));
    wireContextMenu(tile, widget, settings);
  }
}

async function main() {
  let settings = await invoke<Settings>("get_settings").catch(() => null);
  monitorKey = await invoke<string>("strip_monitor_key").catch(() => null);
  let enabled = settings?.enabled_widgets ?? ["cpu", "ram", "gpu", "disk", "conductor"];
  const hidden = settings?.hidden_widgets ?? [];
  // New registry widgets (added after a user's settings.json was written) default
  // to visible: adopt them into enabled_widgets once, unless already hidden.
  const newIds = newWidgetIds(allWidgetIds(), enabled, hidden);
  if (newIds.length > 0) {
    enabled = [...enabled, ...newIds];
    // Awaited, then re-read: reorder_widgets persists (so persist's ensure_instances
    // backfills the new widget's lane entry) but emits no widgets-changed, so rendering
    // the pre-adoption settings would filter the new widget out for the whole session.
    await invoke("reorder_widgets", { order: enabled }).catch(() => {});
    settings = (await invoke<Settings>("get_settings").catch(() => null)) ?? settings;
  }
  row = document.getElementById("strip")!;

  applyOpacity(settings);
  renderTiles(enabled, settings);
  const forceReport = reportStripWidth(row);

  listen("widgets-changed", async () => {
    const s = await invoke<Settings>("get_settings").catch(() => null);
    applyOpacity(s);
    // Config-only saves (opacity, margin, hide_from_capture, show_temp...) reach mounted
    // tiles via subscribeSettings; only a tile-membership/order change remounts the strip.
    if (s && stripNeedsRemount(lastVisibleIds, stripTileIds(idsForThisWindow(s.enabled_widgets, s), s))) {
      renderTiles(s.enabled_widgets, s);
    }
    forceReport();
  });
  listen<{ widget_id: string; item_id: string }>("tile-menu-action", (e) => {
    widgetById(e.payload.widget_id)?.onMenuAction?.(e.payload.item_id);
  });
}

main();
