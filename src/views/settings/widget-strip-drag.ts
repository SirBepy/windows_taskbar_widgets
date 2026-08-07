import { setDragging } from "../../shared/widget";
import { makeDividerId } from "../../shared/divider";
import { widgetById } from "../../widgets/registry";

export const NEW_DIVIDER = "new-divider";
const DRAG_THRESHOLD_PX = 6;
const SLIDE_MS = 190;

interface Drag {
  id: string;
  dropId: string;
  source: HTMLElement;
  fromStrip: boolean;
  started: boolean;
  ox: number;
  oy: number;
  dx: number;
  dy: number;
  w: number;
  h: number;
  clone?: HTMLElement;
  cloneDispose?: () => void;
  slot?: HTMLElement;
}

interface StripDragCallbacks {
  onSelect: (id: string) => void;
  onDrop: (dropId: string, index: number) => void;
  onRemove: (id: string) => void;
  onCancel: () => void;
}

let stripEl: HTMLElement | null = null;
let paletteEl: HTMLElement | null = null;
let callbacks: StripDragCallbacks | null = null;
let drag: Drag | null = null;

export function isStripDragActive(): boolean {
  return drag !== null;
}

/** Animates every tile from where it sat before `mutate` to where it lands after,
 * so the row visibly slides apart around the drop slot instead of jumping. */
function flip(mutate: () => void): void {
  const els = [...stripEl!.children] as HTMLElement[];
  const before = els.map((el) => el.getBoundingClientRect().left);
  mutate();
  els.forEach((el, i) => {
    const shift = before[i] - el.getBoundingClientRect().left;
    if (!shift) return;
    el.style.transition = "none";
    el.style.transform = `translateX(${shift}px)`;
  });
  requestAnimationFrame(() => {
    for (const el of els) {
      el.style.transition = `transform ${SLIDE_MS}ms cubic-bezier(.2, .8, .3, 1)`;
      el.style.transform = "";
    }
  });
}

/** Width a palette entry will occupy once it's a real tile - a chip is narrower,
 * so measuring the chip would open a gap that doesn't match what lands. */
function measureTile(id: string): number {
  const probe = document.createElement("div");
  probe.className = "tile";
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none";
  stripEl!.appendChild(probe);
  const stop = widgetById(id)?.mountTile(probe);
  const width = probe.getBoundingClientRect().width;
  stop?.();
  probe.remove();
  return width;
}

function begin(): void {
  const d = drag!;
  const clone = document.createElement("div");
  clone.className = "tile wsf-clone";
  d.cloneDispose = widgetById(d.dropId)?.mountTile(clone);
  const baseW = d.fromStrip ? d.w : measureTile(d.dropId);
  clone.style.width = `${baseW}px`;
  clone.style.height = `${d.fromStrip ? d.h : stripEl!.getBoundingClientRect().height}px`;
  document.body.appendChild(clone);
  d.clone = clone;
  if (!d.fromStrip) {
    d.dx = baseW / 2;
    d.dy = clone.getBoundingClientRect().height / 2;
  }

  // Read the painted rect, not baseW: the clone's scale(1.04) pick-up affordance
  // makes it wider than its layout box, and a narrower gap reads as a mismatch.
  d.slot = document.createElement("div");
  d.slot.className = "wsf-slot";
  d.slot.style.width = `${clone.getBoundingClientRect().width}px`;

  if (d.fromStrip) {
    flip(() => {
      stripEl!.insertBefore(d.slot!, d.source);
      d.source.classList.add("wsf-lifted");
    });
  } else {
    d.source.classList.add("wsf-lifted");
  }
  setDragging(true);
}

function hits(el: HTMLElement, e: PointerEvent, padX: number, padY: number): boolean {
  const r = el.getBoundingClientRect();
  return (
    e.clientX >= r.left - padX &&
    e.clientX <= r.right + padX &&
    e.clientY >= r.top - padY &&
    e.clientY <= r.bottom + padY
  );
}

function moveSlot(x: number): void {
  const d = drag!;
  const tiles = ([...stripEl!.children] as HTMLElement[]).filter(
    (t) => t !== d.slot && !t.classList.contains("wsf-lifted"),
  );
  let target: HTMLElement | null = null;
  for (const t of tiles) {
    const r = t.getBoundingClientRect();
    if (x < r.left + r.width / 2) {
      target = t;
      break;
    }
  }
  if (d.slot!.parentElement && d.slot!.nextElementSibling === target) return;
  if (!target && d.slot!.parentElement && !d.slot!.nextElementSibling) return;
  flip(() => stripEl!.insertBefore(d.slot!, target));
}

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0 || !stripEl) return;
  const el = (e.target as HTMLElement).closest<HTMLElement>("[data-widget]");
  if (!el) return;
  const id = el.dataset.widget!;
  const r = el.getBoundingClientRect();
  drag = {
    id,
    dropId: id === NEW_DIVIDER ? makeDividerId() : id,
    source: el,
    fromStrip: el.parentElement === stripEl,
    started: false,
    ox: e.clientX,
    oy: e.clientY,
    dx: e.clientX - r.left,
    dy: e.clientY - r.top,
    w: r.width,
    h: r.height,
  };
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}

function onPointerMove(e: PointerEvent): void {
  const d = drag;
  if (!d || !stripEl || !paletteEl) return;
  if (!d.started) {
    if (Math.hypot(e.clientX - d.ox, e.clientY - d.oy) < DRAG_THRESHOLD_PX) return;
    d.started = true;
    begin();
  }
  d.clone!.style.left = `${e.clientX - d.dx}px`;
  d.clone!.style.top = `${e.clientY - d.dy}px`;

  const overStrip = hits(stripEl, e, 40, 24);
  stripEl.classList.toggle("wsf-drop", overStrip);
  paletteEl.classList.toggle("wsf-drop", !overStrip && hits(paletteEl, e, 0, 20));
  d.clone!.classList.toggle("wsf-removing", d.fromStrip && !overStrip);
  if (overStrip) moveSlot(e.clientX);
  else if (d.slot!.parentElement) flip(() => d.slot!.remove());
}

function onPointerUp(): void {
  window.removeEventListener("pointermove", onPointerMove);
  const d = drag;
  drag = null;
  if (!d) return;
  setDragging(false);

  if (!d.started) {
    if (d.fromStrip) callbacks!.onSelect(d.id);
    return;
  }

  stripEl?.classList.remove("wsf-drop");
  paletteEl?.classList.remove("wsf-drop");
  d.cloneDispose?.();
  d.clone?.remove();
  d.source.classList.remove("wsf-lifted");

  const landed = !!d.slot!.parentElement;
  // Counted among the non-lifted children, which is exactly what the drop callback expects.
  const index = landed
    ? ([...stripEl!.children] as HTMLElement[])
        .filter((t) => !t.classList.contains("wsf-lifted"))
        .indexOf(d.slot!)
    : -1;
  d.slot!.remove();

  if (landed) callbacks!.onDrop(d.dropId, index);
  else if (d.fromStrip) callbacks!.onRemove(d.id);
  else callbacks!.onCancel();
}

/** Wires pointer-drag reordering onto a freshly-mounted strip/palette pair. */
export function wireStripDrag(strip: HTMLElement, palette: HTMLElement, cb: StripDragCallbacks): void {
  stripEl = strip;
  paletteEl = palette;
  callbacks = cb;
  strip.addEventListener("pointerdown", onPointerDown);
  palette.addEventListener("pointerdown", onPointerDown);
}
