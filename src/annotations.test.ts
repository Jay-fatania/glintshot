import { describe, expect, it } from "vitest";
import { Annotations, rectFrom, type Shape } from "./annotations";

const rect = (x: number, y: number): Shape => ({
  kind: "rect",
  a: { x, y },
  b: { x: x + 10, y: y + 10 },
  color: "#f00",
  width: 2,
});

const marker = (n: number): Shape => ({ kind: "marker", at: { x: 0, y: 0 }, n, color: "#f00", size: 14 });

describe("rectFrom", () => {
  it("normalises corners dragged in any direction", () => {
    expect(rectFrom({ x: 10, y: 20 }, { x: 4, y: 2 })).toEqual({ x: 4, y: 2, width: 6, height: 18 });
    expect(rectFrom({ x: 4, y: 2 }, { x: 10, y: 20 })).toEqual({ x: 4, y: 2, width: 6, height: 18 });
  });

  it("gives a zero-size rect for a click", () => {
    expect(rectFrom({ x: 5, y: 5 }, { x: 5, y: 5 })).toEqual({ x: 5, y: 5, width: 0, height: 0 });
  });
});

describe("Annotations undo and redo", () => {
  it("undo removes the last shape and redo restores it", () => {
    const a = new Annotations();
    a.add(rect(0, 0));
    a.add(rect(1, 1));
    a.undo();
    expect(a.shapes).toHaveLength(1);
    a.redo();
    expect(a.shapes).toHaveLength(2);
    expect(a.canRedo).toBe(false);
  });

  it("a new shape clears the redo stack", () => {
    const a = new Annotations();
    a.add(rect(0, 0));
    a.undo();
    expect(a.canRedo).toBe(true);
    a.add(rect(2, 2));
    expect(a.canRedo).toBe(false);
  });

  it("undo and redo do nothing when there is nothing to step through", () => {
    const a = new Annotations();
    expect(a.canUndo).toBe(false);
    a.undo();
    a.redo();
    expect(a.shapes).toHaveLength(0);
  });
});

describe("Annotations.nextMarkerNumber", () => {
  it("starts at 1 and counts up", () => {
    const a = new Annotations();
    expect(a.nextMarkerNumber()).toBe(1);
    a.add(marker(1));
    a.add(marker(2));
    expect(a.nextMarkerNumber()).toBe(3);
  });

  it("never reuses a number after a marker is undone from the middle", () => {
    // Regression: the count-based version gave 3 here, duplicating marker 3.
    const a = new Annotations();
    a.add(marker(1));
    a.add(marker(2));
    a.add(marker(3));
    a.shapes.splice(1, 1); // delete marker 2 without touching the redo stack
    expect(a.nextMarkerNumber()).toBe(4);
  });

  it("ignores non-marker shapes", () => {
    const a = new Annotations();
    a.add(rect(0, 0));
    expect(a.nextMarkerNumber()).toBe(1);
  });
});
