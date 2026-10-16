import { describe, expect, it } from "vitest";
import { renderShapes, type Shape } from "./annotations";

// Records the canvas calls made while drawing, with the style that was active
// at the moment of each call. Enough to check what a shape paints without a DOM.
function fakeContext() {
  const calls: { name: string; args: unknown[]; fillStyle: unknown; globalAlpha: number }[] = [];
  const state = { fillStyle: "#000000" as unknown, globalAlpha: 1 };
  const record =
    (name: string) =>
    (...args: unknown[]) =>
      calls.push({ name, args, fillStyle: state.fillStyle, globalAlpha: state.globalAlpha });
  const ctx = {
    save: record("save"),
    restore: record("restore"),
    beginPath: record("beginPath"),
    closePath: record("closePath"),
    moveTo: record("moveTo"),
    lineTo: record("lineTo"),
    arc: record("arc"),
    ellipse: record("ellipse"),
    stroke: record("stroke"),
    fill: record("fill"),
    strokeRect: record("strokeRect"),
    fillRect: record("fillRect"),
    fillText: record("fillText"),
    strokeText: record("strokeText"),
  } as Record<string, unknown>;
  Object.defineProperties(ctx, {
    fillStyle: { get: () => state.fillStyle, set: (v) => (state.fillStyle = v), enumerable: true },
    globalAlpha: { get: () => state.globalAlpha, set: (v) => (state.globalAlpha = v), enumerable: true },
  });
  return { ctx: ctx as unknown as CanvasRenderingContext2D, calls };
}

const source = {} as HTMLCanvasElement;

describe("renderShapes", () => {
  it("paints redaction as solid black over the normalised rectangle", () => {
    const { ctx, calls } = fakeContext();
    const shape: Shape = { kind: "redact", a: { x: 30, y: 40 }, b: { x: 10, y: 20 }, color: "#f00", width: 2 };
    renderShapes(ctx, [shape], source);
    const fill = calls.find((c) => c.name === "fillRect");
    expect(fill?.args).toEqual([10, 20, 20, 20]);
    expect(fill?.fillStyle).toBe("#000");
  });

  it("paints highlight at 35% opacity and restores the state afterwards", () => {
    const { ctx, calls } = fakeContext();
    const shape: Shape = { kind: "highlight", a: { x: 0, y: 0 }, b: { x: 5, y: 5 }, color: "#ff0", width: 2 };
    renderShapes(ctx, [shape], source);
    const fill = calls.find((c) => c.name === "fillRect");
    expect(fill?.globalAlpha).toBeCloseTo(0.35);
    expect(calls[0].name).toBe("save");
    expect(calls[calls.length - 1].name).toBe("restore");
  });

  it("strokes a rectangle with the shape's colour and width", () => {
    const { ctx, calls } = fakeContext();
    renderShapes(ctx, [{ kind: "rect", a: { x: 1, y: 2 }, b: { x: 11, y: 12 }, color: "#0f0", width: 4 }], source);
    expect(calls.find((c) => c.name === "strokeRect")?.args).toEqual([1, 2, 10, 10]);
    expect(calls.find((c) => c.name === "strokeRect")?.fillStyle).toBe("#000000");
  });

  it("draws nothing for a zero-size pixelate region", () => {
    const { ctx, calls } = fakeContext();
    const shape: Shape = { kind: "pixelate", a: { x: 5, y: 5 }, b: { x: 5, y: 5 }, color: "#f00", width: 2 };
    expect(() => renderShapes(ctx, [shape], source)).not.toThrow();
    expect(calls.map((c) => c.name)).toEqual(["save", "restore"]);
  });

  it("draws a text shape line by line", () => {
    const { ctx, calls } = fakeContext();
    const shape: Shape = { kind: "text", at: { x: 0, y: 0 }, text: "a\nb", color: "#fff", size: 10 };
    renderShapes(ctx, [shape], source);
    const lines = calls.filter((c) => c.name === "fillText").map((c) => c.args[0]);
    expect(lines).toEqual(["a", "b"]);
  });

  it("draws each shape in order", () => {
    const { ctx, calls } = fakeContext();
    const shapes: Shape[] = [
      { kind: "redact", a: { x: 0, y: 0 }, b: { x: 1, y: 1 }, color: "#f00", width: 2 },
      { kind: "redact", a: { x: 2, y: 2 }, b: { x: 3, y: 3 }, color: "#f00", width: 2 },
    ];
    renderShapes(ctx, shapes, source);
    expect(calls.filter((c) => c.name === "fillRect").map((c) => c.args[0])).toEqual([0, 2]);
  });
});
