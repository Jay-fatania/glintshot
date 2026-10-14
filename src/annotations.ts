// Annotation model and renderer. All coordinates and sizes are in image
// pixels (the captured frame's physical pixels), so the same shapes render
// identically in the live overlay and in the exported image.

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type DragTool = "arrow" | "rect" | "ellipse" | "highlight" | "redact" | "pixelate";
export type Tool = DragTool | "text" | "marker";

export type Shape =
  | { kind: DragTool; a: Point; b: Point; color: string; width: number }
  | { kind: "text"; at: Point; text: string; color: string; size: number }
  | { kind: "marker"; at: Point; n: number; color: string; size: number };

/** Pixelation block size in image pixels: large enough that text can't be read back. */
const PIXEL_BLOCK = 14;

export function rectFrom(a: Point, b: Point): Rect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

export class Annotations {
  shapes: Shape[] = [];
  private undone: Shape[] = [];

  add(shape: Shape) {
    this.shapes.push(shape);
    this.undone = [];
  }

  undo() {
    const shape = this.shapes.pop();
    if (shape) this.undone.push(shape);
  }

  redo() {
    const shape = this.undone.pop();
    if (shape) this.shapes.push(shape);
  }

  get canUndo() {
    return this.shapes.length > 0;
  }

  get canRedo() {
    return this.undone.length > 0;
  }

  nextMarkerNumber() {
    return this.shapes.filter((s) => s.kind === "marker").length + 1;
  }
}

/**
 * Draws shapes onto `ctx`, whose coordinate system must already map image
 * pixels (translate it for export). `source` is the frozen frame, used by
 * pixelate so it always works from the original screen pixels.
 */
export function renderShapes(ctx: CanvasRenderingContext2D, shapes: Iterable<Shape>, source: HTMLCanvasElement) {
  for (const shape of shapes) {
    ctx.save();
    drawShape(ctx, shape, source);
    ctx.restore();
  }
}

function drawShape(ctx: CanvasRenderingContext2D, shape: Shape, source: HTMLCanvasElement) {
  switch (shape.kind) {
    case "rect": {
      const r = rectFrom(shape.a, shape.b);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.lineJoin = "round";
      ctx.strokeRect(r.x, r.y, r.width, r.height);
      break;
    }
    case "ellipse": {
      const r = rectFrom(shape.a, shape.b);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.beginPath();
      ctx.ellipse(r.x + r.width / 2, r.y + r.height / 2, r.width / 2, r.height / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "arrow":
      drawArrow(ctx, shape.a, shape.b, shape.color, shape.width);
      break;
    case "highlight": {
      const r = rectFrom(shape.a, shape.b);
      ctx.globalAlpha = 0.35;
      ctx.fillStyle = shape.color;
      ctx.fillRect(r.x, r.y, r.width, r.height);
      break;
    }
    case "redact": {
      const r = rectFrom(shape.a, shape.b);
      ctx.fillStyle = "#000";
      ctx.fillRect(r.x, r.y, r.width, r.height);
      break;
    }
    case "pixelate":
      drawPixelated(ctx, rectFrom(shape.a, shape.b), source);
      break;
    case "text":
      drawText(ctx, shape.at, shape.text, shape.color, shape.size);
      break;
    case "marker":
      drawMarker(ctx, shape.at, shape.n, shape.color, shape.size);
      break;
  }
}

function drawArrow(ctx: CanvasRenderingContext2D, a: Point, b: Point, color: string, width: number) {
  const angle = Math.atan2(b.y - a.y, b.x - a.x);
  const length = Math.hypot(b.x - a.x, b.y - a.y);
  const head = Math.min(length * 0.5, Math.max(10, width * 4));
  // Stop the shaft inside the head so its square end doesn't poke out.
  const shaftEnd = {
    x: b.x - Math.cos(angle) * head * 0.6,
    y: b.y - Math.sin(angle) * head * 0.6,
  };
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.lineTo(shaftEnd.x, shaftEnd.y);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(b.x, b.y);
  ctx.lineTo(b.x - head * Math.cos(angle - Math.PI / 7), b.y - head * Math.sin(angle - Math.PI / 7));
  ctx.lineTo(b.x - head * Math.cos(angle + Math.PI / 7), b.y - head * Math.sin(angle + Math.PI / 7));
  ctx.closePath();
  ctx.fill();
}

function drawPixelated(ctx: CanvasRenderingContext2D, r: Rect, source: HTMLCanvasElement) {
  const x = Math.round(r.x);
  const y = Math.round(r.y);
  const width = Math.round(r.width);
  const height = Math.round(r.height);
  if (width < 1 || height < 1) return;
  const small = document.createElement("canvas");
  small.width = Math.max(1, Math.ceil(width / PIXEL_BLOCK));
  small.height = Math.max(1, Math.ceil(height / PIXEL_BLOCK));
  const smallCtx = small.getContext("2d")!;
  // Downscaling with smoothing averages each block; upscaling without it
  // gives hard-edged blocks.
  smallCtx.imageSmoothingEnabled = true;
  smallCtx.imageSmoothingQuality = "medium";
  smallCtx.drawImage(source, x, y, width, height, 0, 0, small.width, small.height);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(small, 0, 0, small.width, small.height, x, y, width, height);
}

function textFont(size: number) {
  return `600 ${size}px system-ui, -apple-system, "Segoe UI", sans-serif`;
}

/** True when white reads better than black on top of `color`. */
function isDark(color: string) {
  const hex = color.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return 0.299 * r + 0.587 * g + 0.114 * b < 150;
}

function drawText(ctx: CanvasRenderingContext2D, at: Point, text: string, color: string, size: number) {
  ctx.font = textFont(size);
  ctx.textBaseline = "top";
  ctx.lineJoin = "round";
  ctx.lineWidth = Math.max(2, size / 6);
  // A contrasting outline keeps text readable on any background.
  ctx.strokeStyle = isDark(color) ? "rgba(255,255,255,0.9)" : "rgba(0,0,0,0.75)";
  ctx.fillStyle = color;
  text.split("\n").forEach((line, i) => {
    const y = at.y + i * size * 1.25;
    ctx.strokeText(line, at.x, y);
    ctx.fillText(line, at.x, y);
  });
}

function drawMarker(ctx: CanvasRenderingContext2D, at: Point, n: number, color: string, radius: number) {
  ctx.beginPath();
  ctx.arc(at.x, at.y, radius, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
  ctx.lineWidth = Math.max(1.5, radius / 7);
  ctx.strokeStyle = "rgba(255,255,255,0.9)";
  ctx.stroke();
  ctx.fillStyle = isDark(color) ? "#fff" : "#111";
  ctx.font = textFont(Math.round(radius * 1.1));
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(String(n), at.x, at.y + radius * 0.06);
}
