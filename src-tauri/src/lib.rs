use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::ipc::Response;
use image::{imageops, RgbaImage};
use tauri::{AppHandle, Manager, Monitor, State, WebviewUrl, WebviewWindowBuilder};

pub mod capture;

const OVERLAY_LABEL: &str = "overlay";

/// The frame captured for the current selection session.
#[derive(Default)]
struct CaptureState {
    frame: Mutex<Option<RgbaImage>>,
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
/// full-screen selection overlay showing that frozen frame.
#[tauri::command]
async fn start_capture(app: AppHandle, state: State<'_, CaptureState>) -> Result<CaptureInfo, String> {
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

fn close_overlay(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.destroy();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// ~/Pictures/Glintshot, created readable only by the current user.
fn capture_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .picture_dir()
        .map_err(|e| e.to_string())?
        .join("Glintshot");
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
            save_region,
            cancel_capture
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
