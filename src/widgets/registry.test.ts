import { describe, expect, it, vi } from "vitest";

// Importing registry.ts pulls in every widget module, and some call invoke/listen
// at import time (disk.ts's module-scope subscribeStats), which needs `window`.
// mem_total_bytes: 0 short-circuits disk.ts's own guard, keeping that call a no-op.
vi.mock("@tauri-apps/api/core", () => ({ invoke: () => Promise.resolve({ mem_total_bytes: 0 }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

import { makeDividerId } from "../shared/divider";
import { allWidgetIds, widgetById, widgetsFor } from "./registry";

describe("registry routes divider ids alongside real widgets", () => {
  it("widgetsFor keeps divider entries in place, duplicates included", () => {
    const d1 = makeDividerId();
    const d2 = makeDividerId();
    const ids = ["cpu", d1, "ram", d2];
    expect(widgetsFor(ids).map((w) => w.id)).toEqual(ids);
  });

  it("widgetById synthesizes a divider widget with no flyout/menu/config", () => {
    const id = makeDividerId();
    const w = widgetById(id);
    expect(w?.id).toBe(id);
    expect(w?.flyout).toBeUndefined();
    expect(w?.menuItems).toBeUndefined();
    expect(w?.configFields).toBeUndefined();
  });

  it("widgetsFor drops an id that matches neither a real widget nor the divider prefix", () => {
    expect(widgetsFor(["not-a-real-widget"])).toEqual([]);
  });

  // main.ts's adopt-new-ids step diffs enabled_widgets against allWidgetIds(), so a
  // divider id staying out of that list is what stops dividers being re-appended.
  it("allWidgetIds() never contains a divider id", () => {
    const dividerId = makeDividerId();
    expect(allWidgetIds()).not.toContain(dividerId);
    expect(allWidgetIds().every((id) => !id.startsWith("divider:"))).toBe(true);
  });
});
