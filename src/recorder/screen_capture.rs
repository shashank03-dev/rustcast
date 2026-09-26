//! Full-screen capture on X11 that leaves RustCast out of the video.
//!
//! Each frame grabs the monitor from the root window (MIT-SHM when possible).
//! Wherever one of RustCast's own windows is on screen — the REC pill, the
//! launcher opened mid-recording, a screenshot thumbnail — that area is
//! rebuilt without it:
//!
//! 1. start from the last clean frame (what was there before RustCast showed
//!    up — wallpaper, panels),
//! 2. draw every other top-level window intersecting it, bottom to top, from
//!    its Composite pixmap (complete even where RustCast covers it).
//!
//! So the video shows the desktop as if RustCast weren't there, while you can
//! still use RustCast during the recording.

use std::collections::{HashMap, HashSet};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, MapState, Window};
use x11rb::rust_connection::RustConnection;

use super::frame::{self, BPP, RectI, blend_cursor};
use crate::platform::linux::x11::Rect;

/// How often (in frames) windows judged "not RustCast" are checked again (in
/// case their properties were set late) and closed windows are forgotten.
const RECHECK_EVERY: u32 = 15;

struct Shm {
    seg: u32,
    addr: *mut u8,
    size: usize,
}

pub struct ScreenCapture {
    conn: RustConnection,
    root: Window,
    area: Rect,
    shm: Option<Shm>,
    shm_ok: bool,
    xfixes: bool,
    opacity_atom: u32,
    pid_atom: u32,
    /// Last frame with RustCast painted out; the background for new holes.
    clean: Option<Vec<u8>>,
    /// Visible top-level windows (WM frames) → whether they are RustCast's.
    /// Every new window is classified on the frame it first becomes visible,
    /// so it never slips into the clean background.
    known: HashMap<Window, bool>,
    frames: u32,
    /// Windows we redirected (to read their pixmaps) and must un-redirect.
    redirected: HashSet<Window>,
}

impl ScreenCapture {
    pub fn new(area: Rect) -> Result<Self, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("cannot connect to X server: {e}"))?;
        let root = conn.setup().roots[screen].root;
        let composite = conn
            .composite_query_version(0, 4)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        if !composite {
            return Err("the X server has no Composite extension".to_string());
        }
        let shm_ok = conn
            .shm_query_version()
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        let xfixes = conn
            .xfixes_query_version(4, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        let intern = |name: &[u8]| {
            conn.intern_atom(false, name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.atom)
                .unwrap_or(x11rb::NONE)
        };
        let opacity_atom = intern(b"_NET_WM_WINDOW_OPACITY");
        let pid_atom = intern(b"_NET_WM_PID");
        Ok(ScreenCapture {
            conn,
            root,
            area,
            shm: None,
            shm_ok,
            xfixes,
            opacity_atom,
            pid_atom,
            clean: None,
            known: HashMap::new(),
            frames: 0,
            redirected: HashSet::new(),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.area.w, self.area.h)
    }

    fn ensure_shm(&mut self, len: usize) -> bool {
        if self.shm.as_ref().is_some_and(|s| s.size >= len) {
            return true;
        }
        self.drop_shm();
        // SAFETY: plain SysV shm calls; failures are checked.
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, len, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            self.shm_ok = false;
            return false;
        }
        let addr = unsafe { libc::shmat(id, std::ptr::null(), 0) };
        if addr as isize == -1 {
            unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            self.shm_ok = false;
            return false;
        }
        let seg = self.conn.generate_id().ok().and_then(|seg| {
            self.conn
                .shm_attach(seg, id as u32, false)
                .ok()?
                .check()
                .ok()?;
            Some(seg)
        });
        unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
        match seg {
            Some(seg) => {
                self.shm = Some(Shm {
                    seg,
                    addr: addr as *mut u8,
                    size: len,
                });
                true
            }
            None => {
                unsafe { libc::shmdt(addr) };
                self.shm_ok = false;
                false
            }
        }
    }

    fn drop_shm(&mut self) {
        if let Some(shm) = self.shm.take() {
            let _ = self.conn.shm_detach(shm.seg);
            let _ = self.conn.flush();
            // SAFETY: `addr` came from a successful shmat.
            unsafe { libc::shmdt(shm.addr as *const libc::c_void) };
        }
    }

    /// Read a rectangle of `drawable` into `out` (tightly packed, 32 bpp).
    /// Returns the image depth.
    fn read(
        &mut self,
        drawable: u32,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        out: &mut Vec<u8>,
    ) -> Option<u8> {
        let len = w as usize * h as usize * BPP;
        if self.shm_ok && self.ensure_shm(len) {
            let shm = self.shm.as_ref().expect("ensured");
            if let Some(reply) = self
                .conn
                .shm_get_image(
                    drawable,
                    x,
                    y,
                    w,
                    h,
                    !0,
                    ImageFormat::Z_PIXMAP.into(),
                    shm.seg,
                    0,
                )
                .ok()
                .and_then(|c| c.reply().ok())
            {
                out.resize(len, 0);
                // SAFETY: segment is >= len and the reply means it is written.
                out.copy_from_slice(unsafe { std::slice::from_raw_parts(shm.addr, len) });
                return Some(reply.depth);
            }
        }
        let img = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, drawable, x, y, w, h, !0)
            .ok()?
            .reply()
            .ok()?;
        if img.data.len() < len {
            return None;
        }
        out.clear();
        out.extend_from_slice(&img.data[..len]);
        Some(img.depth)
    }

    /// WM_CLASS and pid of a window (what [`super::is_rustcast_window`] checks).
    fn describe(&self, w: Window) -> crate::platform::linux::x11::ClientWindow {
        let class = self
            .conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).replace('\0', " "))
            .unwrap_or_default();
        let pid = self
            .conn
            .get_property(false, w, self.pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()));
        crate::platform::linux::x11::ClientWindow {
            id: w,
            title: String::new(),
            class,
            pid,
            minimized: false,
        }
    }

    /// A top-level is RustCast's if it, or a client window directly inside it
    /// (window-manager frame), belongs to RustCast.
    fn toplevel_is_rustcast(&self, top: Window) -> bool {
        if super::is_rustcast_window(&self.describe(top)) {
            return true;
        }
        self.conn
            .query_tree(top)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|t| {
                t.children
                    .iter()
                    .any(|c| super::is_rustcast_window(&self.describe(*c)))
            })
    }

    fn toplevels(&self) -> Vec<Window> {
        self.conn
            .query_tree(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|t| t.children)
            .unwrap_or_default()
    }

    /// Root-relative outer rectangle of a viewable window.
    fn outer_rect(&self, w: Window) -> Option<RectI> {
        let attrs = self.conn.get_window_attributes(w).ok()?.reply().ok()?;
        if attrs.map_state != MapState::VIEWABLE {
            return None;
        }
        let g = self.conn.get_geometry(w).ok()?.reply().ok()?;
        let bw = g.border_width as i32;
        Some(RectI::new(
            g.x as i32,
            g.y as i32,
            g.width as i32 + 2 * bw,
            g.height as i32 + 2 * bw,
        ))
    }

    /// Fully transparent windows (e.g. a ghosted locked window) are invisible
    /// on screen, so they must not be drawn into the patch either.
    fn invisible(&self, top: Window) -> bool {
        let zero = |w: Window| {
            self.conn
                .get_property(false, w, self.opacity_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().and_then(|mut v| v.next()))
                == Some(0)
        };
        zero(top)
            || self
                .conn
                .query_tree(top)
                .ok()
                .and_then(|c| c.reply().ok())
                .is_some_and(|t| t.children.iter().any(|c| zero(*c)))
    }

    /// Draw window `top`'s part inside `region` (canvas coordinates) onto `out`.
    fn paint_window(&mut self, out: &mut [u8], top: Window, rect: RectI, region: RectI) {
        let Some(part) = rect.intersect(&region) else {
            return;
        };
        if !self.redirected.contains(&top) {
            if let Ok(c) = self
                .conn
                .composite_redirect_window(top, Redirect::AUTOMATIC)
            {
                let _ = c.check();
            }
            self.redirected.insert(top);
        }
        let Ok(pixmap) = self.conn.generate_id() else {
            return;
        };
        let named = self
            .conn
            .composite_name_window_pixmap(top, pixmap)
            .ok()
            .is_some_and(|c| c.check().is_ok());
        if !named {
            return;
        }
        // Both are canvas coordinates; the pixmap origin is the window's outer corner.
        let sx = (part.x - rect.x) as i16;
        let sy = (part.y - rect.y) as i16;
        let mut patch = Vec::new();
        let depth = self.read(pixmap, sx, sy, part.w as u16, part.h as u16, &mut patch);
        let _ = self.conn.free_pixmap(pixmap);
        if let Some(depth) = depth {
            frame::draw_patch(
                out,
                self.area.w,
                self.area.h,
                &patch,
                part.x,
                part.y,
                part.w as u32,
                part.h as u32,
                depth == 32,
            );
        }
    }

    /// Grab one frame into `out` (BGRx, monitor-sized) with RustCast removed.
    pub fn grab(&mut self, out: &mut Vec<u8>, draw_cursor: bool) -> Result<(), String> {
        let (w, h) = (self.area.w, self.area.h);
        let depth = self.read(
            self.root,
            self.area.x as i16,
            self.area.y as i16,
            w as u16,
            h as u16,
            out,
        );
        if depth.is_none() {
            return Err("cannot read the screen".to_string());
        }

        let tops = self.toplevels();
        let recheck = self.frames.is_multiple_of(RECHECK_EVERY);
        self.frames = self.frames.wrapping_add(1);
        if recheck {
            self.known.retain(|w, _| tops.contains(w));
        }
        for &top in &tops {
            let decided = self.known.get(&top).copied();
            if decided == Some(true) || (decided == Some(false) && !recheck) {
                continue;
            }
            // Only judge windows that are actually on screen: a frame the
            // window manager is still building has no client inside yet.
            if self.outer_rect(top).is_some() {
                let rc = self.toplevel_is_rustcast(top);
                self.known.insert(top, rc);
            }
        }
        let is_rc = |t: &Window| self.known.get(t) == Some(&true);

        // Canvas-space rectangles covered by RustCast.
        let canvas = RectI::new(0, 0, w as i32, h as i32);
        let offset = |r: RectI| RectI::new(r.x - self.area.x, r.y - self.area.y, r.w, r.h);
        let holes: Vec<RectI> = tops
            .iter()
            .filter(|t| is_rc(t))
            .filter_map(|t| self.outer_rect(*t))
            .filter_map(|r| offset(r).intersect(&canvas))
            .collect();

        if !holes.is_empty() {
            // Other windows, bottom to top, with their canvas rectangles.
            let others: Vec<(Window, RectI)> = tops
                .iter()
                .filter(|t| !is_rc(t))
                .filter_map(|t| Some((*t, offset(self.outer_rect(*t)?))))
                .filter(|(_, r)| holes.iter().any(|h| h.intersect(r).is_some()))
                .filter(|(t, _)| !self.invisible(*t))
                .collect();
            for hole in &holes {
                match &self.clean {
                    Some(clean) => frame::copy_rect(out, clean, w, h, *hole),
                    None => frame::fill_rect(out, w, h, *hole, [0, 0, 0]),
                }
                for (top, rect) in &others {
                    self.paint_window(out, *top, *rect, *hole);
                }
            }
        }

        // Remember the clean frame (before the cursor is drawn on it).
        match self.clean.as_mut() {
            Some(clean) if clean.len() == out.len() => clean.copy_from_slice(out),
            _ => self.clean = Some(out.clone()),
        }

        if draw_cursor
            && self.xfixes
            && let Some(cur) = self
                .conn
                .xfixes_get_cursor_image()
                .ok()
                .and_then(|c| c.reply().ok())
        {
            blend_cursor(
                out,
                w,
                h,
                w as usize * BPP,
                &cur.cursor_image,
                cur.width as u32,
                cur.height as u32,
                cur.x as i32 - cur.xhot as i32 - self.area.x,
                cur.y as i32 - cur.yhot as i32 - self.area.y,
            );
        }
        Ok(())
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.drop_shm();
        for w in self.redirected.drain() {
            if let Ok(c) = self
                .conn
                .composite_unredirect_window(w, Redirect::AUTOMATIC)
            {
                let _ = c.check();
            }
        }
        let _ = self.conn.flush();
    }
}
