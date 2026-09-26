//! Occlusion-proof capture of a single X11 window.
//!
//! The target window is redirected with the Composite extension
//! (`RedirectWindow`, *automatic* mode). The X server then keeps the window's
//! full contents in an off-screen pixmap — independently of whatever is stacked
//! on top of it — while still painting it to the screen as normal. Every frame
//! is read from that pixmap (via MIT-SHM when available), so:
//!
//! - windows dragged over the locked window never show up in the recording,
//! - moving the window around doesn't matter (we capture the window, not a
//!   screen region),
//! - nothing turns black when the window is covered.
//!
//! A *minimized* window is unmapped by the window manager, and an unmapped
//! window has no contents at all. So when the locked window gets minimized we
//! "ghost" it instead: it is mapped again but made fully transparent,
//! click-through and kept below other windows. To the user it is gone, yet the
//! application keeps rendering and the recording carries on. Activating the
//! window again (e.g. from the dock) brings it back; when the recording stops a
//! ghosted window is returned to its minimized state.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ClipOrdering, ConnectionExt as _, EventMask, ImageFormat,
    MapState, PropMode, Rectangle, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::frame::{BPP, blend_cursor};

/// After ghosting, focus that lands on the ghost within this window is the
/// window manager reacting to the re-map, not the user — hand it back.
const GHOST_SETTLE: Duration = Duration::from_millis(1200);

/// A System V shared-memory segment attached to the X server.
struct Shm {
    seg: u32,
    addr: *mut u8,
    size: usize,
}

struct Atoms {
    wm_state: Atom,
    wm_change_state: Atom,
    net_wm_state: Atom,
    net_wm_state_hidden: Atom,
    net_wm_state_below: Atom,
    net_active_window: Atom,
    net_wm_opacity: Atom,
    gtk_frame_extents: Atom,
    net_wm_desktop: Atom,
    net_current_desktop: Atom,
    net_wm_state_sticky: Atom,
}

/// Saved state of a window we ghosted, so it can be restored exactly.
struct GhostState {
    since: Instant,
    prev_active: Option<Window>,
    /// Focus has been somewhere else since ghosting; a later activation of the
    /// ghost is then the user bringing it back.
    seen_other: bool,
    /// The window was minimized when ghosted (restore to minimized on stop).
    was_minimized: bool,
    /// The window lives on another workspace: it was made sticky so it keeps
    /// rendering; this is its real workspace.
    desktop: Option<u32>,
    /// (window, original opacity, original input-shape rectangles)
    saved: Vec<(Window, Option<u32>, Option<Vec<Rectangle>>)>,
}

/// The outcome of grabbing one frame.
pub enum Grab {
    /// A fresh image of `width`×`height` pixels was written to the buffer;
    /// `(x, y)` is its top-left corner in root (screen) coordinates.
    Frame {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// The window exists but has no contents right now (unmapped, minimized,
    /// mid-remap). The caller should keep showing the previous frame.
    Unavailable,
    /// The window was destroyed; recording should end.
    Gone,
}

pub struct WindowCapture {
    conn: RustConnection,
    root: Window,
    win: Window,
    /// Top-level ancestor (the window manager's frame), or `win` itself.
    frame: Window,
    atoms: Atoms,
    shm: Option<Shm>,
    shm_supported: bool,
    xfixes: bool,
    /// `_GTK_FRAME_EXTENTS` (left, right, top, bottom): client-side shadow
    /// margins that are cropped away so only the real window is recorded.
    extents: (u32, u32, u32, u32),
    frames_since_extents: u32,
    ghost: Option<GhostState>,
}

impl WindowCapture {
    /// Connect to the X server and redirect `win` so it can be captured
    /// regardless of occlusion.
    pub fn new(win: Window) -> Result<Self, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("cannot connect to X server: {e}"))?;
        let root = conn.setup().roots[screen].root;

        let composite = conn
            .composite_query_version(0, 4)
            .ok()
            .and_then(|c| c.reply().ok());
        if composite.is_none() {
            return Err("the X server has no Composite extension".to_string());
        }

        // Validate the window up front for a clear error message.
        conn.get_geometry(win)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|_| "that window no longer exists".to_string())?;

        // Automatic redirection keeps the window's contents in an off-screen
        // pixmap. It is allowed even when a compositing manager already holds
        // a (manual) redirection of the same hierarchy.
        if let Ok(cookie) = conn.composite_redirect_window(win, Redirect::AUTOMATIC) {
            let _ = cookie.check();
        }

        let intern = |name: &str| -> Atom {
            conn.intern_atom(false, name.as_bytes())
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.atom)
                .unwrap_or(x11rb::NONE)
        };
        let atoms = Atoms {
            wm_state: intern("WM_STATE"),
            wm_change_state: intern("WM_CHANGE_STATE"),
            net_wm_state: intern("_NET_WM_STATE"),
            net_wm_state_hidden: intern("_NET_WM_STATE_HIDDEN"),
            net_wm_state_below: intern("_NET_WM_STATE_BELOW"),
            net_active_window: intern("_NET_ACTIVE_WINDOW"),
            net_wm_opacity: intern("_NET_WM_WINDOW_OPACITY"),
            gtk_frame_extents: intern("_GTK_FRAME_EXTENTS"),
            net_wm_desktop: intern("_NET_WM_DESKTOP"),
            net_current_desktop: intern("_NET_CURRENT_DESKTOP"),
            net_wm_state_sticky: intern("_NET_WM_STATE_STICKY"),
        };

        let shm_supported = conn
            .shm_query_version()
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        let xfixes = conn
            .xfixes_query_version(4, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();

        let mut cap = WindowCapture {
            frame: win,
            conn,
            root,
            win,
            atoms,
            shm: None,
            shm_supported,
            xfixes,
            extents: (0, 0, 0, 0),
            frames_since_extents: u32::MAX,
            ghost: None,
        };
        cap.frame = cap.toplevel_of(win);
        Ok(cap)
    }

    /// The current size of the window content that will be recorded (after
    /// cropping client-side shadows), or `None` if the window is gone.
    pub fn content_size(&mut self) -> Option<(u32, u32)> {
        let geom = self.conn.get_geometry(self.win).ok()?.reply().ok()?;
        self.refresh_extents();
        let (l, r, t, b) = self.extents;
        Some((
            (geom.width as u32).saturating_sub(l + r).max(1),
            (geom.height as u32).saturating_sub(t + b).max(1),
        ))
    }

    /// Walk up the window tree to the child of the root (the WM frame).
    fn toplevel_of(&self, mut w: Window) -> Window {
        for _ in 0..16 {
            let Some(tree) = self.conn.query_tree(w).ok().and_then(|c| c.reply().ok()) else {
                break;
            };
            if tree.parent == self.root || tree.parent == x11rb::NONE {
                return w;
            }
            w = tree.parent;
        }
        w
    }

    fn refresh_extents(&mut self) {
        // Re-read occasionally; apps change it when (un)maximizing.
        if self.frames_since_extents < 30 {
            self.frames_since_extents += 1;
            return;
        }
        self.frames_since_extents = 0;
        // Window managers may re-frame a window (e.g. after a theme change).
        self.frame = self.toplevel_of(self.win);
        self.extents = self
            .conn
            .get_property(
                false,
                self.win,
                self.atoms.gtk_frame_extents,
                AtomEnum::CARDINAL,
                0,
                4,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
            .filter(|v| v.len() == 4)
            .map(|v| (v[0], v[1], v[2], v[3]))
            .unwrap_or((0, 0, 0, 0));
    }

    /// Grab the window's current contents into `out` (tightly packed BGRx).
    pub fn grab(&mut self, out: &mut Vec<u8>, draw_cursor: bool) -> Grab {
        let Some(geom) = self
            .conn
            .get_geometry(self.win)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return Grab::Gone;
        };
        let viewable = self
            .conn
            .get_window_attributes(self.win)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|a| a.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        if !viewable {
            return Grab::Unavailable;
        }

        self.refresh_extents();
        let (l, r, t, b) = self.extents;
        let bw = geom.border_width as u32;
        let (full_w, full_h) = (geom.width as u32, geom.height as u32);
        // Ignore nonsensical extents rather than cropping the window away.
        let (l, r, t, b) = if l + r >= full_w || t + b >= full_h {
            (0, 0, 0, 0)
        } else {
            (l, r, t, b)
        };
        let (w, h) = (full_w - l - r, full_h - t - b);
        let (sx, sy) = ((bw + l) as i16, (bw + t) as i16);

        // Name a fresh pixmap each frame: the backing pixmap is replaced by the
        // server whenever the window is resized or re-mapped, and naming is a
        // cheap reference, not a copy.
        let Ok(pixmap) = self.conn.generate_id() else {
            return Grab::Unavailable;
        };
        let named = self
            .conn
            .composite_name_window_pixmap(self.win, pixmap)
            .ok()
            .map(|c| c.check().is_ok())
            .unwrap_or(false);
        if !named {
            return Grab::Unavailable;
        }

        let len = w as usize * h as usize * BPP;
        let ok = if self.shm_supported && self.ensure_shm(len) {
            let shm = self.shm.as_ref().expect("shm just ensured");
            let reply = self
                .conn
                .shm_get_image(
                    pixmap,
                    sx,
                    sy,
                    w as u16,
                    h as u16,
                    !0,
                    ImageFormat::Z_PIXMAP.into(),
                    shm.seg,
                    0,
                )
                .ok()
                .and_then(|c| c.reply().ok());
            match reply {
                Some(_) => {
                    out.resize(len, 0);
                    // SAFETY: the segment is at least `len` bytes (ensure_shm)
                    // and the server finished writing it (we have the reply).
                    let src = unsafe { std::slice::from_raw_parts(shm.addr, len) };
                    out.copy_from_slice(src);
                    true
                }
                None => {
                    // e.g. a remote display; fall back to plain GetImage for good.
                    self.shm_supported = false;
                    false
                }
            }
        } else {
            false
        };

        let ok = ok || {
            match self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    pixmap,
                    sx,
                    sy,
                    w as u16,
                    h as u16,
                    !0,
                )
                .ok()
                .and_then(|c| c.reply().ok())
            {
                Some(img) if img.data.len() >= len => {
                    out.clear();
                    out.extend_from_slice(&img.data[..len]);
                    true
                }
                _ => false,
            }
        };
        let _ = self.conn.free_pixmap(pixmap);
        if !ok {
            return Grab::Unavailable;
        }

        let (ox, oy) = self
            .conn
            .translate_coordinates(self.win, self.root, 0, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|o| (o.dst_x as i32 + l as i32, o.dst_y as i32 + t as i32))
            .unwrap_or((0, 0));
        // Only the recorded window may appear: the pointer is drawn only while
        // it is really over this window, not over something covering it.
        if draw_cursor && self.xfixes && self.pointer_on_window() {
            self.draw_cursor(out, w, h, ox, oy);
        }
        Grab::Frame {
            x: ox,
            y: oy,
            width: w,
            height: h,
        }
    }

    /// Whether the pointer is over this window *and* this window is the
    /// topmost one there (so it is what the user actually sees/points at).
    /// The server resolves the top-level under the pointer, honouring input
    /// shapes — so a ghosted (click-through) window, a window stacked on top
    /// or a RustCast window all hide the pointer from this recording.
    fn pointer_on_window(&self) -> bool {
        let Some(p) = self
            .conn
            .query_pointer(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return false;
        };
        p.same_screen && p.child != x11rb::NONE && (p.child == self.frame || p.child == self.win)
    }

    /// Draw the pointer onto an image whose top-left is at root `(ox, oy)`.
    fn draw_cursor(&self, out: &mut [u8], w: u32, h: u32, ox: i32, oy: i32) {
        let Some(cur) = self
            .conn
            .xfixes_get_cursor_image()
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return;
        };
        blend_cursor(
            out,
            w,
            h,
            w as usize * BPP,
            &cur.cursor_image,
            cur.width as u32,
            cur.height as u32,
            cur.x as i32 - cur.xhot as i32 - ox,
            cur.y as i32 - cur.yhot as i32 - oy,
        );
    }

    /// Make sure an attached SHM segment of at least `len` bytes exists.
    fn ensure_shm(&mut self, len: usize) -> bool {
        if self.shm.as_ref().is_some_and(|s| s.size >= len) {
            return true;
        }
        self.drop_shm();
        // Over-allocate a little so small resizes don't reallocate.
        let size = len + len / 4 + 4096;
        // SAFETY: plain SysV shm calls; failures are checked below.
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            self.shm_supported = false;
            return false;
        }
        let addr = unsafe { libc::shmat(id, std::ptr::null(), 0) };
        if addr as isize == -1 {
            unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            self.shm_supported = false;
            return false;
        }
        let attached = self.conn.generate_id().ok().and_then(|seg| {
            self.conn
                .shm_attach(seg, id as u32, false)
                .ok()?
                .check()
                .ok()?;
            Some(seg)
        });
        // Mark for deletion now; it lives until both sides detach.
        unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
        match attached {
            Some(seg) => {
                self.shm = Some(Shm {
                    seg,
                    addr: addr as *mut u8,
                    size,
                });
                true
            }
            None => {
                unsafe { libc::shmdt(addr) };
                self.shm_supported = false;
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

    // ----- minimize handling ------------------------------------------------

    fn active_window(&self) -> Option<Window> {
        self.conn
            .get_property(
                false,
                self.root,
                self.atoms.net_active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
            .filter(|w| *w != 0)
    }

    /// True when the user minimized the window (not merely left it behind on
    /// another workspace).
    pub fn is_minimized(&self) -> bool {
        let hidden = self
            .conn
            .get_property(
                false,
                self.win,
                self.atoms.net_wm_state,
                AtomEnum::ATOM,
                0,
                64,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
            .is_some_and(|v| v.contains(&self.atoms.net_wm_state_hidden));
        if hidden {
            return true;
        }
        // Some window managers also mark windows on *other workspaces* as
        // Iconic, so ICCCM IconicState alone only means "minimized" when the
        // window is on the workspace being shown.
        let iconic = self
            .conn
            .get_property(
                false,
                self.win,
                self.atoms.wm_state,
                self.atoms.wm_state,
                0,
                1,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()))
            == Some(3);
        iconic && self.other_desktop().is_none()
    }

    fn cardinal(&self, win: Window, atom: Atom) -> Option<u32> {
        self.conn
            .get_property(false, win, atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    }

    /// The window's workspace when it is on a *different* workspace than the
    /// one shown (windows there are unmapped by the window manager).
    pub fn other_desktop(&self) -> Option<u32> {
        let mine = self.cardinal(self.win, self.atoms.net_wm_desktop)?;
        let current = self.cardinal(self.root, self.atoms.net_current_desktop)?;
        (mine != 0xFFFF_FFFF && mine != current).then_some(mine)
    }

    pub fn is_ghosted(&self) -> bool {
        self.ghost.is_some()
    }

    fn send_wm_message(&self, type_: Atom, data: [u32; 5]) {
        let ev = ClientMessageEvent::new(32, self.win, type_, data);
        let _ = self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        );
    }

    /// Turn a window that stopped rendering — minimized, or left behind on
    /// another workspace — into an invisible, click-through window that keeps
    /// rendering (and therefore keeps being recorded).
    pub fn ghost(&mut self) {
        if let Some(ghost) = self.ghost.as_mut() {
            // Hidden again while ghosted (e.g. "show desktop"): map it again.
            if ghost.since.elapsed() > Duration::from_millis(500) {
                ghost.since = Instant::now();
                ghost.seen_other = false;
                let _ = self.conn.map_window(self.win);
                let _ = self.conn.flush();
            }
            return;
        }
        let was_minimized = self.is_minimized();
        let desktop = self.other_desktop();
        if !was_minimized && desktop.is_none() {
            return; // just mid-remap; the previous frame covers it
        }
        let prev_active = self.active_window().filter(|w| *w != self.win);
        // The frame may have been replaced while hidden; look it up again.
        self.frame = self.toplevel_of(self.win);

        let mut targets = vec![self.win];
        if self.frame != self.win {
            targets.push(self.frame);
        }
        let mut saved = Vec::new();
        for &w in &targets {
            let opacity = self
                .conn
                .get_property(
                    false,
                    w,
                    self.atoms.net_wm_opacity,
                    AtomEnum::CARDINAL,
                    0,
                    1,
                )
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().and_then(|mut v| v.next()));
            let shape = self
                .conn
                .shape_get_rectangles(w, SK::INPUT)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.rectangles);
            saved.push((w, opacity, shape));

            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                w,
                self.atoms.net_wm_opacity,
                AtomEnum::CARDINAL,
                &[0],
            );
            // Empty input region → pointer events fall through to what's below.
            let _ = self.conn.shape_rectangles(
                SO::SET,
                SK::INPUT,
                ClipOrdering::UNSORTED,
                w,
                0,
                0,
                &[],
            );
        }
        // Keep it under normal windows.
        self.send_wm_message(
            self.atoms.net_wm_state,
            [1, self.atoms.net_wm_state_below, 0, 1, 0],
        );
        if desktop.is_some() {
            // On every workspace → mapped (rendering) wherever the user is.
            // Window managers support one or both of these requests.
            self.send_wm_message(
                self.atoms.net_wm_state,
                [1, self.atoms.net_wm_state_sticky, 0, 1, 0],
            );
            self.send_wm_message(self.atoms.net_wm_desktop, [0xFFFF_FFFF, 2, 0, 0, 0]);
        }
        if was_minimized {
            // ICCCM: Iconic → Normal
            let _ = self.conn.map_window(self.win);
        }
        let _ = self.conn.flush();

        self.ghost = Some(GhostState {
            since: Instant::now(),
            prev_active,
            seen_other: false,
            was_minimized,
            desktop,
            saved,
        });
        log::info!(
            "Recorder: locked window {} — ghosted it to keep recording",
            if desktop.is_some() {
                "is on another workspace"
            } else {
                "minimized"
            }
        );
    }

    /// Some other window to hand keyboard focus to: the one focused before, or
    /// the topmost other client window.
    fn focus_fallback(&self, prev: Option<Window>) -> Option<Window> {
        prev.or_else(|| {
            let atom = self
                .conn
                .intern_atom(false, b"_NET_CLIENT_LIST_STACKING")
                .ok()?
                .reply()
                .ok()?
                .atom;
            let list: Vec<u32> = self
                .conn
                .get_property(false, self.root, atom, AtomEnum::WINDOW, 0, u32::MAX)
                .ok()?
                .reply()
                .ok()?
                .value32()?
                .collect();
            list.into_iter().rev().find(|w| *w != self.win)
        })
    }

    /// Called every frame while ghosted. Hands back focus that the WM gave the
    /// invisible ghost when it was re-mapped, and un-ghosts the window once the
    /// *user* activates it again (e.g. from the dock or alt-tab).
    pub fn tick_ghost(&mut self) {
        // Back on the window's own workspace: it is visible there again.
        if let Some(orig) = self.ghost.as_ref().and_then(|g| g.desktop)
            && self.cardinal(self.root, self.atoms.net_current_desktop) == Some(orig)
        {
            let minimized = self.ghost.as_ref().is_some_and(|g| g.was_minimized);
            log::info!("Recorder: back on the locked window's workspace");
            if minimized {
                // Still minimized by the user: stay ghosted, just not sticky.
                self.unstick(orig);
                if let Some(g) = self.ghost.as_mut() {
                    g.desktop = None;
                }
            } else {
                self.unghost(false);
            }
            return;
        }
        let active = self.active_window();
        let Some(ghost) = self.ghost.as_mut() else {
            return;
        };
        if ghost.desktop.is_some() {
            // Sticky ghost on another workspace: never keeps the keyboard, and
            // activation doesn't bring it back (it belongs elsewhere).
            if active == Some(self.win) {
                let prev = ghost.prev_active;
                self.hand_back_focus(prev);
            }
            return;
        }
        if active != Some(self.win) {
            if ghost.since.elapsed() > Duration::from_millis(150) {
                ghost.seen_other = true;
            }
            return;
        }
        if ghost.seen_other && ghost.since.elapsed() >= GHOST_SETTLE {
            log::info!("Recorder: ghosted window activated — restoring it");
            self.unghost(false);
            return;
        }
        // The WM focused the ghost as a side effect of mapping it: an invisible
        // window must not keep the keyboard.
        let prev = ghost.prev_active;
        self.hand_back_focus(prev);
    }

    fn hand_back_focus(&self, prev: Option<Window>) {
        match self.focus_fallback(prev) {
            Some(other) => {
                let ev = ClientMessageEvent::new(
                    32,
                    other,
                    self.atoms.net_active_window,
                    [2, 0, 0, 0, 0],
                );
                let _ = self.conn.send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    ev,
                );
            }
            None => {
                let _ = self.conn.set_input_focus(
                    x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                    x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                    x11rb::CURRENT_TIME,
                );
            }
        }
        let _ = self.conn.flush();
    }

    /// Undo "sticky" and put the window back on workspace `desktop`.
    fn unstick(&self, desktop: u32) {
        self.send_wm_message(
            self.atoms.net_wm_state,
            [0, self.atoms.net_wm_state_sticky, 0, 1, 0],
        );
        self.send_wm_message(self.atoms.net_wm_desktop, [desktop, 2, 0, 0, 0]);
        let _ = self.conn.flush();
    }

    /// Restore a ghosted window: back to its own workspace, and with
    /// `reminimize` a window the user had minimized goes back to minimized.
    pub fn unghost(&mut self, reminimize: bool) {
        let Some(ghost) = self.ghost.take() else {
            return;
        };
        for (w, opacity, shape) in ghost.saved {
            let _ = match opacity {
                Some(v) => self
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        w,
                        self.atoms.net_wm_opacity,
                        AtomEnum::CARDINAL,
                        &[v],
                    )
                    .map(|_| ()),
                None => self
                    .conn
                    .delete_property(w, self.atoms.net_wm_opacity)
                    .map(|_| ()),
            };
            let _ = match shape {
                Some(rects) => self
                    .conn
                    .shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, w, 0, 0, &rects)
                    .map(|_| ()),
                None => self
                    .conn
                    .shape_mask(SO::SET, SK::INPUT, w, 0, 0, x11rb::NONE)
                    .map(|_| ()),
            };
        }
        self.send_wm_message(
            self.atoms.net_wm_state,
            [0, self.atoms.net_wm_state_below, 0, 1, 0],
        );
        if let Some(desktop) = ghost.desktop {
            self.unstick(desktop);
        }
        // Only windows the user minimized go back to being minimized.
        if reminimize && ghost.was_minimized {
            // ICCCM WM_CHANGE_STATE → IconicState
            self.send_wm_message(self.atoms.wm_change_state, [3, 0, 0, 0, 0]);
        }
        let _ = self.conn.flush();
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        self.unghost(true);
        self.drop_shm();
        if let Ok(c) = self
            .conn
            .composite_unredirect_window(self.win, Redirect::AUTOMATIC)
        {
            let _ = c.check();
        }
        let _ = self.conn.flush();
    }
}
