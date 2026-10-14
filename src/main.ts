import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface CaptureInfo {
  width: number;
  height: number;
  capture_ms: number;
}

interface SaveResult {
  path: string;
  width: number;
  height: number;
  save_ms: number;
}

interface RecordingStarted {
  encoder: string;
  width: number;
  height: number;
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

let lastCapture: CaptureInfo | null = null;
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
    lastCapture = await invoke<CaptureInfo>("start_capture", { mode });
  } catch (e) {
    showError(`Capture failed: ${e}`);
  }
}

async function stopRecording() {
  const button = $<HTMLButtonElement>("stop-btn");
  button.disabled = true;
  button.textContent = "Saving…";
  try {
    const r = await invoke<RecordingFinished>("stop_recording");
    showResult("Last recording", [
      ["Saved to", r.path],
      ["Area", `${r.width} × ${r.height} px`],
      ["Length", `${r.seconds.toFixed(1)} s`],
      ["Size", `${(r.bytes / 1048576).toFixed(1)} MB`],
      ["Encoder", ENCODER_NAMES[r.encoder] ?? r.encoder],
    ]);
  } catch (e) {
    showError(`${e}`);
  } finally {
    button.disabled = false;
    button.textContent = "Stop and save";
    setRecording(false);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  $("capture-btn").addEventListener("click", () => capture("screenshot"));
  $("record-btn").addEventListener("click", () => capture("record"));
  $("stop-btn").addEventListener("click", stopRecording);

  // The overlay window reports results back here.
  listen<SaveResult>("capture-saved", ({ payload }) => {
    showResult("Last screenshot", [
      ["Saved to", payload.path],
      ["Region", `${payload.width} × ${payload.height} px`],
      [
        "Screen grab",
        lastCapture
          ? `${lastCapture.capture_ms.toFixed(1)} ms (${lastCapture.width} × ${lastCapture.height})`
          : "–",
      ],
      ["Crop + save", `${payload.save_ms.toFixed(1)} ms`],
    ]);
  });
  listen<RecordingStarted>("recording-started", ({ payload }) => {
    $("error").hidden = true;
    $("rec-detail").textContent =
      `${payload.width} × ${payload.height} px · ${ENCODER_NAMES[payload.encoder] ?? payload.encoder}`;
    setRecording(true);
  });
  listen<string>("capture-error", ({ payload }) => showError(payload));
});
