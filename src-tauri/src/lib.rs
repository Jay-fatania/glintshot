use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::{imageops, RgbaImage};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub mod capture;
pub mod recording;

const MAIN_LABEL: &str = "main";
const OVERLAY_LABEL: &str = "overlay";
const TRAY_ID: &str = "glintshot";

/// Global hotkeys. The record hotkey also stops a running recording.
const SCREENSHOT_HOTKEY: &str = "ctrl+alt+s";
const RECORD_HOTKEY: &str = "ctrl+alt+r";

#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CaptureMode {
    #[default]
    Screenshot,
    Record,
}

/// State for the current selection session and any running recording.
#[derive(Default)]
struct CaptureState {
    frame: Mutex<Option<RgbaImage>>,
    mode: Mutex<CaptureMode>,
    /// Desktop position of the captured monitor, in physical pixels.
    origin: Mutex<(i32, i32)>,
    recording: Mutex<Option<recording::Recording>>,
    hotkeys: Mutex<Vec<HotkeyStatus>>,
}

/// Tray menu entries whose enabled state follows the recording state.
struct TrayItems {
    screenshot: MenuItem<Wry>,
    record: MenuItem<Wry>,
    stop: MenuItem<Wry>,
}

#[derive(Clone, Serialize)]
struct HotkeyStatus {
    keys: String,
    action: String,
    /// None when registered; otherwise why it failed (e.g. taken by another app).
    error: Option<String>,
}

#[derive(Serialize)]
struct CaptureInfo {
    width: u32,
    height: u32,
    capture_ms: f64,
}

#[derive(Serialize)]
struct SaveResult {
    path: String,
    width: u32,
    height: u32,
    save_ms: f64,
}

#[derive(Serialize)]
struct RecordingStarted {
    encoder: recording::Encoder,
    width: u32,
    height: u32,
}

fn monitor_under_cursor(app: &AppHandle) -> Result<Monitor, String> {
    if let Ok(pos) = app.cursor_position()
        && let Ok(Some(m)) = app.monitor_from_point(pos.x, pos.y)
    {
        return Ok(m);
    }
    app.primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor found".to_string())
}

/// Hides the main window, grabs the monitor under the cursor and opens the
/// full-screen selection overlay showing that frozen frame. In record mode the
/// selection is then passed to `start_recording` instead of saved.
async fn begin_capture(app: &AppHandle, mode: CaptureMode) -> Result<CaptureInfo, String> {
    let state = app.state::<CaptureState>();
    if state.recording.lock().unwrap().is_some() {
        return Err("A recording is already running.".into());
    }
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Err("A capture is already in progress.".into());
    }
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        main.hide().map_err(|e| e.to_string())?;
    }
    // Give the compositor time to remove the main window from the screen.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let monitor = monitor_under_cursor(app)?;
    let rect = capture::Rect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    };
    let started = Instant::now();
    let frame = capture::capture_rect(rect)?;
    let capture_ms = started.elapsed().as_secs_f64() * 1000.0;

    let info = CaptureInfo {
        width: frame.width(),
        height: frame.height(),
        capture_ms,
    };
    *state.frame.lock().unwrap() = Some(frame);
    *state.mode.lock().unwrap() = mode;
    *state.origin.lock().unwrap() = (rect.x, rect.y);

    let scale = monitor.scale_factor();
    WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Glintshot capture")
        .position(rect.x as f64 / scale, rect.y as f64 / scale)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .fullscreen(true)
        .focused(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(info)
}

/// Starts a capture from the tray or a hotkey. There is no caller to return
/// errors to, so they are shown in the main window.
fn spawn_capture(app: &AppHandle, mode: CaptureMode) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = begin_capture(&app, mode).await {
            report_error(&app, e);
        }
    });
}

#[tauri::command]
async fn start_capture(app: AppHandle, mode: CaptureMode) -> Result<CaptureInfo, String> {
    begin_capture(&app, mode).await
}

/// Returns the frozen frame as raw RGBA, prefixed with width and height
/// (u32 little-endian each). Raw bytes avoid PNG encoding on the hot path.
#[tauri::command]
fn get_frame(state: State<'_, CaptureState>) -> Result<Response, String> {
    let guard = state.frame.lock().unwrap();
    let frame = guard.as_ref().ok_or("no frame captured")?;
    let mut bytes = Vec::with_capacity(8 + frame.as_raw().len());
    bytes.extend_from_slice(&frame.width().to_le_bytes());
    bytes.extend_from_slice(&frame.height().to_le_bytes());
    bytes.extend_from_slice(frame.as_raw());
    Ok(Response::new(bytes))
}

#[tauri::command]
fn capture_mode(state: State<'_, CaptureState>) -> CaptureMode {
    *state.mode.lock().unwrap()
}

/// Crops the frozen frame to the selection (in image pixels) and saves it as a
/// PNG in the private Glintshot pictures folder.
#[tauri::command]
fn save_region(
    app: AppHandle,
    state: State<'_, CaptureState>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<SaveResult, String> {
    let started = Instant::now();
    let frame = state.frame.lock().unwrap().take().ok_or("no frame captured")?;
    let x = x.min(frame.width().saturating_sub(1));
    let y = y.min(frame.height().saturating_sub(1));
    let width = width.clamp(1, frame.width() - x);
    let height = height.clamp(1, frame.height() - y);
    let region = imageops::crop_imm(&frame, x, y, width, height).to_image();

    let dir = private_dir(app.path().picture_dir().map_err(|e| e.to_string())?)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("glintshot-{stamp}.png"));
    write_private_png(&region, &path)?;

    close_overlay(&app);
    Ok(SaveResult {
        path: path.display().to_string(),
        width,
        height,
        save_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

#[tauri::command]
fn cancel_capture(app: AppHandle, state: State<'_, CaptureState>) {
    state.frame.lock().unwrap().take();
    close_overlay(&app);
}

/// Starts recording the selection (image pixels relative to the captured
/// monitor). The overlay is closed first so it never appears in the video;
/// because the caller is gone by then, the outcome is sent to the main window
/// as a `recording-started` or `capture-error` event. The main window stays
/// hidden while recording; it is stopped from the tray or with the hotkey.
#[tauri::command]
async fn start_recording(
    app: AppHandle,
    state: State<'_, CaptureState>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<RecordingStarted, String> {
    let frame = state.frame.lock().unwrap().take().ok_or("no frame captured")?;
    let (ox, oy) = *state.origin.lock().unwrap();
    let x = x.min(frame.width().saturating_sub(1));
    let y = y.min(frame.height().saturating_sub(1));
    let rect = capture::Rect {
        x: ox + x as i32,
        y: oy + y as i32,
        width: width.clamp(1, frame.width() - x),
        height: height.clamp(1, frame.height() - y),
    };
    drop(frame);

    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.destroy();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let dir = private_dir(app.path().video_dir().map_err(|e| e.to_string())?)?;
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let started = tauri::async_runtime::spawn_blocking(move || recording::start(rect, &dir, &log_dir))
        .await
        .map_err(|e| e.to_string())?;

    match started {
        Ok(rec) => {
            let info = RecordingStarted { encoder: rec.encoder, width: rec.rect.width, height: rec.rect.height };
            *state.recording.lock().unwrap() = Some(rec);
            set_recording_ui(&app, true);
            let _ = app.emit_to(MAIN_LABEL, "recording-started", &info);
            Ok(info)
        }
        Err(e) => {
            report_error(&app, e.clone());
            Err(e)
        }
    }
}

/// Stops the running recording, saves it, and shows the result in the main
/// window: always `recording-ended`, then `recording-stopped` or `capture-error`.
async fn finish_recording(app: &AppHandle) -> Result<recording::Finished, String> {
    let rec = app
        .state::<CaptureState>()
        .recording
        .lock()
        .unwrap()
        .take()
        .ok_or("No recording is running.")?;
    let result = tauri::async_runtime::spawn_blocking(move || rec.stop())
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);

    set_recording_ui(app, false);
    let _ = app.emit_to(MAIN_LABEL, "recording-ended", ());
    match &result {
        Ok(done) => {
            let _ = app.emit_to(MAIN_LABEL, "recording-stopped", done);
            show_main(app);
        }
        Err(e) => report_error(app, e.clone()),
    }
    result
}

fn spawn_finish_recording(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = finish_recording(&app).await;
    });
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> Result<recording::Finished, String> {
    finish_recording(&app).await
}

#[tauri::command]
fn hotkeys(state: State<'_, CaptureState>) -> Vec<HotkeyStatus> {
    state.hotkeys.lock().unwrap().clone()
}

fn report_error(app: &AppHandle, message: String) {
    let _ = app.emit_to(MAIN_LABEL, "capture-error", message);
    show_main(app);
}

fn close_overlay(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.destroy();
    }
    show_main(app);
}

fn show_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
}

/// Updates the tray to match whether a recording is running.
fn set_recording_ui(app: &AppHandle, active: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.screenshot.set_enabled(!active);
        let _ = items.record.set_enabled(!active);
        let _ = items.stop.set_enabled(active);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(if active { "Glintshot: recording" } else { "Glintshot" }));
        // Shown next to the icon on Linux (AppIndicator) and macOS.
        let _ = tray.set_title(if active { Some("● REC") } else { None });
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let screenshot = MenuItem::with_id(app, "screenshot", "Screenshot region", true, Some("Ctrl+Alt+S"))?;
    let record = MenuItem::with_id(app, "record", "Record region", true, Some("Ctrl+Alt+R"))?;
    let stop = MenuItem::with_id(app, "stop", "Stop recording", false, Some("Ctrl+Alt+R"))?;
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Glintshot", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&screenshot, &record, &stop, &separator, &show, &quit])?;
    app.manage(TrayItems { screenshot, record, stop });

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Glintshot")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "screenshot" => spawn_capture(app, CaptureMode::Screenshot),
            "record" => spawn_capture(app, CaptureMode::Record),
            "stop" => spawn_finish_recording(app),
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)
}

/// Registers the global hotkeys, remembering which ones failed (for example
/// because another app already owns the key combination).
fn register_hotkeys(app: &AppHandle) {
    let wanted = [
        (SCREENSHOT_HOTKEY, "Screenshot region"),
        (RECORD_HOTKEY, "Record region / stop recording"),
    ];
    let statuses = wanted
        .iter()
        .map(|(keys, action)| HotkeyStatus {
            keys: keys.to_string(),
            action: action.to_string(),
            error: app.global_shortcut().register(*keys).err().map(|e| e.to_string()),
        })
        .collect();
    *app.state::<CaptureState>().hotkeys.lock().unwrap() = statuses;
}

fn on_hotkey(app: &AppHandle, shortcut: &Shortcut) {
    let is_record = RECORD_HOTKEY.parse::<Shortcut>().is_ok_and(|s| s == *shortcut);
    let recording = app.state::<CaptureState>().recording.lock().unwrap().is_some();
    match (is_record, recording) {
        (true, true) => spawn_finish_recording(app),
        // Showing an error window mid-recording could put it in the video.
        (false, true) => {}
        // Pressing a hotkey while the overlay is open does nothing.
        _ if app.get_webview_window(OVERLAY_LABEL).is_some() => {}
        (true, false) => spawn_capture(app, CaptureMode::Record),
        (false, false) => spawn_capture(app, CaptureMode::Screenshot),
    }
}

/// `<base>/Glintshot`, created readable only by the current user.
fn private_dir(base: PathBuf) -> Result<PathBuf, String> {
    let dir = base.join("Glintshot");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Writes to a temporary file first, then renames, so a crash never leaves a
/// half-written screenshot behind.
fn write_private_png(image: &RgbaImage, path: &Path) -> Result<(), String> {
    let tmp = path.with_extension("png.tmp");
    image
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        on_hotkey(app, shortcut);
                    }
                })
                .build(),
        )
        .manage(CaptureState::default())
        .setup(|app| {
            build_tray(app.handle())?;
            register_hotkeys(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_capture,
            get_frame,
            capture_mode,
            save_region,
            cancel_capture,
            start_recording,
            stop_recording,
            hotkeys
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // Closing the main window hides it; Glintshot keeps running in the tray.
            RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. }
                if label == MAIN_LABEL =>
            {
                api.prevent_close();
                if let Some(main) = app.get_webview_window(MAIN_LABEL) {
                    let _ = main.hide();
                }
            }
            // Never leave FFmpeg running or a recording unfinished on quit.
            RunEvent::Exit => {
                let rec = app.state::<CaptureState>().recording.lock().unwrap().take();
                if let Some(rec) = rec {
                    let _ = rec.stop();
                }
            }
            _ => {}
        });
}
