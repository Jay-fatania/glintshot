//! Dev tool: prints the windows Glintshot can pick by clicking, topmost
//! first, with their outer bounds. Read-only; nothing on screen changes.

use glintshot_lib::capture::Capturer;

fn main() {
    let capturer = Capturer::new().unwrap_or_else(|e| panic!("capture unavailable: {e}"));
    let windows = capturer.windows().unwrap_or_else(|e| panic!("could not list windows: {e}"));
    for w in windows {
        let r = w.rect;
        println!("{:>5},{:<5} {:>4}x{:<4}  {}", r.x, r.y, r.width, r.height, w.title);
    }
}
