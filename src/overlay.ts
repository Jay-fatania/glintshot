import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const frameCanvas = document.getElementById("frame") as HTMLCanvasElement;
const uiCanvas = document.getElementById("ui") as HTMLCanvasElement;
const sizeLabel = document.getElementById("size-label") as HTMLDivElement;
const ui = uiCanvas.getContext("2d")!;

let imageWidth = 0;
let imageHeight = 0;
let dragStart: { x: number; y: number } | null = null;
let selection: Rect | null = null;
let busy = false;

/** Converts a mouse position (CSS pixels) to image pixels. */
function toImage(e: MouseEvent) {
  return {
    x: Math.round((e.clientX * imageWidth) / window.innerWidth),
    y: Math.round((e.clientY * imageHeight) / window.innerHeight),
  };
}

function rectBetween(a: { x: number; y: number }, b: { x: number; y: number }): Rect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

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
  ui.strokeStyle = "#a78bfa";
  ui.lineWidth = Math.max(1, Math.round(imageWidth / window.innerWidth));
  ui.strokeRect(x + 0.5, y + 0.5, width, height);

  const cssX = (x * window.innerWidth) / imageWidth;
  const cssY = (y * window.innerHeight) / imageHeight;
  sizeLabel.textContent = `${width} × ${height}`;
  sizeLabel.style.left = `${cssX}px`;
  sizeLabel.style.top = `${Math.max(0, cssY - 26)}px`;
  sizeLabel.hidden = false;
}

async function loadFrame() {
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

async function save() {
  if (!selection || busy) return;
  busy = true;
  try {
    const result = await invoke("save_region", { ...selection });
    await emit("capture-saved", result);
  } catch (e) {
    await emit("capture-error", `Save failed: ${e}`);
    await invoke("cancel_capture");
  }
}

async function cancel() {
  if (busy) return;
  busy = true;
  await invoke("cancel_capture");
}

window.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || busy) return;
  dragStart = toImage(e);
  selection = null;
  draw();
});

window.addEventListener("mousemove", (e) => {
  if (!dragStart) return;
  selection = rectBetween(dragStart, toImage(e));
  draw();
});

window.addEventListener("mouseup", (e) => {
  if (!dragStart) return;
  selection = rectBetween(dragStart, toImage(e));
  dragStart = null;
  draw();
  // Ignore accidental clicks; anything at least 4 px each way is saved.
  if (selection.width >= 4 && selection.height >= 4) {
    save();
  } else {
    selection = null;
    draw();
  }
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") cancel();
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

loadFrame().catch(async (e) => {
  await emit("capture-error", `Could not load the captured frame: ${e}`);
  await invoke("cancel_capture");
});
