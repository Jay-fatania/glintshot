import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

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

interface Settings {
  screenshot_dir: string;
  recording_dir: string;
  screenshot_hotkey: string;
  record_hotkey: string;
  start_at_login: boolean;
  enter_action: "save" | "copy";
  recording_fps: number;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const ENCODER_NAMES: Record<string, string> = {
  vaapi: "GPU (VA-API H.264)",
  x264: "CPU (libx264)",
};

let timer: number | undefined;
let savedSettings: Settings | null = null;

// ---------------------------------------------------------------- capture tab

function showError(message: string) {
  const el = $("error");
  el.textContent = message;
  el.hidden = false;
  showTab("capture");
}

function showResult(title: string, rows: [string, string][]) {
  $("r-title").textContent = title;
  $("r-list").replaceChildren(
    ...rows.flatMap(([label, value]) => {
      const dt = document.createElement("dt");
      dt.textContent = label;
      const dd = document.createElement("dd");
      dd.textContent = value;
      return [dt, dd];
    }),
  );
  $("result").hidden = false;
  showTab("capture");
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
    await invoke("start_capture", { mode });
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

/** "ctrl+alt+KeyS" → "Ctrl+Alt+S" */
function formatKeys(keys: string) {
  const names: Record<string, string> = { ctrl: "Ctrl", control: "Ctrl", alt: "Alt", shift: "Shift", super: "Super" };
  return keys
    .split("+")
    .map((k) => names[k.toLowerCase()] ?? k.replace(/^Key(?=[A-Z]$)/, "").replace(/^Digit(?=\d$)/, ""))
    .map((k) => (k.length === 1 ? k.toUpperCase() : k))
    .join("+");
}

function showHotkeys(hotkeys: HotkeyStatus[]) {
  $("hotkeys").replaceChildren(
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
  const record = hotkeys.find((h) => h.action.startsWith("Record"));
  $("stop-hint").textContent = record
    ? `You can also stop from the tray icon or with ${formatKeys(record.keys)}.`
    : "You can also stop from the tray icon.";
}

// ---------------------------------------------------------------- tabs

function showTab(name: "capture" | "settings") {
  for (const tab of ["capture", "settings"] as const) {
    const active = tab === name;
    $(`tab-${tab}`).classList.toggle("active", active);
    $(`tab-${tab}`).setAttribute("aria-selected", String(active));
    $(`panel-${tab}`).hidden = !active;
  }
}

// ---------------------------------------------------------------- settings tab

const hotkeyFields = ["s-screenshot-hotkey", "s-record-hotkey"] as const;
let recordingField: HTMLButtonElement | null = null;

function fillSettings(s: Settings) {
  $<HTMLInputElement>("s-screenshot-dir").value = s.screenshot_dir;
  $<HTMLInputElement>("s-recording-dir").value = s.recording_dir;
  setHotkeyField($<HTMLButtonElement>("s-screenshot-hotkey"), s.screenshot_hotkey);
  setHotkeyField($<HTMLButtonElement>("s-record-hotkey"), s.record_hotkey);
  $<HTMLSelectElement>("s-enter-action").value = s.enter_action;
  $<HTMLSelectElement>("s-fps").value = String(s.recording_fps);
  $<HTMLInputElement>("s-start-at-login").checked = s.start_at_login;
}

function readSettings(): Settings {
  return {
    screenshot_dir: $<HTMLInputElement>("s-screenshot-dir").value.trim(),
    recording_dir: $<HTMLInputElement>("s-recording-dir").value.trim(),
    screenshot_hotkey: $("s-screenshot-hotkey").dataset.keys ?? "",
    record_hotkey: $("s-record-hotkey").dataset.keys ?? "",
    start_at_login: $<HTMLInputElement>("s-start-at-login").checked,
    enter_action: $<HTMLSelectElement>("s-enter-action").value as Settings["enter_action"],
    recording_fps: Number($<HTMLSelectElement>("s-fps").value),
  };
}

function setHotkeyField(field: HTMLButtonElement, keys: string) {
  field.dataset.keys = keys;
  field.textContent = formatKeys(keys);
}

function setStatus(message: string, isError = false) {
  const el = $("s-status");
  el.textContent = message;
  el.classList.toggle("error", isError);
}

function updateDirtyState() {
  const dirty = savedSettings !== null && JSON.stringify(readSettings()) !== JSON.stringify(savedSettings);
  $<HTMLButtonElement>("s-save").disabled = !dirty;
  $<HTMLButtonElement>("s-reset").disabled = !dirty;
  if (dirty) setStatus("Unsaved changes.");
}

async function loadSettings() {
  savedSettings = await invoke<Settings>("get_settings");
  fillSettings(savedSettings);
  updateDirtyState();
  setStatus("");
}

async function saveSettings(e: SubmitEvent) {
  e.preventDefault();
  stopRecordingHotkey(false);
  const button = $<HTMLButtonElement>("s-save");
  button.disabled = true;
  try {
    const saved = await invoke<{ settings: Settings; hotkeys: HotkeyStatus[] }>("save_settings", {
      settings: readSettings(),
    });
    savedSettings = saved.settings;
    fillSettings(saved.settings);
    showHotkeys(saved.hotkeys);
    updateDirtyState();
    const failed = saved.hotkeys.filter((h) => h.error);
    setStatus(
      failed.length
        ? `Saved, but ${failed.map((h) => formatKeys(h.keys)).join(" and ")} could not be registered (another app may use it).`
        : "Settings saved.",
      failed.length > 0,
    );
  } catch (err) {
    setStatus(`${err}`, true);
    updateDirtyState();
  }
}

async function browse(inputId: string) {
  const input = $<HTMLInputElement>(inputId);
  const picked = await open({ directory: true, defaultPath: input.value || undefined, title: "Choose a folder" });
  if (typeof picked === "string") {
    input.value = picked;
    updateDirtyState();
  }
}

// Hotkey recording: global hotkeys are paused so the current combination
// doesn't fire while the user presses it.

function startRecordingHotkey(field: HTMLButtonElement) {
  stopRecordingHotkey(false);
  recordingField = field;
  field.classList.add("recording");
  field.textContent = "Press keys…";
  invoke("pause_hotkeys", { paused: true });
}

function stopRecordingHotkey(keep: boolean, keys?: string) {
  const field = recordingField;
  if (!field) return;
  recordingField = null;
  field.classList.remove("recording");
  setHotkeyField(field, keep && keys ? keys : (field.dataset.keys ?? ""));
  invoke("pause_hotkeys", { paused: false });
  updateDirtyState();
}

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "OS", "AltGraph", "Super", "Hyper"]);

function onHotkeyKeydown(e: KeyboardEvent) {
  if (!recordingField) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") {
    stopRecordingHotkey(false);
    return;
  }
  if (MODIFIER_KEYS.has(e.key)) return; // wait for the main key
  const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "super"].filter(Boolean);
  if (mods.length === 0) {
    recordingField.textContent = "Add Ctrl, Alt, Shift or Super…";
    return;
  }
  stopRecordingHotkey(true, [...mods, e.code].join("+"));
}

// ---------------------------------------------------------------- startup

window.addEventListener("DOMContentLoaded", () => {
  $("tab-capture").addEventListener("click", () => showTab("capture"));
  $("tab-settings").addEventListener("click", () => {
    showTab("settings");
    loadSettings().catch((e) => setStatus(`Could not load settings: ${e}`, true));
  });

  $("capture-btn").addEventListener("click", () => capture("screenshot"));
  $("record-btn").addEventListener("click", () => capture("record"));
  $("stop-btn").addEventListener("click", stopRecording);

  const form = $<HTMLFormElement>("panel-settings");
  form.addEventListener("submit", saveSettings);
  form.addEventListener("input", updateDirtyState);
  form.addEventListener("change", updateDirtyState);
  $("s-reset").addEventListener("click", () => {
    if (savedSettings) fillSettings(savedSettings);
    updateDirtyState();
    setStatus("");
  });
  for (const b of form.querySelectorAll<HTMLButtonElement>("[data-browse]")) {
    b.addEventListener("click", () => browse(b.dataset.browse!));
  }
  for (const id of hotkeyFields) {
    const field = $<HTMLButtonElement>(id);
    field.addEventListener("click", () => startRecordingHotkey(field));
    field.addEventListener("blur", () => stopRecordingHotkey(false));
  }
  window.addEventListener("keydown", onHotkeyKeydown, true);

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

  invoke<HotkeyStatus[]>("hotkeys")
    .then(showHotkeys)
    .catch((e) => showError(`Could not read hotkeys: ${e}`));
});
