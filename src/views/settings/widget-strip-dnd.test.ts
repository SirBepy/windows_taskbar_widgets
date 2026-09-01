import { describe, expect, it } from "vitest";
import { insertAt, placeAt, removeFirst, removeId } from "./widget-strip-dnd";

const ORDER = ["cpu", "ram", "divider:a", "gpu"];

describe("placeAt", () => {
  it("moves a tile forward without the removal shifting it one slot short", () => {
    expect(placeAt(ORDER, "cpu", 2)).toEqual(["ram", "divider:a", "cpu", "gpu"]);
  });

  it("moves a tile backward", () => {
    expect(placeAt(ORDER, "gpu", 0)).toEqual(["gpu", "cpu", "ram", "divider:a"]);
  });

  it("inserts an id that was not in the order yet", () => {
    expect(placeAt(ORDER, "disk", 1)).toEqual(["cpu", "disk", "ram", "divider:a", "gpu"]);
  });

  it("appends when the index is past the end", () => {
    expect(placeAt(ORDER, "disk", 99)).toEqual([...ORDER, "disk"]);
  });

  it("clamps a negative index to the front", () => {
    expect(placeAt(ORDER, "disk", -3)).toEqual(["disk", ...ORDER]);
  });

  it("is a no-op when a tile lands back on its own index", () => {
    expect(placeAt(ORDER, "ram", 1)).toEqual(ORDER);
  });

  it("keeps duplicate divider instances distinct", () => {
    const two = ["divider:a", "cpu", "divider:b"];
    expect(placeAt(two, "divider:b", 0)).toEqual(["divider:b", "divider:a", "cpu"]);
  });
});

describe("removeId", () => {
  it("drops only the named id", () => {
    expect(removeId(ORDER, "divider:a")).toEqual(["cpu", "ram", "gpu"]);
  });

  it("leaves the order untouched when the id is absent", () => {
    expect(removeId(ORDER, "disk")).toEqual(ORDER);
  });
});

describe("removeFirst", () => {
  // A lane can hold two copies of one kind, and both are the same string, so a drag
  // of the second copy must not take the first one with it.
  it("drops one copy and leaves the sibling", () => {
    expect(removeFirst(["cpu", "ram", "cpu"], "cpu")).toEqual(["ram", "cpu"]);
  });

  it("leaves the order untouched when the id is absent", () => {
    expect(removeFirst(ORDER, "disk")).toEqual(ORDER);
  });
});

describe("insertAt", () => {
  it("inserts at the index", () => {
    expect(insertAt(["cpu", "ram"], "gpu", 1)).toEqual(["cpu", "gpu", "ram"]);
  });

  it("clamps past the end and before the front", () => {
    expect(insertAt(["cpu"], "gpu", 99)).toEqual(["cpu", "gpu"]);
    expect(insertAt(["cpu"], "gpu", -3)).toEqual(["gpu", "cpu"]);
  });

  it("allows a second copy of a kind already in the lane", () => {
    expect(insertAt(["cpu", "ram"], "cpu", 2)).toEqual(["cpu", "ram", "cpu"]);
  });
});
