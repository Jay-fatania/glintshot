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

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

let lastCapture: CaptureInfo | null = null;

function showError(message: string) {
  const el = $("error");
  el.textContent = message;
  el.hidden = false;
}

async function capture() {
  $("error").hidden = true;
  try {
    lastCapture = await invoke<CaptureInfo>("start_capture");
  } catch (e) {
    showError(`Capture failed: ${e}`);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  $("capture-btn").addEventListener("click", capture);

  // The overlay window reports the saved file back here.
  listen<SaveResult>("capture-saved", ({ payload }) => {
    $("r-path").textContent = payload.path;
    $("r-size").textContent = `${payload.width} × ${payload.height} px`;
    $("r-capture").textContent = lastCapture
      ? `${lastCapture.capture_ms.toFixed(1)} ms (${lastCapture.width} × ${lastCapture.height})`
      : "–";
    $("r-save").textContent = `${payload.save_ms.toFixed(1)} ms`;
    $("result").hidden = false;
  });
  listen<string>("capture-error", ({ payload }) => showError(payload));
});
