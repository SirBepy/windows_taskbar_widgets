import { setDragging } from "./widget";

const DRAG_THRESHOLD_PX = 6;

/** Moves `tile` at most one slot per call, toward wherever the pointer center
 * now sits; repeated pointermove calls converge it to the right spot. */
function reorderByPointer(row: HTMLElement, tile: HTMLElement, clientX: number) {
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

export interface WireDragOptions {
  /** Fires once a drag actually starts (threshold crossed), before any reordering. */
  onDragStart?: () => void;
  /** Fires on release, with `row.children`'s ids read back in their new DOM order. */
  onReorder: (order: string[]) => void;
  /** Defaults to `el.dataset.widget`. */
  getId?: (el: HTMLElement) => string;
}

/** Pointer drag-to-reorder for one sibling of `row`. Live strip only (main.ts):
 * an instant one-slot swap, deliberately snappier than the settings screen's
 * gap-opening drag (widget-strip-drag.ts), since the strip is dragged far more. */
export function wireDragReorder(row: HTMLElement, tile: HTMLElement, opts: WireDragOptions): void {
  const getId = opts.getId ?? ((el) => el.dataset.widget!);
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
      opts.onDragStart?.();
      tile.setPointerCapture(pointerId);
      tile.classList.add("dragging");
    }
    reorderByPointer(row, tile, e.clientX);
  });

  const endDrag = (e: PointerEvent) => {
    if (pointerId === null || e.pointerId !== pointerId) return;
    if (isDragging) {
      tile.releasePointerCapture(pointerId);
      tile.classList.remove("dragging");
      const order = Array.from(row.children).map((el) => getId(el as HTMLElement));
      opts.onReorder(order);
    }
    isDragging = false;
    setDragging(false);
    pointerId = null;
  };
  tile.addEventListener("pointerup", endDrag);
  tile.addEventListener("pointercancel", endDrag);
}
