import { setDragging } from "../../shared/widget";
import { makeDividerId } from "../../shared/divider";
import { widgetById } from "../../widgets/registry";

export const NEW_DIVIDER = "new-divider";
const DRAG_THRESHOLD_PX = 6;
const SLIDE_MS = 190;
const PAD_X = 40;
// Stacked lanes sit ~38px apart, so one lane's 24px grab pad would reach into its
// neighbour and make the drop land on the wrong monitor. Only the sole-lane case
// keeps the generous pad it was tuned with.
const PAD_Y_SINGLE = 24;
const PAD_Y_STACKED = 8;

interface Drag {
  id: string;
  dropId: string;
  source: HTMLElement;
  fromMonitor: string | null;
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
  over?: HTMLElement;
}

interface StripDragCallbacks {
  onSelect: (id: string) => void;
  onDrop: (dropId: string, from: string | null, to: string, index: number) => void;
  onRemove: (id: string, monitor: string) => void;
  onCancel: () => void;
}

let lanesEl: HTMLElement | null = null;
let paletteEl: HTMLElement | null = null;
let callbacks: StripDragCallbacks | null = null;
let drag: Drag | null = null;

export function isStripDragActive(): boolean {
  return drag !== null;
}

/** Every lane's strip, in stacked order. Read live rather than cached: the lane set is
 * re-rendered whenever the monitor list changes, which would strand a cached array. */
function strips(): HTMLElement[] {
  return [...(lanesEl?.querySelectorAll<HTMLElement>(".wsf-strip") ?? [])];
}

function monitorOf(strip: HTMLElement): string {
  return strip.dataset.monitor ?? "";
}

/** Animates every tile from where it sat before `mutate` to where it lands after,
 * so the row visibly slides apart around the drop slot instead of jumping. */
function flip(strip: HTMLElement, mutate: () => void): void {
  const els = [...strip.children] as HTMLElement[];
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
function measureTile(strip: HTMLElement, id: string): number {
  const probe = document.createElement("div");
  probe.className = "tile";
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none";
  strip.appendChild(probe);
  const stop = widgetById(id)?.mountTile(probe);
  const width = probe.getBoundingClientRect().width;
  stop?.();
  probe.remove();
  return width;
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

/** The lane the pointer is over. Padded rects can overlap once lanes are stacked, so
 * ties go to the lane whose centre line is nearest. */
function stripAt(e: PointerEvent): HTMLElement | null {
  const all = strips();
  const padY = all.length > 1 ? PAD_Y_STACKED : PAD_Y_SINGLE;
  let best: HTMLElement | null = null;
  let bestDistance = Infinity;
  for (const strip of all) {
    if (!hits(strip, e, PAD_X, padY)) continue;
    const r = strip.getBoundingClientRect();
    const distance = Math.abs(e.clientY - (r.top + r.height / 2));
    if (distance < bestDistance) {
      bestDistance = distance;
      best = strip;
    }
  }
  return best;
}

function begin(e: PointerEvent): void {
  const d = drag!;
  const home = d.source.closest<HTMLElement>(".wsf-strip") ?? stripAt(e) ?? strips()[0];
  const clone = document.createElement("div");
  clone.className = "tile wsf-clone";
  d.cloneDispose = widgetById(d.dropId)?.mountTile(clone);
  const fromStrip = d.fromMonitor !== null;
  const baseW = fromStrip ? d.w : measureTile(home, d.dropId);
  clone.style.width = `${baseW}px`;
  clone.style.height = `${fromStrip ? d.h : home.getBoundingClientRect().height}px`;
  document.body.appendChild(clone);
  d.clone = clone;
  if (!fromStrip) {
    d.dx = baseW / 2;
    d.dy = clone.getBoundingClientRect().height / 2;
  }

  // Read the painted rect, not baseW: the clone's scale(1.04) pick-up affordance
  // makes it wider than its layout box, and a narrower gap reads as a mismatch.
  d.slot = document.createElement("div");
  d.slot.className = "wsf-slot";
  d.slot.style.width = `${clone.getBoundingClientRect().width}px`;

  if (fromStrip) {
    d.over = home;
    flip(home, () => {
      home.insertBefore(d.slot!, d.source);
      d.source.classList.add("wsf-lifted");
    });
  } else {
    d.source.classList.add("wsf-lifted");
  }
  setDragging(true);
}

function moveSlot(strip: HTMLElement, x: number): void {
  const d = drag!;
  const tiles = ([...strip.children] as HTMLElement[]).filter(
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
  if (d.slot!.parentElement === strip && d.slot!.nextElementSibling === target) return;
  if (!target && d.slot!.parentElement === strip && !d.slot!.nextElementSibling) return;
  flip(strip, () => strip.insertBefore(d.slot!, target));
}

function detachSlot(): void {
  const d = drag!;
  const from = d.slot!.parentElement as HTMLElement | null;
  if (!from) return;
  flip(from, () => d.slot!.remove());
}

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0) return;
  const el = (e.target as HTMLElement).closest<HTMLElement>("[data-widget]");
  if (!el) return;
  const id = el.dataset.widget!;
  const r = el.getBoundingClientRect();
  const home = el.closest<HTMLElement>(".wsf-strip");
  drag = {
    id,
    dropId: id === NEW_DIVIDER ? makeDividerId() : id,
    source: el,
    fromMonitor: home ? monitorOf(home) : null,
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
  if (!d || !lanesEl || !paletteEl) return;
  if (!d.started) {
    if (Math.hypot(e.clientX - d.ox, e.clientY - d.oy) < DRAG_THRESHOLD_PX) return;
    d.started = true;
    begin(e);
  }
  d.clone!.style.left = `${e.clientX - d.dx}px`;
  d.clone!.style.top = `${e.clientY - d.dy}px`;

  const over = stripAt(e);
  d.over = over ?? undefined;
  for (const strip of strips()) strip.classList.toggle("wsf-drop", strip === over);
  paletteEl.classList.toggle("wsf-drop", !over && hits(paletteEl, e, 0, 20));
  d.clone!.classList.toggle("wsf-removing", d.fromMonitor !== null && !over);
  if (over) moveSlot(over, e.clientX);
  else detachSlot();
}

function onPointerUp(): void {
  window.removeEventListener("pointermove", onPointerMove);
  const d = drag;
  drag = null;
  if (!d) return;
  setDragging(false);

  if (!d.started) {
    if (d.fromMonitor !== null) callbacks!.onSelect(d.id);
    return;
  }

  for (const strip of strips()) strip.classList.remove("wsf-drop");
  paletteEl?.classList.remove("wsf-drop");
  d.cloneDispose?.();
  d.clone?.remove();
  d.source.classList.remove("wsf-lifted");

  const landedIn = d.slot!.parentElement as HTMLElement | null;
  // Counted among the non-lifted children, which is exactly what the drop callback
  // expects - and the lifted source only sits in its own lane, so a cross-lane drop
  // needs no shift correction either.
  const index = landedIn
    ? ([...landedIn.children] as HTMLElement[])
        .filter((t) => !t.classList.contains("wsf-lifted"))
        .indexOf(d.slot!)
    : -1;
  d.slot!.remove();

  if (landedIn) callbacks!.onDrop(d.dropId, d.fromMonitor, monitorOf(landedIn), index);
  else if (d.fromMonitor !== null) callbacks!.onRemove(d.id, d.fromMonitor);
  else callbacks!.onCancel();
}

/** Wires pointer-drag reordering onto the lane container and palette. Listens on the
 * container, not on each strip, so re-rendering the lanes (a monitor plugged in) needs
 * no re-wiring. */
export function wireStripDrag(lanes: HTMLElement, palette: HTMLElement, cb: StripDragCallbacks): void {
  lanesEl = lanes;
  paletteEl = palette;
  callbacks = cb;
  lanes.addEventListener("pointerdown", onPointerDown);
  palette.addEventListener("pointerdown", onPointerDown);
}
