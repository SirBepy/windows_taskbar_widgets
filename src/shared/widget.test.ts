import { describe, expect, it } from "vitest";
import {
  overlayDims,
  overlayRenderer,
  placementOf,
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
