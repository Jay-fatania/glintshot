import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface CaptureInfo {
  width: number;
  height: number;
  capture_ms: number;
}

interface ScreenshotResult {
  path: string | null;
  copied: boolean;
  width: number;
  height: number;
  capture_ms: number;
  save_ms: number;
}

interface RecordingStarted {
  encoder: string;
  width: number;
  height: number;
}

interface HotkeyStatus {
  keys: string;
  action: string;
  error: string | null;
}

interface RecordingFinished {
  path: string;
  encoder: string;
  width: number;
  height: number;
  seconds: number;
  bytes: number;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const ENCODER_NAMES: Record<string, string> = {
  vaapi: "GPU (VA-API H.264)",
  x264: "CPU (libx264)",
};

let timer: number | undefined;

function showError(message: string) {
  const el = $("error");
  el.textContent = message;
  el.hidden = false;
}

function showResult(title: string, rows: [string, string][]) {
  $("r-title").textContent = title;
  const list = $("r-list");
  list.replaceChildren(
    ...rows.flatMap(([label, value]) => {
      const dt = document.createElement("dt");
      dt.textContent = label;
      const dd = document.createElement("dd");
      dd.textContent = value;
      return [dt, dd];
    }),
  );
  $("result").hidden = false;
}

function setRecording(active: boolean) {
  $("idle-controls").hidden = active;
  $("hint").hidden = active;
  $("recording").hidden = !active;
  window.clearInterval(timer);
  if (active) {
    const started = Date.now();
    $("rec-time").textContent = "0:00";
    timer = window.setInterval(() => {
      const s = Math.floor((Date.now() - started) / 1000);
      $("rec-time").textContent = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
    }, 500);
  }
}

async function capture(mode: "screenshot" | "record") {
  $("error").hidden = true;
  try {
    await invoke<CaptureInfo>("start_capture", { mode });
  } catch (e) {
    showError(`Capture failed: ${e}`);
  }
}

function showRecordingResult(r: RecordingFinished) {
  showResult("Last recording", [
    ["Saved to", r.path],
    ["Area", `${r.width} × ${r.height} px`],
    ["Length", `${r.seconds.toFixed(1)} s`],
    ["Size", `${(r.bytes / 1048576).toFixed(1)} MB`],
    ["Encoder", ENCODER_NAMES[r.encoder] ?? r.encoder],
  ]);
}

// The result arrives as a `recording-stopped` event, the same way as when
// stopping from the tray or hotkey; errors arrive as `capture-error`.
async function stopRecording() {
  const button = $<HTMLButtonElement>("stop-btn");
  button.disabled = true;
  button.textContent = "Saving…";
  try {
    await invoke("stop_recording");
  } catch {
    // Already reported through the capture-error event.
  } finally {
    button.disabled = false;
    button.textContent = "Stop and save";
  }
}

function formatKeys(keys: string) {
  return keys
    .split("+")
    .map((k) => k.charAt(0).toUpperCase() + k.slice(1))
    .join("+");
}

async function loadHotkeys() {
  const list = $("hotkeys");
  const hotkeys = await invoke<HotkeyStatus[]>("hotkeys");
  list.replaceChildren(
    ...hotkeys.map((h) => {
      const li = document.createElement("li");
      const kbd = document.createElement("kbd");
      kbd.textContent = formatKeys(h.keys);
      li.append(kbd, ` ${h.action}`);
      if (h.error) {
        li.classList.add("error");
        li.append(` (unavailable: ${h.error})`);
      }
      return li;
    }),
  );
}

window.addEventListener("DOMContentLoaded", () => {
  $("capture-btn").addEventListener("click", () => capture("screenshot"));
  $("record-btn").addEventListener("click", () => capture("record"));
  $("stop-btn").addEventListener("click", stopRecording);

  // The overlay window reports results back here.
  listen<ScreenshotResult>("capture-saved", ({ payload }) => {
    const rows: [string, string][] = [];
    if (payload.path) rows.push(["Saved to", payload.path]);
    if (payload.copied) rows.push(["Clipboard", "Copied"]);
    rows.push(
      ["Region", `${payload.width} × ${payload.height} px`],
      ["Screen grab", `${payload.capture_ms.toFixed(1)} ms`],
      [payload.path ? "Save + copy" : "Copy", `${payload.save_ms.toFixed(1)} ms`],
    );
    showResult("Last screenshot", rows);
  });
  listen<RecordingStarted>("recording-started", ({ payload }) => {
    $("error").hidden = true;
    $("rec-detail").textContent =
      `${payload.width} × ${payload.height} px · ${ENCODER_NAMES[payload.encoder] ?? payload.encoder}`;
    setRecording(true);
  });
  listen("recording-ended", () => setRecording(false));
  listen<RecordingFinished>("recording-stopped", ({ payload }) => showRecordingResult(payload));
  listen<string>("capture-error", ({ payload }) => showError(payload));
  loadHotkeys().catch((e) => showError(`Could not read hotkeys: ${e}`));
});
