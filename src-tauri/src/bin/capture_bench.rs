//! Spike benchmark: repeatedly grabs the whole screen and reports how fast
//! full-frame capture is. Usage: capture_bench [seconds] (default 10).
//!
//! Each grab is a full screenshot, so this measures the upper bound for
//! screenshot latency, not a streaming recording pipeline.

use std::time::{Duration, Instant};

use glintshot_lib::capture::Capturer;

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let capturer = Capturer::new().unwrap_or_else(|e| panic!("capture unavailable: {e}"));
    let rect = capturer.screen_rect();
    println!("screen: {}x{}", rect.width, rect.height);

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut samples_ms = Vec::new();
    while Instant::now() < deadline {
        let started = Instant::now();
        let frame = capturer.capture(rect).expect("capture failed");
        samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(frame);
    }

    samples_ms.sort_by(|a, b| a.total_cmp(b));
    let n = samples_ms.len();
    let pct = |p: f64| samples_ms[((n as f64 - 1.0) * p).round() as usize];
    let mean = samples_ms.iter().sum::<f64>() / n as f64;
    println!("frames: {n} in {seconds}s ({:.1} fps)", n as f64 / seconds as f64);
    println!(
        "capture ms: mean {mean:.2} | p50 {:.2} | p95 {:.2} | p99 {:.2} | max {:.2}",
        pct(0.50),
        pct(0.95),
        pct(0.99),
        pct(1.0)
    );
    println!(
        "frames under 16.7 ms (60 fps budget): {:.1}%",
        samples_ms.iter().filter(|&&ms| ms < 16.7).count() as f64 * 100.0 / n as f64
    );
}
