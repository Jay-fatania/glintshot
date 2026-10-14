//! Platform screen-capture backends.
//!
//! Linux X11: direct `GetImage` on the root window via x11rb (no PipeWire;
//! Ubuntu 22.04's PipeWire 0.3.48 is too old for the Rust PipeWire bindings).
//! Linux Wayland: not implemented in the spike; needs the ScreenCast portal.
//! Windows: xcap (Windows.Graphics.Capture / GDI).

/// A rectangle in physical (device) pixels, in desktop coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A visible top-level window, for click-to-capture.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub title: String,
    /// Outer bounds in desktop coordinates: title bar included, client-side
    /// shadows excluded.
    pub rect: Rect,
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
    use super::WindowInfo;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, ImageFormat, ImageOrder, MapState, Window};
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

        fn atom(&self, name: &str) -> Result<Atom, String> {
            Ok(self
                .conn
                .intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .atom)
        }

        /// Raw property bytes, or empty if the window doesn't have it.
        fn property(&self, window: Window, property: Atom, type_: impl Into<Atom>) -> Vec<u8> {
            self.conn
                .get_property(false, window, property, type_.into(), 0, 4096)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| reply.value)
                .unwrap_or_default()
        }

        fn cardinals(&self, window: Window, property: Atom) -> Vec<u32> {
            self.conn
                .get_property(false, window, property, AtomEnum::ANY, 0, 4096)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().map(|v| v.collect()))
                .unwrap_or_default()
        }

        /// Visible top-level windows on the current workspace, topmost first.
        pub fn windows(&self) -> Result<Vec<WindowInfo>, String> {
            let stacking = self.atom("_NET_CLIENT_LIST_STACKING")?;
            let current_desktop = self.atom("_NET_CURRENT_DESKTOP")?;
            let wm_desktop = self.atom("_NET_WM_DESKTOP")?;
            let wm_state = self.atom("_NET_WM_STATE")?;
            let state_hidden = self.atom("_NET_WM_STATE_HIDDEN")?;
            let wm_type = self.atom("_NET_WM_WINDOW_TYPE")?;
            let type_desktop = self.atom("_NET_WM_WINDOW_TYPE_DESKTOP")?;
            let type_dock = self.atom("_NET_WM_WINDOW_TYPE_DOCK")?;
            // Window-manager decorations (title bar) around the client window.
            let frame_extents = self.atom("_NET_FRAME_EXTENTS")?;
            // Invisible shadow margin that GTK client-side-decorated windows include.
            let gtk_extents = self.atom("_GTK_FRAME_EXTENTS")?;
            let wm_name = self.atom("_NET_WM_NAME")?;
            let utf8 = self.atom("UTF8_STRING")?;

            let current = self.cardinals(self.root, current_desktop).first().copied();
            let mut windows = Vec::new();
            for window in self.cardinals(self.root, stacking).into_iter().rev() {
                let Some(attrs) = self.conn.get_window_attributes(window).ok().and_then(|c| c.reply().ok()) else {
                    continue;
                };
                if attrs.map_state != MapState::VIEWABLE {
                    continue;
                }
                // 0xFFFFFFFF means "on all workspaces".
                if let (Some(current), Some(&desktop)) = (current, self.cardinals(window, wm_desktop).first())
                    && desktop != current
                    && desktop != u32::MAX
                {
                    continue;
                }
                if self.cardinals(window, wm_state).contains(&state_hidden) {
                    continue;
                }
                if self.cardinals(window, wm_type).iter().any(|t| *t == type_desktop || *t == type_dock) {
                    continue;
                }
                let Some(geometry) = self.conn.get_geometry(window).ok().and_then(|c| c.reply().ok()) else {
                    continue;
                };
                let Some(origin) = self
                    .conn
                    .translate_coordinates(window, self.root, 0, 0)
                    .ok()
                    .and_then(|c| c.reply().ok())
                else {
                    continue;
                };

                let (mut x, mut y) = (origin.dst_x as i32, origin.dst_y as i32);
                let (mut w, mut h) = (geometry.width as i32, geometry.height as i32);
                if let [l, r, t, b] = self.cardinals(window, gtk_extents)[..] {
                    let [l, r, t, b] = [l, r, t, b].map(|v| v as i32);
                    (x, y, w, h) = (x + l, y + t, w - l - r, h - t - b);
                }
                if let [l, r, t, b] = self.cardinals(window, frame_extents)[..] {
                    let [l, r, t, b] = [l, r, t, b].map(|v| v as i32);
                    (x, y, w, h) = (x - l, y - t, w + l + r, h + t + b);
                }
                if w <= 0 || h <= 0 {
                    continue;
                }

                let mut title = self.property(window, wm_name, utf8);
                if title.is_empty() {
                    title = self.property(window, AtomEnum::WM_NAME.into(), AtomEnum::STRING);
                }
                windows.push(WindowInfo {
                    title: String::from_utf8_lossy(&title).into_owned(),
                    rect: Rect { x, y, width: w as u32, height: h as u32 },
                });
            }
            Ok(windows)
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

        /// Window picking is not implemented on Windows yet.
        pub fn windows(&self) -> Result<Vec<super::WindowInfo>, String> {
            Ok(Vec::new())
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
