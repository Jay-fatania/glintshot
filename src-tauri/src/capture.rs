//! Platform screen-capture backends.
//!
//! Linux X11: direct `GetImage` on the root window via x11rb (no PipeWire;
//! Ubuntu 22.04's PipeWire 0.3.48 is too old for the Rust PipeWire bindings).
//! Linux Wayland: not implemented in the spike; needs the ScreenCast portal.
//! Windows: xcap (Windows.Graphics.Capture / GDI).

use image::RgbaImage;

/// A rectangle in physical (device) pixels, in desktop coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "linux")]
pub use linux::Capturer;

#[cfg(windows)]
pub use windows::Capturer;

#[cfg(target_os = "linux")]
mod linux {
    use super::Rect;
    use image::RgbaImage;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder, Window};
    use x11rb::rust_connection::RustConnection;

    pub struct Capturer {
        conn: RustConnection,
        root: Window,
        bits_per_pixel: u8,
        lsb_first: bool,
    }

    impl Capturer {
        pub fn new() -> Result<Self, String> {
            if std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("wayland")) {
                return Err("Wayland sessions are not supported in this spike build yet; log in with \"Ubuntu on Xorg\".".into());
            }
            let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("cannot connect to X server: {e}"))?;
            let setup = conn.setup();
            let screen = &setup.roots[screen_num];
            let depth = screen.root_depth;
            if depth != 24 && depth != 32 {
                return Err(format!("unsupported X11 root depth {depth}"));
            }
            let bits_per_pixel = setup
                .pixmap_formats
                .iter()
                .find(|f| f.depth == depth)
                .map(|f| f.bits_per_pixel)
                .ok_or("no pixmap format for root depth")?;
            if bits_per_pixel != 32 {
                return Err(format!("unsupported X11 pixel size {bits_per_pixel} bpp"));
            }
            let root = screen.root;
            let lsb_first = setup.image_byte_order == ImageOrder::LSB_FIRST;
            Ok(Self { conn, root, bits_per_pixel, lsb_first })
        }

        /// Size of the whole X screen (all monitors).
        pub fn screen_rect(&self) -> Rect {
            let screen = &self.conn.setup().roots[0];
            Rect { x: 0, y: 0, width: screen.width_in_pixels as u32, height: screen.height_in_pixels as u32 }
        }

        pub fn capture(&self, rect: Rect) -> Result<RgbaImage, String> {
            let reply = self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    self.root,
                    rect.x as i16,
                    rect.y as i16,
                    rect.width as u16,
                    rect.height as u16,
                    !0,
                )
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| format!("X11 GetImage failed: {e}"))?;

            debug_assert_eq!(self.bits_per_pixel, 32);
            let expected = rect.width as usize * rect.height as usize * 4;
            if reply.data.len() < expected {
                return Err("X11 returned a short image".into());
            }
            // 32 bpp ZPixmap is B,G,R,X in memory on little-endian servers.
            let mut rgba = Vec::with_capacity(expected);
            let (pixels, _) = reply.data[..expected].as_chunks::<4>();
            for px in pixels {
                if self.lsb_first {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    rgba.extend_from_slice(&[px[1], px[2], px[3], 255]);
                }
            }
            RgbaImage::from_raw(rect.width, rect.height, rgba).ok_or_else(|| "bad image buffer".to_string())
        }
    }
}

#[cfg(windows)]
mod windows {
    // Not built or tested yet: the spike runs on Ubuntu first.
    use super::Rect;
    use image::{imageops, RgbaImage};
    use xcap::Monitor;

    pub struct Capturer;

    impl Capturer {
        pub fn new() -> Result<Self, String> {
            Ok(Self)
        }

        pub fn screen_rect(&self) -> Rect {
            let m = Monitor::all().ok().and_then(|ms| ms.into_iter().next());
            Rect {
                x: 0,
                y: 0,
                width: m.as_ref().and_then(|m| m.width().ok()).unwrap_or(0),
                height: m.as_ref().and_then(|m| m.height().ok()).unwrap_or(0),
            }
        }

        pub fn capture(&self, rect: Rect) -> Result<RgbaImage, String> {
            let monitor = Monitor::from_point(rect.x, rect.y).map_err(|e| e.to_string())?;
            let frame = monitor.capture_image().map_err(|e| e.to_string())?;
            let ox = (rect.x - monitor.x().unwrap_or(0)).max(0) as u32;
            let oy = (rect.y - monitor.y().unwrap_or(0)).max(0) as u32;
            // xcap's image crate may differ in version; convert via raw bytes.
            let (w, h) = (frame.width(), frame.height());
            let frame = RgbaImage::from_raw(w, h, frame.into_raw()).ok_or("bad image buffer")?;
            Ok(imageops::crop_imm(&frame, ox, oy, rect.width.min(w - ox), rect.height.min(h - oy)).to_image())
        }
    }
}

/// Convenience for one-off captures.
pub fn capture_rect(rect: Rect) -> Result<RgbaImage, String> {
    Capturer::new()?.capture(rect)
}
