import { describe, expect, it } from "vitest";
import { FLYOUT_INSET_PX, panelBox, rowDelay, swapTimeline } from "./flyout-panel";

describe("rowDelay", () => {
  const t = { riseDelay: 120 };

  it("staggers rows that arrive on time", () => {
    expect(rowDelay(t, 0, 0)).toBe(120);
    expect(rowDelay(t, 3, 0)).toBe(120 + 3 * 26);
  });

  // The regression: the top-processes block lands ~100 ms after mount and used to pop in at once.
  it("gives a late row the next slot in the sequence, not zero", () => {
    expect(rowDelay(t, 2, 100)).toBe(120 + 2 * 26 - 100);
  });

  it("never goes negative for a row that arrives after its slot", () => {
    expect(rowDelay(t, 1, 900)).toBe(0);
  });
});

const show = { widget_id: "cpu", left_css: 100, width_css: 245, height_css: 270 };

describe("panelBox", () => {
  it("insets the visible panel by .flyout-body's padding on every side", () => {
    const b = panelBox(show);
    expect(b.left).toBe(100 + FLYOUT_INSET_PX);
    expect(b.width).toBe(245 - 2 * FLYOUT_INSET_PX);
    expect(b.height).toBe(270 - 2 * FLYOUT_INSET_PX);
  });

  // The content layer must equal what a bare #flyout measured to, or conductor's
  // measured height (which includes the 8px wrapper padding) stops fitting exactly.
  it("sizes the content layer to the panel minus its padding and border", () => {
    const b = panelBox(show);
    expect(b.innerWidth).toBe(b.width - 30);
    expect(b.innerHeight).toBe(b.height - 30);
  });

  it("never goes negative for a degenerate size", () => {
    const b = panelBox({ ...show, width_css: 4, height_css: 10 });
    expect(b.width).toBe(0);
    expect(b.height).toBe(2);
    expect(b.innerWidth).toBe(0);
    expect(b.innerHeight).toBe(0);
  });
});

describe("swapTimeline", () => {
  it("starts the new rows only once the old ones have faded", () => {
    const t = swapTimeline(300, true);
    expect(t.fadeOut).toBe(120);
    expect(t.riseDelay).toBe(t.fadeOut);
  });

  it("rises sooner when there is nothing to fade out", () => {
    expect(swapTimeline(300, false).riseDelay).toBeLessThan(swapTimeline(300, true).riseDelay);
  });

  it("collapses to zero for reduced motion", () => {
    expect(swapTimeline(0, true)).toEqual({ fadeOut: 0, riseDelay: 0, rise: 0 });
  });
});
