use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::{imageops, RgbaImage};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, Monitor, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

pub mod capture;
pub mod recording;

const OVERLAY_LABEL: &str = "overlay";

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

fn monitor_under_cursor(app: &AppHandle) -> Result<Monitor, String> {
    if let Ok(pos) = app.cursor_position() {
        if let Ok(Some(m)) = app.monitor_from_point(pos.x, pos.y) {
            return Ok(m);
        }
    }
    app.primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor found".to_string())
}

/// Hides the main window, grabs the monitor under the cursor and opens the
/// full-screen selection overlay showing that frozen frame. In record mode the
/// selection is then passed to `start_recording` instead of saved.
#[tauri::command]
async fn start_capture(
    app: AppHandle,
    state: State<'_, CaptureState>,
    mode: CaptureMode,
) -> Result<CaptureInfo, String> {
    if state.recording.lock().unwrap().is_some() {
        return Err("A recording is already running.".into());
    }
    if let Some(main) = app.get_webview_window("main") {
        main.hide().map_err(|e| e.to_string())?;
    }
    // Give the compositor time to remove the main window from the screen.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let monitor = monitor_under_cursor(&app)?;
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
    let x = rect.x as f64 / scale;
    let y = rect.y as f64 / scale;

    if let Some(old) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = old.destroy();
    }
    WebviewWindowBuilder::new(&app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Glintshot capture")
        .position(x, y)
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

    let dir = capture_dir(&app)?;
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

#[derive(Serialize)]
struct RecordingStarted {
    encoder: recording::Encoder,
    width: u32,
    height: u32,
}

/// Starts recording the selection (image pixels relative to the captured
/// monitor). The overlay is closed first so it never appears in the video;
/// because the caller is gone by then, the outcome is also sent to the main
/// window as a `recording-started` or `capture-error` event.
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

    let result = started.map(|rec| {
        let info = RecordingStarted { encoder: rec.encoder, width: rec.rect.width, height: rec.rect.height };
        *state.recording.lock().unwrap() = Some(rec);
        info
    });
    let _ = match &result {
        Ok(info) => app.emit_to("main", "recording-started", info),
        Err(e) => app.emit_to("main", "capture-error", e),
    };
    show_main(&app);
    result
}

#[tauri::command]
async fn stop_recording(state: State<'_, CaptureState>) -> Result<recording::Finished, String> {
    let rec = state.recording.lock().unwrap().take().ok_or("No recording is running.")?;
    tauri::async_runtime::spawn_blocking(move || rec.stop())
        .await
        .map_err(|e| e.to_string())?
}

fn close_overlay(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.destroy();
    }
    show_main(app);
}

fn show_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// ~/Pictures/Glintshot, created readable only by the current user.
fn capture_dir(app: &AppHandle) -> Result<PathBuf, String> {
    private_dir(app.path().picture_dir().map_err(|e| e.to_string())?)
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
        .manage(CaptureState::default())
        .invoke_handler(tauri::generate_handler![
            start_capture,
            get_frame,
            capture_mode,
            save_region,
            cancel_capture,
            start_recording,
            stop_recording
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Never leave FFmpeg running or a recording unfinished on quit.
            if let RunEvent::Exit = event {
                let rec = app.state::<CaptureState>().recording.lock().unwrap().take();
                if let Some(rec) = rec {
                    let _ = rec.stop();
                }
            }
        });
}
