//! Grab the whole desktop as one frozen frame.
//!
//! - X11: a single `GetImage` of the root window. The server hands back BGRX
//!   pixels, which is already cairo's `ARGB32` memory layout on little-endian
//!   machines, so the frame goes to the editor without any conversion or copy.
//! - Wayland: XWayland cannot see native windows, so ask the compositor through
//!   the `xdg-desktop-portal` Screenshot interface (non-interactive), falling back
//!   to `grim` / `gnome-screenshot` / `spectacle`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, ImageOrder, MapState};

/// A captured frame: premultiplied BGRA (cairo `ARGB32`), tightly packed.
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn stride(&self) -> i32 {
        self.width * 4
    }

    /// Copy out the `(x, y, w, h)` sub-rectangle (clamped to the frame).
    pub fn crop(self, x: i32, y: i32, w: i32, h: i32) -> Frame {
        let x = x.clamp(0, self.width - 1);
        let y = y.clamp(0, self.height - 1);
        let w = w.clamp(1, self.width - x);
        let h = h.clamp(1, self.height - y);
        if x == 0 && y == 0 && w == self.width && h == self.height {
            return self;
        }
        let src_stride = self.stride() as usize;
        let row = w as usize * 4;
        let mut data = Vec::with_capacity(row * h as usize);
        for r in 0..h as usize {
            let start = (y as usize + r) * src_stride + x as usize * 4;
            data.extend_from_slice(&self.data[start..start + row]);
        }
        Frame {
            width: w,
            height: h,
            data,
        }
    }

    /// Build a frame from straight RGBA (consumes the buffer, converts in place).
    pub fn from_rgba(img: image::RgbaImage) -> Frame {
        let (width, height) = (img.width() as i32, img.height() as i32);
        let mut data = img.into_raw();
        for px in data.chunks_exact_mut(4) {
            let a = u16::from(px[3]);
            let (r, g, b) = (px[0], px[1], px[2]);
            if a == 255 {
                px[0] = b;
                px[2] = r;
            } else {
                px[0] = ((u16::from(b) * a + 127) / 255) as u8;
                px[1] = ((u16::from(g) * a + 127) / 255) as u8;
                px[2] = ((u16::from(r) * a + 127) / 255) as u8;
            }
        }
        Frame {
            width,
            height,
            data,
        }
    }
}

/// Convert premultiplied BGRA rows (with `stride`) into a straight RGBA image.
pub fn bgra_to_rgba(data: &[u8], width: i32, height: i32, stride: i32) -> image::RgbaImage {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let row = &data[y * stride as usize..y * stride as usize + width as usize * 4];
        for px in row.chunks_exact(4) {
            let a = px[3];
            let un = |c: u8| -> u8 {
                if a == 255 || a == 0 {
                    c
                } else {
                    ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
                }
            };
            out.extend_from_slice(&[un(px[2]), un(px[1]), un(px[0]), a]);
        }
    }
    image::RgbaImage::from_raw(width as u32, height as u32, out).unwrap_or_default()
}

/// Grab the whole desktop.
pub fn grab_desktop() -> Result<Frame, String> {
    if crate::recorder::portal::is_wayland_session() {
        portal()
            .or_else(|e| {
                log::warn!("portal screenshot failed ({e}); trying CLI tools");
                cli()
            })
            .map_err(|e| format!("Could not capture the screen: {e}"))
    } else {
        x11().or_else(|e| {
            log::warn!("X11 grab failed ({e}); trying CLI tools");
            cli()
        })
    }
}

fn x11() -> Result<Frame, String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let screen = &conn.setup().roots[screen_num];
    let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, w, h, !0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == reply.depth)
        .map(|f| f.bits_per_pixel)
        .unwrap_or(32);
    if bpp != 32 || reply.data.len() < w as usize * h as usize * 4 {
        return Err(format!(
            "unsupported root format (depth {}, {bpp} bpp)",
            reply.depth
        ));
    }
    let msb = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
    let mut data = reply.data;
    for px in data.chunks_exact_mut(4) {
        if msb {
            px.reverse();
        }
        px[3] = 255;
    }
    Ok(Frame {
        width: i32::from(w),
        height: i32::from(h),
        data,
    })
}

fn portal() -> Result<Frame, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let uri = rt.block_on(async {
        use ashpd::desktop::screenshot::Screenshot;
        let response = Screenshot::request()
            .interactive(false)
            .modal(false)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(response.uri().as_str().to_string())
    })?;
    let path = PathBuf::from(percent_decode(
        uri.strip_prefix("file://")
            .ok_or("portal returned a non-file URI")?,
    ));
    // The portal saves into ~/Pictures; hide the file from the screenshot
    // watcher straight away (dot-files are ignored), then delete it.
    let hidden = path.with_file_name(format!(".rustcast-grab-{}.png", std::process::id()));
    let src = if std::fs::rename(&path, &hidden).is_ok() {
        hidden
    } else {
        path
    };
    let frame = load(&src);
    let _ = std::fs::remove_file(&src);
    frame
}

fn cli() -> Result<Frame, String> {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let out = dir.join(format!("rustcast-grab-{}.png", std::process::id()));
    let o = out.to_string_lossy().to_string();
    let tools: [(&str, Vec<&str>); 5] = [
        ("grim", vec![&o]),
        ("gnome-screenshot", vec!["-f", &o]),
        ("spectacle", vec!["-b", "-n", "-f", "-o", &o]),
        ("scrot", vec!["-o", &o]),
        ("import", vec!["-window", "root", &o]),
    ];
    for (tool, args) in tools {
        let mut cmd = Command::new(tool);
        cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::null());
        // These tools must talk to the real Wayland compositor when there is one.
        if let Some(wd) = std::env::var_os("RUSTCAST_WAYLAND_DISPLAY") {
            cmd.env("WAYLAND_DISPLAY", wd);
            cmd.env_remove("GDK_BACKEND");
        }
        if cmd.status().is_ok_and(|s| s.success()) && out.exists() {
            let frame = load(&out);
            let _ = std::fs::remove_file(&out);
            return frame;
        }
    }
    Err("no screenshot tool worked (install grim or gnome-screenshot)".to_string())
}

fn load(path: &Path) -> Result<Frame, String> {
    let img = image::open(path).map_err(|e| e.to_string())?;
    Ok(Frame::from_rgba(img.into_rgba8()))
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Visible top-level windows on the current workspace, top-most first, as
/// `(x, y, w, h)` in root (device) pixels, including their decorations.
/// Only meaningful on X11 — on Wayland XWayland only sees its own clients.
pub fn visible_windows() -> Vec<(i32, i32, i32, i32)> {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return Vec::new();
    };
    let root = conn.setup().roots[screen_num].root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
    };
    let cardinals = |win, prop: Option<u32>, n: u32| -> Vec<u32> {
        prop.and_then(|p| {
            conn.get_property(false, win, p, AtomEnum::ANY, 0, n)
                .ok()?
                .reply()
                .ok()?
                .value32()
                .map(|v| v.collect())
        })
        .unwrap_or_default()
    };

    let stacking = cardinals(root, atom("_NET_CLIENT_LIST_STACKING"), u32::MAX);
    let current = cardinals(root, atom("_NET_CURRENT_DESKTOP"), 1)
        .first()
        .copied();
    let (wm_desktop, frame_ext, gtk_ext, wm_state, hidden) = (
        atom("_NET_WM_DESKTOP"),
        atom("_NET_FRAME_EXTENTS"),
        atom("_GTK_FRAME_EXTENTS"),
        atom("_NET_WM_STATE"),
        atom("_NET_WM_STATE_HIDDEN"),
    );

    let mut out = Vec::new();
    for &win in stacking.iter().rev() {
        let viewable = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|a| a.map_state == MapState::VIEWABLE);
        if !viewable {
            continue;
        }
        if let (Some(cur), Some(&desk)) = (current, cardinals(win, wm_desktop, 1).first())
            && desk != cur
            && desk != u32::MAX
        {
            continue;
        }
        if let Some(h) = hidden
            && cardinals(win, wm_state, 64).contains(&h)
        {
            continue;
        }
        let Some(geo) = conn.get_geometry(win).ok().and_then(|c| c.reply().ok()) else {
            continue;
        };
        let Some(pos) = conn
            .translate_coordinates(win, root, 0, 0)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            continue;
        };
        let (mut x, mut y) = (i32::from(pos.dst_x), i32::from(pos.dst_y));
        let (mut w, mut h) = (i32::from(geo.width), i32::from(geo.height));
        // Server-side decorations sit outside the client window…
        if let [l, r, t, b] = cardinals(win, frame_ext, 4)[..] {
            x -= l as i32;
            y -= t as i32;
            w += (l + r) as i32;
            h += (t + b) as i32;
        }
        // …client-side shadows sit inside it.
        if let [l, r, t, b] = cardinals(win, gtk_ext, 4)[..] {
            x += l as i32;
            y += t as i32;
            w -= (l + r) as i32;
            h -= (t + b) as i32;
        }
        if w > 8 && h > 8 {
            out.push((x, y, w, h));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_handles_spaces_and_utf8() {
        assert_eq!(
            percent_decode("/home/a/Pictures/Screenshot%20from%202024.png"),
            "/home/a/Pictures/Screenshot from 2024.png"
        );
        assert_eq!(percent_decode("/tmp/%C3%A9.png"), "/tmp/é.png");
        assert_eq!(percent_decode("/tmp/100%"), "/tmp/100%");
    }

    #[test]
    fn crop_copies_the_right_rows() {
        let mut data = Vec::new();
        for i in 0..16u8 {
            data.extend_from_slice(&[i, i, i, 255]);
        }
        let f = Frame {
            width: 4,
            height: 4,
            data,
        };
        let c = f.crop(1, 2, 2, 2);
        assert_eq!((c.width, c.height), (2, 2));
        let firsts: Vec<u8> = c.data.chunks(4).map(|p| p[0]).collect();
        assert_eq!(firsts, vec![9, 10, 13, 14]);
    }

    #[test]
    fn rgba_round_trip_is_lossless_for_opaque_pixels() {
        let img =
            image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 200, 100, 50, 255]).unwrap();
        let f = Frame::from_rgba(img.clone());
        assert_eq!(&f.data[..4], &[30, 20, 10, 255]);
        let back = bgra_to_rgba(&f.data, 2, 1, 8);
        assert_eq!(back, img);
    }
}
