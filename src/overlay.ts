import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import {
  Annotations,
  type DragTool,
  type Point,
  type Rect,
  type Shape,
  type Tool,
  rectFrom,
  renderShapes,
} from "./annotations";

const frameCanvas = document.getElementById("frame") as HTMLCanvasElement;
const uiCanvas = document.getElementById("ui") as HTMLCanvasElement;
const sizeLabel = document.getElementById("size-label") as HTMLDivElement;
const ui = uiCanvas.getContext("2d")!;

const COLORS = ["#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7", "#111827", "#ffffff"];
/** Stroke widths, text sizes and marker radii per size level, in CSS pixels. */
const SIZES = {
  stroke: [2, 4, 7],
  text: [16, 22, 32],
  marker: [11, 14, 18],
};

const TOOL_INFO: Record<Tool, { label: string; key: string; icon: string }> = {
  arrow: { label: "Arrow", key: "a", icon: '<path d="M5 19 19 5M11 5h8v8"/>' },
  rect: { label: "Rectangle", key: "r", icon: '<rect x="4" y="6" width="16" height="12" rx="1.5"/>' },
  ellipse: { label: "Ellipse", key: "e", icon: '<ellipse cx="12" cy="12" rx="8.5" ry="6.5"/>' },
  text: { label: "Text", key: "t", icon: '<path d="M5 6h14M12 6v13M9 19h6"/>' },
  highlight: {
    label: "Highlight",
    key: "h",
    icon: '<path d="m7 15 8.5-8.5 3 3L10 18H7Z"/><path d="M4 21h16" stroke-width="2.6"/>',
  },
  marker: {
    label: "Numbered marker",
    key: "n",
    icon: '<circle cx="12" cy="12" r="8"/><path d="M10.5 9.5 12 8.5v7" stroke-linecap="round"/>',
  },
  redact: {
    label: "Redact (solid black)",
    key: "x",
    icon: '<rect x="4" y="7" width="16" height="10" rx="1" fill="currentColor"/>',
  },
  pixelate: {
    label: "Pixelate",
    key: "p",
    icon: '<path d="M4 4h5v5H4zM15 4h5v5h-5zM9.5 9.5h5v5h-5zM4 15h5v5H4zM15 15h5v5h-5z" fill="currentColor" stroke="none"/>',
  },
};

let imageWidth = 0;
let imageHeight = 0;
let mode: "screenshot" | "record" = "screenshot";
let phase: "select" | "edit" = "select";
let busy = false;

let dragStart: Point | null = null;
let selection: Rect | null = null;

const annotations = new Annotations();
/** The drag shape being drawn, not yet committed. */
let draft: Extract<Shape, { kind: DragTool }> | null = null;
let tool: Tool = "arrow";
let color = COLORS[0];
let sizeLevel = 1;
let textInput: HTMLTextAreaElement | null = null;
let toolbar: HTMLDivElement | null = null;

/** Image pixels per CSS pixel. */
const scale = () => imageWidth / window.innerWidth;

/** Converts a mouse position (CSS pixels) to image pixels. */
function toImage(e: MouseEvent): Point {
  return {
    x: Math.round((e.clientX * imageWidth) / window.innerWidth),
    y: Math.round((e.clientY * imageHeight) / window.innerHeight),
  };
}

function toCss(p: Point): Point {
  return { x: (p.x * window.innerWidth) / imageWidth, y: (p.y * window.innerHeight) / imageHeight };
}

function inside(p: Point, r: Rect) {
  return p.x >= r.x && p.y >= r.y && p.x <= r.x + r.width && p.y <= r.y + r.height;
}

function clampToSelection(p: Point): Point {
  const r = selection!;
  return {
    x: Math.min(Math.max(p.x, r.x), r.x + r.width),
    y: Math.min(Math.max(p.y, r.y), r.y + r.height),
  };
}

// ---------------------------------------------------------------- drawing

function draw() {
  ui.clearRect(0, 0, imageWidth, imageHeight);
  ui.fillStyle = "rgba(0, 0, 0, 0.45)";
  ui.fillRect(0, 0, imageWidth, imageHeight);
  if (!selection) {
    sizeLabel.hidden = true;
    return;
  }
  const { x, y, width, height } = selection;
  ui.clearRect(x, y, width, height);

  if (phase === "edit") {
    ui.save();
    ui.beginPath();
    ui.rect(x, y, width, height);
    ui.clip();
    renderShapes(ui, draft ? [...annotations.shapes, draft] : annotations.shapes, frameCanvas);
    ui.restore();
  }

  ui.strokeStyle = "#a78bfa";
  ui.lineWidth = Math.max(1, Math.round(scale()));
  ui.strokeRect(x + 0.5, y + 0.5, width, height);

  const css = toCss(selection);
  sizeLabel.textContent = `${width} × ${height}`;
  sizeLabel.style.left = `${css.x}px`;
  sizeLabel.style.top = `${Math.max(0, css.y - 26)}px`;
  sizeLabel.hidden = false;
}

async function loadFrame() {
  mode = await invoke<"screenshot" | "record">("capture_mode");
  if (mode === "record") document.title = "Glintshot: select recording area";
  const buffer = await invoke<ArrayBuffer>("get_frame");
  const header = new DataView(buffer, 0, 8);
  imageWidth = header.getUint32(0, true);
  imageHeight = header.getUint32(4, true);

  for (const canvas of [frameCanvas, uiCanvas]) {
    canvas.width = imageWidth;
    canvas.height = imageHeight;
  }
  const pixels = new Uint8ClampedArray(buffer, 8, imageWidth * imageHeight * 4);
  frameCanvas.getContext("2d")!.putImageData(new ImageData(pixels, imageWidth, imageHeight), 0, 0);
  draw();
}

// ---------------------------------------------------------------- toolbar

function svgIcon(body: string) {
  return `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round">${body}</svg>`;
}

function button(label: string, html: string, onClick: () => void, className = "") {
  const b = document.createElement("button");
  b.type = "button";
  b.title = label;
  b.setAttribute("aria-label", label);
  b.className = className;
  b.innerHTML = html;
  // Keep focus (and keyboard shortcuts) on the overlay, not the button.
  b.addEventListener("mousedown", (e) => e.preventDefault());
  b.addEventListener("click", onClick);
  return b;
}

function buildToolbar() {
  toolbar = document.createElement("div");
  toolbar.id = "toolbar";
  toolbar.addEventListener("mousedown", (e) => e.stopPropagation());

  const tools = document.createElement("div");
  tools.className = "group";
  for (const t of Object.keys(TOOL_INFO) as Tool[]) {
    const info = TOOL_INFO[t];
    const b = button(`${info.label} (${info.key.toUpperCase()})`, svgIcon(info.icon), () => setTool(t), "tool");
    b.dataset.tool = t;
    tools.append(b);
  }

  const colors = document.createElement("div");
  colors.className = "group";
  for (const c of COLORS) {
    const b = button(`Color ${c}`, "", () => setColor(c), "swatch");
    b.style.setProperty("--swatch", c);
    b.dataset.color = c;
    colors.append(b);
  }

  const sizes = document.createElement("div");
  sizes.className = "group";
  ["S", "M", "L"].forEach((label, i) => {
    const b = button(`Size ${label} (${i + 1})`, label, () => setSize(i), "size");
    b.dataset.size = String(i);
    sizes.append(b);
  });

  const history = document.createElement("div");
  history.className = "group";
  history.append(
    button("Undo (Ctrl+Z)", svgIcon('<path d="M9 7H5V3M5.4 7A8 8 0 1 1 4.5 15"/>'), undo, "history undo"),
    button("Redo (Ctrl+Shift+Z)", svgIcon('<path d="M15 7h4V3M18.6 7A8 8 0 1 0 19.5 15"/>'), redo, "history redo"),
  );

  const actions = document.createElement("div");
  actions.className = "group actions";
  actions.append(
    button("Cancel (Esc)", "Cancel", cancel, "action"),
    button("Copy to clipboard (Ctrl+C)", "Copy", () => finish("copy"), "action"),
    button("Save and copy (Ctrl+S or Enter)", "Save", () => finish("save"), "action primary"),
  );

  toolbar.append(tools, colors, sizes, history, actions);
  document.body.append(toolbar);
  syncToolbar();
  positionToolbar();
}

/** Places the toolbar below the selection, else above it, else inside it. */
function positionToolbar() {
  if (!toolbar || !selection) return;
  const topLeft = toCss(selection);
  const bottomRight = toCss({ x: selection.x + selection.width, y: selection.y + selection.height });
  const bar = toolbar.getBoundingClientRect();
  const margin = 8;
  let top = bottomRight.y + margin;
  if (top + bar.height > window.innerHeight - margin) top = topLeft.y - bar.height - margin;
  if (top < margin) top = bottomRight.y - bar.height - margin;
  const left = Math.min(Math.max(bottomRight.x - bar.width, margin), window.innerWidth - bar.width - margin);
  toolbar.style.left = `${left}px`;
  toolbar.style.top = `${Math.max(margin, top)}px`;
}

function syncToolbar() {
  if (!toolbar) return;
  for (const b of toolbar.querySelectorAll<HTMLElement>("[data-tool]")) b.classList.toggle("active", b.dataset.tool === tool);
  for (const b of toolbar.querySelectorAll<HTMLElement>("[data-color]")) b.classList.toggle("active", b.dataset.color === color);
  for (const b of toolbar.querySelectorAll<HTMLElement>("[data-size]")) b.classList.toggle("active", b.dataset.size === String(sizeLevel));
  toolbar.querySelector<HTMLButtonElement>(".undo")!.disabled = !annotations.canUndo;
  toolbar.querySelector<HTMLButtonElement>(".redo")!.disabled = !annotations.canRedo;
  uiCanvas.style.cursor = tool === "text" ? "text" : "crosshair";
}

function setTool(t: Tool) {
  commitText();
  tool = t;
  syncToolbar();
}

function setColor(c: string) {
  color = c;
  if (textInput) textInput.style.color = c;
  syncToolbar();
}

function setSize(level: number) {
  sizeLevel = level;
  if (textInput) textInput.style.fontSize = `${SIZES.text[level]}px`;
  syncToolbar();
}

function undo() {
  commitText();
  annotations.undo();
  syncToolbar();
  draw();
}

function redo() {
  annotations.redo();
  syncToolbar();
  draw();
}

// ---------------------------------------------------------------- text tool

function startText(at: Point) {
  commitText();
  const css = toCss(at);
  const input = document.createElement("textarea");
  input.className = "text-input";
  input.rows = 1;
  input.spellcheck = false;
  input.style.left = `${css.x}px`;
  input.style.top = `${css.y}px`;
  input.style.color = color;
  input.style.fontSize = `${SIZES.text[sizeLevel]}px`;
  input.dataset.x = String(at.x);
  input.dataset.y = String(at.y);
  input.addEventListener("mousedown", (e) => e.stopPropagation());
  input.addEventListener("input", () => {
    input.style.height = "auto";
    input.style.height = `${input.scrollHeight}px`;
  });
  input.addEventListener("keydown", (e) => {
    // Typing must not trigger tool shortcuts or cancel the capture.
    e.stopPropagation();
    if (e.key === "Escape") {
      input.remove();
      textInput = null;
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      commitText();
    }
  });
  document.body.append(input);
  textInput = input;
  input.focus();
}

function commitText() {
  const input = textInput;
  if (!input) return;
  textInput = null;
  const text = input.value.replace(/\s+$/, "");
  if (text) {
    annotations.add({
      kind: "text",
      at: { x: Number(input.dataset.x), y: Number(input.dataset.y) },
      text,
      color,
      size: Math.round(SIZES.text[sizeLevel] * scale()),
    });
  }
  input.remove();
  syncToolbar();
  draw();
}

// ---------------------------------------------------------------- finishing

/** Renders the selection with its annotations into raw RGBA. */
function exportImage() {
  const r = selection!;
  const canvas = document.createElement("canvas");
  canvas.width = r.width;
  canvas.height = r.height;
  const ctx = canvas.getContext("2d")!;
  ctx.drawImage(frameCanvas, r.x, r.y, r.width, r.height, 0, 0, r.width, r.height);
  ctx.translate(-r.x, -r.y);
  renderShapes(ctx, annotations.shapes, frameCanvas);
  const data = ctx.getImageData(0, 0, r.width, r.height).data;
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

async function finish(action: "save" | "copy") {
  if (!selection || busy) return;
  commitText();
  busy = true;
  try {
    // Rust closes this overlay and reports the result to the main window.
    await invoke("finish_screenshot", exportImage(), {
      headers: { "x-width": String(selection.width), "x-height": String(selection.height), "x-action": action },
    });
  } catch (e) {
    busy = false;
    await emit("capture-error", `${action === "save" ? "Save" : "Copy"} failed: ${e}`);
  }
}

async function startRecording() {
  if (!selection || busy) return;
  busy = true;
  try {
    // Rust closes this overlay and reports the outcome to the main window.
    await invoke("start_recording", { ...selection });
  } catch (e) {
    await emit("capture-error", `${e}`);
    await invoke("cancel_capture");
  }
}

async function cancel() {
  if (busy) return;
  busy = true;
  await invoke("cancel_capture");
}

// ---------------------------------------------------------------- input

window.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || busy) return;
  const p = toImage(e);

  if (phase === "select") {
    dragStart = p;
    selection = null;
    draw();
    return;
  }

  commitText();
  if (!selection || !inside(p, selection)) return;
  if (tool === "text") {
    startText(p);
  } else if (tool === "marker") {
    annotations.add({
      kind: "marker",
      at: p,
      n: annotations.nextMarkerNumber(),
      color,
      size: Math.round(SIZES.marker[sizeLevel] * scale()),
    });
    syncToolbar();
    draw();
  } else {
    const width = Math.max(1, Math.round(SIZES.stroke[sizeLevel] * scale()));
    draft = { kind: tool, a: p, b: p, color, width };
  }
});

window.addEventListener("mousemove", (e) => {
  if (phase === "select" && dragStart) {
    selection = rectFrom(dragStart, toImage(e));
    draw();
  } else if (draft) {
    draft.b = clampToSelection(toImage(e));
    draw();
  }
});

window.addEventListener("mouseup", (e) => {
  if (phase === "select" && dragStart) {
    selection = rectFrom(dragStart, toImage(e));
    dragStart = null;
    // Ignore accidental clicks; anything at least 4 px each way counts.
    if (selection.width < 4 || selection.height < 4) {
      selection = null;
      draw();
    } else if (mode === "record") {
      draw();
      startRecording();
    } else {
      phase = "edit";
      buildToolbar();
      draw();
    }
    return;
  }
  if (draft) {
    const r = rectFrom(draft.a, draft.b);
    if (Math.max(r.width, r.height) >= 3) annotations.add(draft);
    draft = null;
    syncToolbar();
    draw();
  }
});

window.addEventListener("keydown", (e) => {
  if (busy) return;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();

  if (e.key === "Escape") {
    if (draft) {
      draft = null;
      draw();
    } else {
      cancel();
    }
    return;
  }
  if (phase !== "edit") return;

  if (ctrl && key === "z" && !e.shiftKey) undo();
  else if (ctrl && (key === "y" || (key === "z" && e.shiftKey))) redo();
  else if (ctrl && key === "c") finish("copy");
  else if ((ctrl && key === "s") || e.key === "Enter") finish("save");
  else if (!ctrl && !e.altKey && ["1", "2", "3"].includes(e.key)) setSize(Number(e.key) - 1);
  else {
    const match = ctrl || e.altKey ? undefined : (Object.keys(TOOL_INFO) as Tool[]).find((t) => TOOL_INFO[t].key === key);
    if (!match) return;
    setTool(match);
  }
  e.preventDefault();
});

window.addEventListener("resize", positionToolbar);
window.addEventListener("contextmenu", (e) => e.preventDefault());

loadFrame().catch(async (e) => {
  await emit("capture-error", `Could not load the captured frame: ${e}`);
  await invoke("cancel_capture");
});
