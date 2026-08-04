import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { runAutoUpdateCheck } from "../vendor/tauri_kit/frontend/updater/auto-check";
import { allWidgetIds, widgetsFor, widgetById } from "./widgets/registry";
import { reportErrors } from "./shared/report-errors";
import { applyOpacity, isDragging, setDragging, Settings, TaskbarWidget } from "./shared/widget";

reportErrors("strip");

// Skip in dev: the dev binary lags the released version, so check() would always
// "find" an update and nag. Reads __kit_auto_update from settings for the mode.
if (!import.meta.env.DEV) runAutoUpdateCheck();

// No native context/inspect menu anywhere in this app's webviews; tiles wire
// their own native menu below via show_tile_menu.
document.addEventListener("contextmenu", (e) => e.preventDefault());

const HOVER_OPEN_DELAY_MS = 250;
const DRAG_THRESHOLD_PX = 6;

let row: HTMLElement;
let tileCleanups: (() => void)[] = [];
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
    if (isDragging()) return;
    flyoutOpenTimer = window.setTimeout(() => {
      const r = tile.getBoundingClientRect();
      invoke("open_flyout", {
        widgetId: widget.id,
        anchorXCss: r.left + r.width / 2,
        widthCss: widget.flyout!.widthCss,
        heightCss: widget.flyout!.heightCss,
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

function wireContextMenu(tile: HTMLElement, widget: TaskbarWidget) {
  tile.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("show_tile_menu", {
      widgetId: widget.id,
      items: widget.menuItems?.() ?? [],
    }).catch(() => {});
  });
}

// Moves `tile` at most one slot per call, toward wherever the pointer center
// now sits; repeated pointermove calls converge it to the right spot.
function reorderByPointer(tile: HTMLElement, clientX: number) {
  const children = Array.from(row.children) as HTMLElement[];
  const tileIndex = children.indexOf(tile);
  for (const sib of children) {
    if (sib === tile) continue;
    const r = sib.getBoundingClientRect();
    const mid = r.left + r.width / 2;
    const sibIndex = children.indexOf(sib);
    if (clientX < mid && tileIndex > sibIndex) {
      row.insertBefore(tile, sib);
      return;
    }
    if (clientX > mid && tileIndex < sibIndex) {
      row.insertBefore(tile, sib.nextSibling);
      return;
    }
  }
}

function wireDrag(tile: HTMLElement) {
  let startX = 0;
  let pointerId: number | null = null;
  let isDragging = false;

  tile.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    startX = e.clientX;
    pointerId = e.pointerId;
  });

  tile.addEventListener("pointermove", (e) => {
    if (pointerId === null || e.pointerId !== pointerId) return;
    if (!isDragging) {
      if (Math.abs(e.clientX - startX) < DRAG_THRESHOLD_PX) return;
      isDragging = true;
      setDragging(true);
      window.clearTimeout(flyoutOpenTimer);
      tile.setPointerCapture(pointerId);
      tile.classList.add("dragging");
    }
    reorderByPointer(tile, e.clientX);
  });

  const endDrag = (e: PointerEvent) => {
    if (pointerId === null || e.pointerId !== pointerId) return;
    if (isDragging) {
      tile.releasePointerCapture(pointerId);
      tile.classList.remove("dragging");
      const order = Array.from(row.children).map((el) => (el as HTMLElement).dataset.widget!);
      invoke("reorder_widgets", { order }).catch(() => {});
    }
    isDragging = false;
    setDragging(false);
    pointerId = null;
  };
  tile.addEventListener("pointerup", endDrag);
  tile.addEventListener("pointercancel", endDrag);
}

function renderTiles(ids: string[]) {
  tileCleanups.forEach((stop) => stop());
  tileCleanups = [];
  row.replaceChildren();
  for (const widget of widgetsFor(ids)) {
    const tile = document.createElement("div");
    tile.className = "tile";
    tile.dataset.widget = widget.id;
    row.appendChild(tile);
    tileCleanups.push(widget.mountTile(tile));
    tileCleanups.push(wireFlyoutHover(tile, widget));
    wireContextMenu(tile, widget);
    wireDrag(tile);
  }
}

async function main() {
  const settings = await invoke<Settings>("get_settings").catch(() => null);
  let enabled = settings?.enabled_widgets ?? ["cpu", "ram", "gpu", "disk", "conductor"];
  const hidden = settings?.hidden_widgets ?? [];
  // New registry widgets (added after a user's settings.json was written) default
  // to visible: adopt them into enabled_widgets once, unless already hidden.
  const newIds = allWidgetIds().filter((id) => !enabled.includes(id) && !hidden.includes(id));
  if (newIds.length > 0) {
    enabled = [...enabled, ...newIds];
    invoke("reorder_widgets", { order: enabled }).catch(() => {});
  }
  row = document.getElementById("strip")!;

  applyOpacity(settings);
  renderTiles(enabled);
  const forceReport = reportStripWidth(row);

  listen("widgets-changed", async () => {
    const s = await invoke<Settings>("get_settings").catch(() => null);
    applyOpacity(s);
    if (s) renderTiles(s.enabled_widgets);
    forceReport();
  });
  listen<{ widget_id: string; item_id: string }>("tile-menu-action", (e) => {
    widgetById(e.payload.widget_id)?.onMenuAction?.(e.payload.item_id);
  });
}

main();
