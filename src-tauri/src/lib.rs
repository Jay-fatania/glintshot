use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub mod capture;
pub mod recording;
pub mod settings;

use settings::Settings;

const MAIN_LABEL: &str = "main";
const OVERLAY_LABEL: &str = "overlay";
const TRAY_ID: &str = "glintshot";

/// Passed by the login item so Glintshot starts hidden in the tray.
const BACKGROUND_ARG: &str = "--background";

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
    /// Windows on the captured monitor (image pixels, topmost first).
    windows: Mutex<Vec<PickableWindow>>,
    /// How long the last screen grab took, for the result panel.
    capture_ms: Mutex<f64>,
    /// Kept alive for the whole run: on Linux the app itself serves the
    /// clipboard contents, so they vanish if this is dropped.
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

struct SettingsState {
    path: PathBuf,
    settings: Mutex<Settings>,
}

fn current_settings(app: &AppHandle) -> Settings {
    app.state::<SettingsState>().settings.lock().unwrap().clone()
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

/// A window the overlay can select by clicking, in image pixels relative to
/// the captured monitor and clipped to it.
#[derive(Clone, Serialize)]
struct PickableWindow {
    title: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct CaptureInfo {
    width: u32,
    height: u32,
    capture_ms: f64,
}

#[derive(Clone, Serialize)]
struct ScreenshotResult {
    /// None when the image was only copied.
    path: Option<String>,
    copied: bool,
    width: u32,
    height: u32,
    capture_ms: f64,
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
    let capturer = capture::Capturer::new()?;
    let started = Instant::now();
    let frame = capturer.capture(rect)?;
    let capture_ms = started.elapsed().as_secs_f64() * 1000.0;
    // Listed right after the grab so they match the frozen frame. Failure
    // only disables click-to-select, so it isn't fatal.
    let windows = capturer.windows().unwrap_or_default();
    *state.windows.lock().unwrap() = pickable_windows(&windows, rect);

    let info = CaptureInfo {
        width: frame.width(),
        height: frame.height(),
        capture_ms,
    };
    *state.frame.lock().unwrap() = Some(frame);
    *state.mode.lock().unwrap() = mode;
    *state.origin.lock().unwrap() = (rect.x, rect.y);
    *state.capture_ms.lock().unwrap() = capture_ms;

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

/// Converts desktop windows to monitor-relative image pixels, clipped to the
/// monitor, dropping any that are tiny or off this monitor.
fn pickable_windows(windows: &[capture::WindowInfo], monitor: capture::Rect) -> Vec<PickableWindow> {
    let (mx, my) = (monitor.x as i64, monitor.y as i64);
    let (mw, mh) = (monitor.width as i64, monitor.height as i64);
    windows
        .iter()
        .filter_map(|w| {
            let left = (w.rect.x as i64 - mx).max(0);
            let top = (w.rect.y as i64 - my).max(0);
            let right = (w.rect.x as i64 + w.rect.width as i64 - mx).min(mw);
            let bottom = (w.rect.y as i64 + w.rect.height as i64 - my).min(mh);
            (right - left >= 16 && bottom - top >= 16).then(|| PickableWindow {
                title: w.title.clone(),
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            })
        })
        .collect()
}

#[tauri::command]
fn get_windows(state: State<'_, CaptureState>) -> Vec<PickableWindow> {
    state.windows.lock().unwrap().clone()
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

#[derive(Serialize)]
struct OverlayConfig {
    mode: CaptureMode,
    enter_action: settings::EnterAction,
}

#[tauri::command]
fn overlay_config(app: AppHandle, state: State<'_, CaptureState>) -> OverlayConfig {
    OverlayConfig {
        mode: *state.mode.lock().unwrap(),
        enter_action: current_settings(&app).enter_action,
    }
}

/// Receives the finished screenshot (selection with annotations drawn, as raw
/// RGBA) from the overlay. Headers: `x-width`, `x-height`, and `x-action`
/// (`save`: write a PNG and copy it; `copy`: clipboard only). The overlay is
/// closed here, so the result goes to the main window as `capture-saved`.
#[tauri::command]
fn finish_screenshot(
    app: AppHandle,
    state: State<'_, CaptureState>,
    request: Request<'_>,
) -> Result<ScreenshotResult, String> {
    let started = Instant::now();
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| format!("missing {name} header"))
    };
    let width: u32 = header("x-width")?.parse().map_err(|_| "bad x-width")?;
    let height: u32 = header("x-height")?.parse().map_err(|_| "bad x-height")?;
    let save = match header("x-action")?.as_str() {
        "save" => true,
        "copy" => false,
        other => return Err(format!("unknown action {other}")),
    };
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw image bytes".into());
    };
    let image = RgbaImage::from_raw(width, height, bytes.clone()).ok_or("image size does not match its data")?;

    let path = if save {
        let dir = current_settings(&app).screenshot_dir;
        settings::ensure_dir(&dir)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("glintshot-{stamp}.png"));
        write_private_png(&image, &path)?;
        Some(path.display().to_string())
    } else {
        None
    };
    copy_to_clipboard(&state, &image)?;

    state.frame.lock().unwrap().take();
    let result = ScreenshotResult {
        path,
        copied: true,
        width,
        height,
        capture_ms: *state.capture_ms.lock().unwrap(),
        save_ms: started.elapsed().as_secs_f64() * 1000.0,
    };
    let _ = app.emit_to(MAIN_LABEL, "capture-saved", &result);
    close_overlay(&app);
    Ok(result)
}

fn copy_to_clipboard(state: &CaptureState, image: &RgbaImage) -> Result<(), String> {
    let mut guard = state.clipboard.lock().unwrap();
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?);
    }
    guard
        .as_mut()
        .unwrap()
        .set_image(arboard::ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: std::borrow::Cow::Borrowed(image.as_raw()),
        })
        .map_err(|e| format!("could not copy to clipboard: {e}"))
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

    let config = current_settings(&app);
    let dir = config.recording_dir;
    settings::ensure_dir(&dir)?;
    let fps = config.recording_fps;
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let started = tauri::async_runtime::spawn_blocking(move || recording::start(rect, &dir, &log_dir, fps))
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
    // No accelerator labels: hotkeys are configurable and listed in the window.
    let screenshot = MenuItem::with_id(app, "screenshot", "Screenshot", true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", "Record", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop recording", false, None::<&str>)?;
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

/// (Re-)registers the hotkeys from settings, remembering which ones failed
/// (for example because another app already owns the key combination).
fn register_hotkeys(app: &AppHandle) {
    let config = current_settings(app);
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let statuses = [
        (config.screenshot_hotkey, "Screenshot"),
        (config.record_hotkey, "Record / stop recording"),
    ]
    .into_iter()
    .map(|(keys, action)| {
        let error = settings::parse_hotkey(&keys)
            .and_then(|shortcut| shortcuts.register(shortcut).map_err(|e| e.to_string()))
            .err();
        HotkeyStatus { keys, action: action.to_string(), error }
    })
    .collect();
    *app.state::<CaptureState>().hotkeys.lock().unwrap() = statuses;
}

/// Lets the settings screen record a new combination without the current
/// hotkeys firing (X11 grabs would otherwise swallow the key press).
#[tauri::command]
fn pause_hotkeys(app: AppHandle, paused: bool) {
    if paused {
        let _ = app.global_shortcut().unregister_all();
    } else {
        register_hotkeys(&app);
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    let mut config = current_settings(&app);
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        config.start_at_login = enabled;
    }
    config
}

#[derive(Serialize)]
struct SettingsSaved {
    settings: Settings,
    hotkeys: Vec<HotkeyStatus>,
}

/// Validates and applies new settings: output folders, login item, hotkeys.
/// Nothing is stored unless every step succeeds.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<SettingsSaved, String> {
    settings.validate()?;
    settings::ensure_dir(&settings.screenshot_dir)?;
    settings::ensure_dir(&settings.recording_dir)?;

    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(!settings.start_at_login) != settings.start_at_login {
        let result = if settings.start_at_login { autolaunch.enable() } else { autolaunch.disable() };
        result.map_err(|e| format!("Could not change start at login: {e}"))?;
    }

    let state = app.state::<SettingsState>();
    settings::save(&state.path, &settings)?;
    *state.settings.lock().unwrap() = settings.clone();
    register_hotkeys(&app);
    Ok(SettingsSaved {
        settings,
        hotkeys: app.state::<CaptureState>().hotkeys.lock().unwrap().clone(),
    })
}

fn on_hotkey(app: &AppHandle, shortcut: &Shortcut) {
    let is_record = settings::parse_hotkey(&current_settings(app).record_hotkey).is_ok_and(|s| s == *shortcut);
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

/// Loads settings (defaults: ~/Pictures/Glintshot and ~/Videos/Glintshot).
fn init_settings(app: &AppHandle) -> tauri::Result<()> {
    let paths = app.path();
    let home = paths.home_dir()?;
    let pictures = paths.picture_dir().unwrap_or_else(|_| home.join("Pictures"));
    let videos = paths.video_dir().unwrap_or_else(|_| home.join("Videos"));
    let path = paths.app_config_dir()?.join("settings.json");
    let mut loaded = settings::load(&path, Settings::defaults(&pictures, &videos));
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        loaded.start_at_login = enabled;
    }
    app.manage(SettingsState { path, settings: Mutex::new(loaded) });
    Ok(())
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![BACKGROUND_ARG])))
        .manage(CaptureState::default())
        .setup(|app| {
            init_settings(app.handle())?;
            build_tray(app.handle())?;
            register_hotkeys(app.handle());
            // The window starts hidden (see tauri.conf.json) so a login start
            // stays in the tray without flashing it.
            if !std::env::args().any(|a| a == BACKGROUND_ARG) {
                show_main(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_capture,
            get_frame,
            overlay_config,
            get_windows,
            finish_screenshot,
            cancel_capture,
            start_recording,
            stop_recording,
            hotkeys,
            pause_hotkeys,
            get_settings,
            save_settings
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
