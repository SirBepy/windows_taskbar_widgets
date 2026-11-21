import { createFlyoutController } from "./flyout-mount";

/** What `open_flyout` hands the page: the panel, in the flyout window's own CSS pixels. */
export interface PanelShow {
  widget_id: string;
  left_css: number;
  width_css: number;
  height_css: number;
}

export const FLYOUT_MS = 300;
export const FLYOUT_EASE = "cubic-bezier(.65, 0, .35, 1)";
const ROW_STAGGER_MS = 26;
const ROW_RISE_PX = 9;
/** `.flyout-body`'s padding. A widget's declared flyout dims include it, so the visible panel is inset. */
export const FLYOUT_INSET_PX = 4;
/** `#flyout`'s own padding (14 x 2) plus border (1 x 2): what the content layer loses to the panel's chrome. */
const CHROME_PX = 30;

export interface PanelBox {
  left: number;
  width: number;
  height: number;
  innerWidth: number;
  innerHeight: number;
}

/** Pure: the visible panel's box and its content layer's size, from the declared dims. */
export function panelBox(p: PanelShow): PanelBox {
  const width = Math.max(0, p.width_css - 2 * FLYOUT_INSET_PX);
  const height = Math.max(0, p.height_css - 2 * FLYOUT_INSET_PX);
  return {
    left: p.left_css + FLYOUT_INSET_PX,
    width,
    height,
    innerWidth: Math.max(0, width - CHROME_PX),
    innerHeight: Math.max(0, height - CHROME_PX),
  };
}

/**
 * Pure: one content swap in ms. The old rows fade out while the panel resizes, and the new
 * rows only start rising once the resize has finished, so content never lands in a half-sized box.
 */
export function swapTimeline(durMs: number) {
  return {
    fadeOut: Math.round(durMs * 0.4),
    riseDelay: durMs,
    rise: Math.round(durMs * 0.75),
  };
}

/**
 * Pure: ms after mount at which a row starts rising. Rows queue one stagger behind the previous
 * row, and never start before they exist: a block that lands after the first slots have passed
 * (the data arrives ~250 ms after mount, past the end of an anchored-to-mount stagger) still
 * rises row by row instead of every row starting in the same frame.
 */
export function rowStart(
  t: { riseDelay: number },
  prevStart: number | null,
  elapsedMs: number,
): number {
  const earliest = prevStart === null ? t.riseDelay : prevStart + ROW_STAGGER_MS;
  return Math.max(earliest, elapsedMs);
}

/** How long after mount a newly added row still counts as part of the swap. */
const LATE_ROW_WINDOW_MS = 1500;

export interface FlyoutPanelDeps {
  panel: HTMLElement;
  mountWidget: (id: string, root: HTMLElement) => (() => void) | null;
  /** The close animation finished; the native window can go. */
  onHidden: () => void;
}

/**
 * The one flyout element. It never goes away between tiles: its box animates to each widget's
 * size, and its content swaps inside it. Only `hide()` collapses it.
 */
export function createFlyoutPanel({ panel, mountWidget, onHidden }: FlyoutPanelDeps) {
  const duration = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : FLYOUT_MS);
  let layer: HTMLElement | null = null;
  let box: PanelBox | null = null;
  let open = false;
  let hideTimer: number | undefined;

  const rows = (root: HTMLElement) => Array.from(root.children) as HTMLElement[];

  // Fresh layer per widget: lit-html caches its ChildPart on the render target, so reusing a
  // wiped node renders into detached DOM (blank panel).
  const controller = createFlyoutController((id) => {
    const old = layer;
    const next = document.createElement("div");
    next.className = "flyout-content fly-layer";
    // Sized to the target, not left at the panel's animating size: a layer that tracked the
    // panel would reflow its rows and flash a scrollbar mid-animation.
    next.style.width = `${box!.innerWidth}px`;
    next.style.height = `${box!.innerHeight}px`;
    panel.appendChild(next);
    layer = next;
    const cleanup = mountWidget(id, next);
    swapRows(old, next);
    return cleanup;
  });

  function swapRows(old: HTMLElement | null, next: HTMLElement) {
    const d = duration();
    if (d === 0) {
      old?.remove();
      return;
    }
    const t = swapTimeline(d);
    if (old) {
      const fade = old.animate([{ opacity: 1 }, { opacity: 0 }], { duration: t.fadeOut, fill: "forwards" });
      fade.onfinish = () => old.remove();
      window.setTimeout(() => old.remove(), t.fadeOut + 60);
    }
    const mountedAt = performance.now();
    let prevStart: number | null = null;
    const rise = (row: HTMLElement) => {
      const elapsed = performance.now() - mountedAt;
      const start = rowStart(t, prevStart, elapsed);
      prevStart = start;
      return row.animate(
        [
          { opacity: 0, transform: `translateY(${ROW_RISE_PX}px)` },
          { opacity: 1, transform: "translateY(0)" },
        ],
        { duration: t.rise, delay: start - elapsed, easing: FLYOUT_EASE, fill: "backwards" },
      );
    };
    rows(next).forEach(rise);
    // Rows the widget adds after its first paint join the same sequence.
    const late = new MutationObserver((records) => {
      for (const r of records) {
        r.addedNodes.forEach((n) => {
          if (n instanceof HTMLElement) rise(n);
        });
      }
    });
    late.observe(next, { childList: true });
    window.setTimeout(() => late.disconnect(), LATE_ROW_WINDOW_MS);
  }

  function transitions(on: boolean, glide: boolean) {
    const d = duration();
    if (!on || d === 0) {
      panel.style.transition = "none";
      return;
    }
    const props = [`height ${d}ms ${FLYOUT_EASE}`, `opacity ${Math.round(d * 0.7)}ms ease`];
    if (glide) props.push(`left ${d}ms ${FLYOUT_EASE}`, `width ${d}ms ${FLYOUT_EASE}`);
    panel.style.transition = props.join(", ");
  }

  return {
    show(p: PanelShow): void {
      window.clearTimeout(hideTimer);
      hideTimer = undefined;
      const wasOpen = open;
      box = panelBox(p);
      if (!wasOpen) {
        // First open: place the collapsed box without a sideways animation, then grow it.
        transitions(false, false);
        panel.style.left = `${box.left}px`;
        panel.style.width = `${box.width}px`;
        panel.style.height = "0px";
        panel.style.opacity = "0";
        panel.style.visibility = "visible";
        void panel.offsetHeight;
      }
      transitions(true, wasOpen);
      panel.style.left = `${box.left}px`;
      panel.style.width = `${box.width}px`;
      panel.style.height = `${box.height}px`;
      panel.style.opacity = "1";
      open = true;
      controller.show(p.widget_id);
    },

    hide(): void {
      if (!open) return;
      open = false;
      // Unmounting now, not after the animation: it is what stops the widget's polling.
      controller.hide();
      transitions(true, false);
      panel.style.height = "0px";
      panel.style.opacity = "0";
      const d = duration();
      const dying = layer;
      dying?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: Math.round(d * 0.4), fill: "forwards" });
      hideTimer = window.setTimeout(() => {
        panel.replaceChildren();
        layer = null;
        panel.style.visibility = "hidden";
        onHidden();
      }, d + 30);
    },

    isOpen: (): boolean => open,
  };
}
