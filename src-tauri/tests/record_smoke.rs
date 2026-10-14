//! Needs a live X11 display and FFmpeg, so it is ignored by default.
//! Run with: cargo test --test record_smoke -- --ignored --nocapture

use std::process::Command;
use std::thread;
use std::time::Duration;

use glintshot_lib::capture::Rect;
use glintshot_lib::recording;

#[test]
#[ignore]
fn records_a_region_to_mp4() {
    let dir = std::env::temp_dir().join(format!("glintshot-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Odd size on purpose: the recorder must round down to even dimensions.
    let rect = Rect { x: 100, y: 100, width: 641, height: 361 };
    let rec = recording::start(rect, &dir, &dir).expect("recording should start");
    println!("encoder: {:?}", rec.encoder);
    thread::sleep(Duration::from_secs(3));
    let done = rec.stop().expect("recording should finish");
    println!("saved {} ({} bytes, {:.1} s)", done.path, done.bytes, done.seconds);

    assert_eq!((done.width, done.height), (640, 360));
    assert!(done.bytes > 0);
    assert!(!std::path::Path::new(&format!("{}.part", done.path)).exists());

    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-count_frames"])
        .args(["-show_entries", "stream=codec_name,width,height,nb_read_frames", "-of", "csv=p=0"])
        .arg(&done.path)
        .output()
        .expect("ffprobe should run");
    let info = String::from_utf8_lossy(&probe.stdout);
    println!("ffprobe: {}", info.trim());
    let fields: Vec<&str> = info.trim().split(',').collect();
    assert_eq!(&fields[..3], &["h264", "640", "360"]);
    let frames: u32 = fields[3].parse().unwrap();
    let fps = frames as f64 / done.seconds;
    println!("effective fps: {fps:.1}");
    // Reported length must match the video: ~60 fps, allowing for timing slack.
    assert!((50.0..=65.0).contains(&fps), "{frames} frames over {:.2} s", done.seconds);

    std::fs::remove_dir_all(&dir).unwrap();
}
