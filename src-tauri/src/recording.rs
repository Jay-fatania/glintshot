//! Screen recording through an FFmpeg child process.
//!
//! Linux X11 only for now: `x11grab` (uses MIT-SHM) with Intel/AMD VA-API
//! hardware encoding, falling back to software libx264 when VA-API fails.
//! The spike measured VA-API at 60 fps with no dropped frames on ~0.4 core.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::capture::Rect;

const FPS: u32 = 60;
const VAAPI_DEVICE: &str = "/dev/dri/renderD128";
/// How long FFmpeg must survive after launch before we trust the encoder.
const STARTUP_CHECK: Duration = Duration::from_millis(800);
/// How long to wait for FFmpeg to finalize the MP4 after asking it to stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    Vaapi,
    X264,
}

impl Encoder {
    fn args(self) -> &'static [&'static str] {
        match self {
            Encoder::Vaapi => &["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-qp", "23"],
            Encoder::X264 => &["-vf", "format=yuv420p", "-c:v", "libx264", "-preset", "veryfast", "-crf", "23"],
        }
    }
}

pub struct Recording {
    child: Child,
    part: PathBuf,
    output: PathBuf,
    log: PathBuf,
    started: Instant,
    pub encoder: Encoder,
    pub rect: Rect,
}

#[derive(Serialize)]
pub struct Finished {
    pub path: String,
    pub encoder: Encoder,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub bytes: u64,
}

/// Starts recording `rect` (desktop coordinates, physical pixels) into `dir`.
/// Tries VA-API first, then libx264; set `GLINTSHOT_ENCODER=x264` to skip
/// VA-API (useful for testing the fallback). Blocks for up to ~1.6 s while checking
/// that FFmpeg actually started, so call it off the main thread.
pub fn start(rect: Rect, dir: &Path, log_dir: &Path) -> Result<Recording, String> {
    if std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("wayland")) {
        return Err("Recording on Wayland is not supported yet; log in with \"Ubuntu on Xorg\".".into());
    }
    let display = std::env::var("DISPLAY").map_err(|_| "DISPLAY is not set; is this an X11 session?")?;

    // H.264 with 4:2:0 colour needs even dimensions.
    let rect = Rect { width: rect.width & !1, height: rect.height & !1, ..rect };
    if rect.width < 16 || rect.height < 16 {
        return Err("Recording area is too small (minimum 16 × 16 px).".into());
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let output = dir.join(format!("glintshot-{stamp}.mp4"));
    let part = dir.join(format!("glintshot-{stamp}.mp4.part"));
    fs::create_dir_all(log_dir).map_err(|e| e.to_string())?;
    let log = log_dir.join("ffmpeg-last.log");

    let force_x264 = std::env::var("GLINTSHOT_ENCODER").is_ok_and(|e| e == "x264");
    let mut encoders = Vec::new();
    if !force_x264 && Path::new(VAAPI_DEVICE).exists() {
        encoders.push(Encoder::Vaapi);
    }
    encoders.push(Encoder::X264);

    let mut last_error = String::from("no encoder available");
    for encoder in encoders {
        let mut child = spawn_ffmpeg(&display, rect, encoder, &part, &log)?;
        // FFmpeg records during the startup check, so the clock starts now.
        let started = Instant::now();
        thread::sleep(STARTUP_CHECK);
        match child.try_wait() {
            Ok(None) => {
                return Ok(Recording { child, part, output, log, started, encoder, rect });
            }
            Ok(Some(status)) => {
                last_error = format!("{encoder:?} encoder exited ({status}): {}", log_tail(&log));
                let _ = fs::remove_file(&part);
            }
            Err(e) => {
                let _ = child.kill();
                last_error = e.to_string();
            }
        }
    }
    Err(format!("Could not start recording. {last_error}"))
}

fn spawn_ffmpeg(display: &str, rect: Rect, encoder: Encoder, part: &Path, log: &Path) -> Result<Child, String> {
    let log_file = File::create(log).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostats", "-y"]);
    if encoder == Encoder::Vaapi {
        cmd.args(["-vaapi_device", VAAPI_DEVICE]);
    }
    cmd.args(["-f", "x11grab", "-draw_mouse", "1"])
        .args(["-framerate", &FPS.to_string()])
        .args(["-video_size", &format!("{}x{}", rect.width, rect.height)])
        .args(["-i", &format!("{display}+{},{}", rect.x, rect.y)])
        .args(encoder.args())
        .args(["-movflags", "+faststart", "-f", "mp4"])
        .arg(part)
        // Stdin is how we ask FFmpeg to stop ("q"); stderr goes to a file so a
        // full pipe can never stall the recording.
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file));
    cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "FFmpeg is not installed (sudo apt install ffmpeg).".to_string(),
        _ => format!("could not start FFmpeg: {e}"),
    })
}

impl Recording {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Asks FFmpeg to finish the file, waits for it, and moves the finished MP4
    /// into place with user-only permissions.
    pub fn stop(mut self) -> Result<Finished, String> {
        let seconds = self.started.elapsed().as_secs_f64();
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"q");
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break None;
                }
            }
        };

        let finalized = status.is_some_and(|s| s.success()) && self.part.exists();
        if !finalized {
            let _ = fs::remove_file(&self.part);
            return Err(format!("Recording could not be saved. {}", log_tail(&self.log)));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.part, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
        fs::rename(&self.part, &self.output).map_err(|e| e.to_string())?;
        let bytes = fs::metadata(&self.output).map(|m| m.len()).unwrap_or(0);
        Ok(Finished {
            path: self.output.display().to_string(),
            encoder: self.encoder,
            width: self.rect.width,
            height: self.rect.height,
            seconds,
            bytes,
        })
    }
}

/// Last few lines of the FFmpeg log, for error messages.
fn log_tail(log: &Path) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" | ");
    if tail.is_empty() { "No FFmpeg output.".into() } else { tail }
}
