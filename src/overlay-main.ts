import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { widgetById } from "./widgets/registry";
import { reportErrors } from "./shared/report-errors";
import {
  applyOpacity,
  overlayDims,
  overlayRenderer,
  placementOf,
  type Settings,
} from "./shared/widget";

reportErrors("overlay");

// No native context/inspect menu anywhere in this app's webviews.
document.addEventListener("contextmenu", (e) => e.preventDefault());

// Matches the design's snapping threshold; assistance only, free x/y is still stored.
const SNAP_PX = 12;
const PERSIST_DEBOUNCE_MS = 250;
// Mirrors MIN_VISIBLE in src-tauri/src/overlay.rs, which clamps the same way for
// hand-edited settings and vanished monitors. Same CSS px space, so no conversion.
const MIN_VISIBLE_PX = 48;

const win = getCurrentWindow();
const root = document.getElementById("overlay")!;
const grip = document.getElementById("overlay-resize")!;

let widgetId: string | null = null;
let persistTimer: number | undefined;

interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

async function applySettings(): Promise<void> {
  if (!widgetId) return;
  const settings = await invoke<Settings>("get_settings").catch(() => null);
  const placement = placementOf(settings, widgetId);
  applyOpacity(settings, placement.kind === "overlay" ? placement.opacity : null);
}

// Turns the live window rect into monitor-relative CSS coords, snaps to the monitor's
// edges, and writes it back. Never emits widgets-changed: a re-render mid-drag is wrong.
async function persistGeometry(): Promise<void> {
  if (!widgetId) return;
  const [pos, size] = [await win.outerPosition(), await win.outerSize()];
  const monitor = await invoke<MonitorInfo | null>("monitor_at_point", {
    x: pos.x,
    y: pos.y,
  }).catch(() => null);
  if (!monitor) return;

  const w = size.width / monitor.scale;
  const h = size.height / monitor.scale;
  let x = (pos.x - monitor.x) / monitor.scale;
  let y = (pos.y - monitor.y) / monitor.scale;

  if (Math.abs(x) <= SNAP_PX) x = 0;
  if (Math.abs(monitor.width - (x + w)) <= SNAP_PX) x = monitor.width - w;
  if (Math.abs(y) <= SNAP_PX) y = 0;
  if (Math.abs(monitor.height - (y + h)) <= SNAP_PX) y = monitor.height - h;

  // After snapping, so a deliberate edge snap is never pulled back off the edge.
  x = Math.min(Math.max(x, MIN_VISIBLE_PX - w), monitor.width - MIN_VISIBLE_PX);
  y = Math.min(Math.max(y, 0), monitor.height - MIN_VISIBLE_PX);

  await invoke("save_overlay_geometry", {
    id: widgetId,
    x,
    y,
    w,
    h,
    monitor: monitor.name,
  }).catch(() => {});

  const snappedX = Math.round(x * monitor.scale) + monitor.x;
  const snappedY = Math.round(y * monitor.scale) + monitor.y;
  if (snappedX !== pos.x || snappedY !== pos.y) {
    await win.setPosition(new PhysicalPosition(snappedX, snappedY)).catch(() => {});
  }
}

function schedulePersist(): void {
  window.clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => void persistGeometry(), PERSIST_DEBOUNCE_MS);
}

async function main(): Promise<void> {
  widgetId = await invoke<string | null>("overlay_widget_id").catch(() => null);
  const widget = widgetId ? widgetById(widgetId) : null;
  if (!widget) return;

  // Declared dims are the floor the user can shrink to, enforced natively so the
  // widget is never asked to render below the size it was authored for.
  const dims = overlayDims(widget);
  if (dims) await win.setMinSize(new LogicalSize(dims.widthCss, dims.heightCss)).catch(() => {});

  const mount = document.createElement("div");
  mount.className = "overlay-content";
  root.replaceChildren(mount);
  overlayRenderer(widget)(mount);

  void applySettings();
  listen("widgets-changed", () => void applySettings());

  // Anywhere on the background drags the window; the grip resizes from the corner.
  // Widgets with their own click targets opt out with .no-drag, same intent as the
  // strip's isDragging() guard.
  root.addEventListener("pointerdown", (e) => {
    const el = e.target as HTMLElement;
    if (e.button !== 0 || el.closest(".no-drag, button, a, input, select")) return;
    void win.startDragging();
  });
  grip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    void win.startResizeDragging("SouthEast");
  });

  root.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("show_tile_menu", {
      widgetId: widget.id,
      items: widget.menuItems?.() ?? [],
    }).catch(() => {});
  });

  void win.onMoved(schedulePersist);
  void win.onResized(schedulePersist);
}

void main();
