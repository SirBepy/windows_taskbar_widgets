import { describe, expect, it } from "vitest";
import {
  fmtClock,
  overlayDims,
  overlayRenderer,
  placementOf,
  stripNeedsRemount,
  stripTileIds,
  type Settings,
  type TaskbarWidget,
} from "./widget";

const noop = () => () => {};
// Tests run in vitest's node environment, so there is no real DOM to hand a renderer.
const root = {} as HTMLElement;

function widget(extra: Partial<TaskbarWidget> = {}): TaskbarWidget {
  return { id: "w", name: "W", mountTile: noop, ...extra };
}

describe("placementOf", () => {
  it("defaults to the strip when the widget has no entry", () => {
    expect(placementOf({ widget_placement: {} } as Settings, "cpu")).toEqual({ kind: "strip" });
    expect(placementOf(null, "cpu")).toEqual({ kind: "strip" });
  });

  it("returns the stored overlay spec", () => {
    const settings = {
      widget_placement: { cpu: { kind: "overlay", monitor: "", x: 10, y: 20 } },
    } as unknown as Settings;

    expect(placementOf(settings, "cpu")).toEqual({ kind: "overlay", monitor: "", x: 10, y: 20 });
  });
});

describe("stripTileIds", () => {
  it("returns all ids when nothing is overlay-placed", () => {
    expect(stripTileIds(["cpu", "ram"], null)).toEqual(["cpu", "ram"]);
  });

  it("drops ids placed as a floating overlay", () => {
    const settings = {
      widget_placement: { cpu: { kind: "overlay", monitor: "", x: 0, y: 0 } },
    } as unknown as Settings;

    expect(stripTileIds(["cpu", "ram"], settings)).toEqual(["ram"]);
  });
});

describe("stripNeedsRemount", () => {
  it("is false when membership and order are unchanged", () => {
    expect(stripNeedsRemount(["cpu", "ram"], ["cpu", "ram"])).toBe(false);
  });

  it("is true on a length change", () => {
    expect(stripNeedsRemount(["cpu"], ["cpu", "ram"])).toBe(true);
  });

  it("is true on a reorder even with the same ids", () => {
    expect(stripNeedsRemount(["cpu", "ram"], ["ram", "cpu"])).toBe(true);
  });
});

describe("overlayDims", () => {
  it("prefers declared overlay dims over the flyout's", () => {
    const w = widget({
      overlay: { widthCss: 400, heightCss: 300 },
      flyout: { widthCss: 245, heightCss: 270 },
    });

    expect(overlayDims(w)).toEqual({ widthCss: 400, heightCss: 300 });
  });

  it("falls back to the flyout's dims", () => {
    expect(overlayDims(widget({ flyout: { widthCss: 245, heightCss: 270 } }))).toEqual({
      widthCss: 245,
      heightCss: 270,
    });
  });

  // Size never falls back to a measured tile; a widget with neither is not overlay-placeable.
  it("returns null when the widget declares no size at all", () => {
    expect(overlayDims(widget())).toBeNull();
  });
});

describe("fmtClock", () => {
  it("defaults to unpadded minutes with no hours branch (Spotify's usage)", () => {
    expect(fmtClock(0)).toBe("0:00");
    expect(fmtClock(30_000)).toBe("0:30");
    expect(fmtClock(330_000)).toBe("5:30");
    expect(fmtClock(60_000)).toBe("1:00");
    expect(fmtClock(3_600_000)).toBe("60:00");
  });

  it("padMinutes zero-pads mm without enabling an hours branch", () => {
    expect(fmtClock(30_000, { padMinutes: true })).toBe("00:30");
    expect(fmtClock(3_600_000, { padMinutes: true })).toBe("60:00");
  });

  it("hours adds an h:mm:ss branch once the duration reaches an hour", () => {
    expect(fmtClock(3_599_000, { padMinutes: true, hours: true })).toBe("59:59");
    expect(fmtClock(3_600_000, { padMinutes: true, hours: true })).toBe("1:00:00");
    expect(fmtClock(35_999_000, { padMinutes: true, hours: true })).toBe("9:59:59");
  });

  it("padMinutes and hours are independent flags", () => {
    expect(fmtClock(3_665_000, { hours: true })).toBe("1:1:05");
  });
});

describe("overlayRenderer", () => {
  it("prefers mountOverlay", () => {
    const calls: string[] = [];
    const w = widget({
      mountTile: () => (calls.push("tile"), noop()),
      mountFlyout: () => (calls.push("flyout"), noop()),
      mountOverlay: () => (calls.push("overlay"), noop()),
    });

    overlayRenderer(w)(root);

    expect(calls).toEqual(["overlay"]);
  });

  it("falls back to mountFlyout, then to mountTile", () => {
    const withFlyout: string[] = [];
    overlayRenderer(
      widget({
        mountTile: () => (withFlyout.push("tile"), noop()),
        mountFlyout: () => (withFlyout.push("flyout"), noop()),
      }),
    )(root);

    const tileOnly: string[] = [];
    overlayRenderer(widget({ mountTile: () => (tileOnly.push("tile"), noop()) }))(root);

    expect(withFlyout).toEqual(["flyout"]);
    expect(tileOnly).toEqual(["tile"]);
  });
});
