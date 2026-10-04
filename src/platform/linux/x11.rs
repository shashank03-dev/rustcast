//! Low-level X11 helpers (EWMH window control, XTEST paste injection,
//! monitor geometry). Each helper opens a short-lived connection — these are
//! invoked occasionally (show/hide, tile, paste), so the cost is negligible.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, InputFocus, MapState, PropMode,
    Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const KEYSYM_CONTROL_L: u32 = 0xffe3;
const KEYSYM_V: u32 = 0x0076;

struct X11 {
    conn: RustConnection,
    root: Window,
}

impl X11 {
    fn open() -> Option<Self> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen_num].root;
        Some(X11 { conn, root })
    }

    fn atom(&self, name: &str) -> Option<Atom> {
        self.conn
            .intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    }

    /// Read a single-window CARDINAL/WINDOW property from a window.
    fn window_property(&self, win: Window, prop: Atom) -> Option<Window> {
        let reply = self
            .conn
            .get_property(false, win, prop, AtomEnum::ANY, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let vals: Vec<u32> = reply.value32()?.collect();
        vals.first().copied()
    }

    fn send_client_message(&self, win: Window, type_: Atom, data: [u32; 5]) -> Option<()> {
        let event = ClientMessageEvent::new(32, win, type_, data);
        self.conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .ok()?;
        self.conn.flush().ok()?;
        Some(())
    }
}

/// Return the currently active (focused) toplevel window, via `_NET_ACTIVE_WINDOW`.
pub fn active_window() -> Option<u32> {
    let x = X11::open()?;
    let atom = x.atom("_NET_ACTIVE_WINDOW")?;
    x.window_property(x.root, atom).filter(|w| *w != 0)
}

/// True when the active window belongs to another application, i.e. the user
/// clicked outside RustCast or switched apps. Stray focus-out events (focus
/// going nowhere, or staying on one of our own windows) return false.
pub fn another_app_is_active() -> bool {
    let Some(win) = active_window() else {
        return false;
    };
    match window_info(win) {
        Some(info) => info.pid != Some(std::process::id()),
        None => true,
    }
}

/// Raise and focus the given window (used to restore the previously-focused app
/// before pasting).
pub fn focus_window(win: u32) -> Option<()> {
    let x = X11::open()?;
    let atom = x.atom("_NET_ACTIVE_WINDOW")?;
    // source indication = 2 (pager), timestamp 0 (CurrentTime)
    x.send_client_message(win, atom, [2, 0, 0, 0, 0]);
    let _ = x
        .conn
        .set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME);
    let _ = x.conn.flush();
    Some(())
}

/// Raise a window to the top of the stack, even when it is already the
/// active window (activation alone does not restack it then).
pub fn raise_window(win: u32) -> Option<()> {
    let x = X11::open()?;
    let atom = x.atom("_NET_RESTACK_WINDOW")?;
    // source indication = 2 (pager), sibling = None, detail = Above (0)
    x.send_client_message(win, atom, [2, 0, 0, 0, 0]);
    // Window managers that ignore the EWMH request still honour a plain
    // stacking request for their client.
    use x11rb::protocol::xproto::{ConfigureWindowAux, StackMode};
    let _ = x
        .conn
        .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
    x.conn.flush().ok()?;
    Some(())
}

fn keysym_to_keycode(x: &X11, keysym: u32) -> Option<u8> {
    let setup = x.conn.setup();
    let min = setup.min_keycode;
    let max = setup.max_keycode;
    let count = max - min + 1;
    let mapping = x.conn.get_keyboard_mapping(min, count).ok()?.reply().ok()?;
    let per = mapping.keysyms_per_keycode as usize;
    for (i, chunk) in mapping.keysyms.chunks(per).enumerate() {
        if chunk.iter().any(|&k| k == keysym) {
            return Some(min + i as u8);
        }
    }
    None
}

/// Synthesize Ctrl+V into the currently-focused window using the XTEST
/// extension. Equivalent to the macOS `simulate_paste`.
pub fn send_paste() -> Option<()> {
    let x = X11::open()?;
    let ctrl = keysym_to_keycode(&x, KEYSYM_CONTROL_L)?;
    let v = keysym_to_keycode(&x, KEYSYM_V)?;

    const PRESS: u8 = 2; // KeyPress
    const RELEASE: u8 = 3; // KeyRelease

    x.conn
        .xtest_fake_input(PRESS, ctrl, 0, x.root, 0, 0, 0)
        .ok()?;
    x.conn.xtest_fake_input(PRESS, v, 0, x.root, 0, 0, 0).ok()?;
    x.conn
        .xtest_fake_input(RELEASE, v, 0, x.root, 0, 0, 0)
        .ok()?;
    x.conn
        .xtest_fake_input(RELEASE, ctrl, 0, x.root, 0, 0, 0)
        .ok()?;
    x.conn.flush().ok()?;
    Some(())
}

/// A rectangle in root-window (top-left origin, y-down) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Work area of the monitor containing `win` (full monitor geometry; panel
/// struts are not subtracted). Falls back to the first monitor / root size.
pub fn monitor_workarea(win: u32) -> Option<Rect> {
    use x11rb::protocol::randr::ConnectionExt as _;
    let x = X11::open()?;

    // Window position (translate to root coordinates).
    let geom = x.conn.get_geometry(win).ok()?.reply().ok()?;
    let trans = x
        .conn
        .translate_coordinates(win, x.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    let cx = trans.dst_x as i32 + geom.width as i32 / 2;
    let cy = trans.dst_y as i32 + geom.height as i32 / 2;

    let monitors = x.conn.randr_get_monitors(x.root, true).ok()?.reply().ok()?;
    let mut chosen: Option<Rect> = None;
    let mut primary: Option<Rect> = None;
    for m in monitors.monitors.iter() {
        let r = Rect {
            x: m.x as i32,
            y: m.y as i32,
            w: m.width as u32,
            h: m.height as u32,
        };
        if primary.is_none() || m.primary {
            primary = Some(r);
        }
        if cx >= r.x && cx < r.x + r.w as i32 && cy >= r.y && cy < r.y + r.h as i32 {
            chosen = Some(r);
        }
    }
    chosen.or(primary)
}

/// Move and resize a window using `_NET_MOVERESIZE_WINDOW`, clearing maximized
/// states first so tiling takes effect.
pub fn move_resize(win: u32, rect: Rect) -> Option<()> {
    let x = X11::open()?;

    // Un-maximize / un-fullscreen so geometry changes are honoured.
    if let (Some(state), Some(max_v), Some(max_h), Some(full)) = (
        x.atom("_NET_WM_STATE"),
        x.atom("_NET_WM_STATE_MAXIMIZED_VERT"),
        x.atom("_NET_WM_STATE_MAXIMIZED_HORZ"),
        x.atom("_NET_WM_STATE_FULLSCREEN"),
    ) {
        const REMOVE: u32 = 0;
        x.send_client_message(win, state, [REMOVE, max_v, max_h, 1, 0]);
        x.send_client_message(win, state, [REMOVE, full, 0, 1, 0]);
    }

    if let Some(moveresize) = x.atom("_NET_MOVERESIZE_WINDOW") {
        // gravity 0 (default) + flags: x,y,w,h provided (bits 8-11), source 2 (bits 12-13).
        let flags: u32 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (2 << 12);
        x.send_client_message(
            win,
            moveresize,
            [flags, rect.x as u32, rect.y as u32, rect.w, rect.h],
        );
    }
    Some(())
}

/// Move a window's top-left corner to root coordinates `(x_pos, y_pos)` without
/// changing its size. Used to place the launcher Spotlight-style; the window then
/// grows downward from this anchor as results stream in.
///
/// `_NET_MOVERESIZE_WINDOW` is a request to the window manager (sent to the root
/// with SUBSTRUCTURE_REDIRECT): the WM only honours it for a window it is already
/// managing. A window opened via the toggle hotkey is positioned from an iced
/// `window::run` callback that can fire before mutter has finished mapping the
/// freshly-created window, in which case the WM silently drops the move and the
/// window stays at the compositor's default spot (top-left) instead of centred —
/// an intermittent, race-dependent misplacement. So we first wait (briefly) for
/// the window to become viewable, then send the move, then re-send once to cover
/// compositors that finish managing a beat after the window maps.
///
/// This blocks for up to ~500 ms, so call it off the iced runtime thread (e.g.
/// from a spawned thread in `position_launcher`).
pub fn move_window(win: u32, x_pos: i32, y_pos: i32) -> Option<()> {
    let x = X11::open()?;
    let moveresize = x.atom("_NET_MOVERESIZE_WINDOW")?;
    // gravity 0 (default) + flags: only x,y provided (bits 8-9), source 2 (bits 12-13).
    let flags: u32 = (1 << 8) | (1 << 9) | (2 << 12);
    let data = [flags, x_pos as u32, y_pos as u32, 0, 0];

    // Wait for the WM to actually be managing (window viewable) before moving it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        let viewable = x
            .conn
            .get_window_attributes(win)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|a| a.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        if viewable {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    x.send_client_message(win, moveresize, data);
    std::thread::sleep(std::time::Duration::from_millis(40));
    x.send_client_message(win, moveresize, data);
    Some(())
}

/// Geometry of the primary monitor (falling back to the first monitor). Unlike
/// [`monitor_workarea`], this needs no existing window, so it can place the
/// launcher before it has a window to query.
pub fn primary_monitor() -> Option<Rect> {
    use x11rb::protocol::randr::ConnectionExt as _;
    let x = X11::open()?;
    let monitors = x.conn.randr_get_monitors(x.root, true).ok()?.reply().ok()?;
    let mut primary: Option<Rect> = None;
    for m in monitors.monitors.iter() {
        let r = Rect {
            x: m.x as i32,
            y: m.y as i32,
            w: m.width as u32,
            h: m.height as u32,
        };
        if primary.is_none() || m.primary {
            primary = Some(r);
        }
    }
    primary
}

/// Mark a (mapped) window as always-on-top, sticky across all workspaces, and
/// hidden from taskbar/pager — the Linux analogue of the macOS floating /
/// can-join-all-spaces window config.
pub fn set_overlay_states(win: u32) -> Option<()> {
    let x = X11::open()?;
    let state = x.atom("_NET_WM_STATE")?;
    const ADD: u32 = 1;
    for name in [
        "_NET_WM_STATE_ABOVE",
        "_NET_WM_STATE_STICKY",
        "_NET_WM_STATE_SKIP_TASKBAR",
        "_NET_WM_STATE_SKIP_PAGER",
    ] {
        if let Some(s) = x.atom(name) {
            x.send_client_message(win, state, [ADD, s, 0, 1, 0]);
        }
    }
    Some(())
}

/// KWin's blur-behind protocol: a window lists the rectangles (x, y, w, h) the
/// compositor should blur behind it. KWin announces support by setting this
/// property on the root window while its blur effect is loaded.
const BLUR_BEHIND: &str = "_KDE_NET_WM_BLUR_BEHIND_REGION";

/// True when a running compositor blurs behind windows that ask for it.
pub fn compositor_blurs() -> bool {
    let Some(x) = X11::open() else {
        return false;
    };
    let Some(atom) = x.atom(BLUR_BEHIND) else {
        return false;
    };
    x.conn
        .get_property(false, x.root, atom, AtomEnum::ANY, 0, 0)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.type_ != u32::from(AtomEnum::NONE))
}

/// Rectangles covering a `w`×`h` rounded rectangle of corner `radius`, one
/// per corner row plus the body, so the blur stops at the rounded edge instead
/// of showing in the transparent corners.
pub fn rounded_region(w: u32, h: u32, radius: u32) -> Vec<u32> {
    let r = radius.min(w / 2).min(h / 2);
    let mut rects = Vec::with_capacity(4 * (r as usize * 2 + 1));
    for i in 0..r {
        let dy = r as f32 - i as f32 - 0.5;
        let inset = (r as f32 - (r as f32 * r as f32 - dy * dy).max(0.0).sqrt()).round() as u32;
        let row_w = w.saturating_sub(inset * 2);
        rects.extend_from_slice(&[inset, i, row_w, 1]);
        rects.extend_from_slice(&[inset, h - 1 - i, row_w, 1]);
    }
    if h > 2 * r {
        rects.extend_from_slice(&[0, r, w, h - 2 * r]);
    }
    rects
}

/// Ask the compositor to blur behind `win`, clipped to its rounded corners.
///
/// `logical_width` and `radius` are in iced's logical pixels; the window's
/// real size is read back from the server, which also gives the scale factor.
/// A resize is granted by the window manager a moment after it is requested,
/// so the region is refreshed while the size settles (~0.4 s, off-thread).
pub fn set_blur_behind(win: u32, logical_width: f32, radius: f32) {
    std::thread::spawn(move || {
        let Some(x) = X11::open() else { return };
        let Some(atom) = x.atom(BLUR_BEHIND) else {
            return;
        };
        let mut last = (0, 0);
        for _ in 0..25 {
            if let Some(g) = x.conn.get_geometry(win).ok().and_then(|c| c.reply().ok()) {
                let size = (g.width as u32, g.height as u32);
                if size != last && size.0 > 0 && size.1 > 0 {
                    last = size;
                    let scale = (size.0 as f32 / logical_width.max(1.0)).clamp(1.0, 4.0);
                    let region = rounded_region(size.0, size.1, (radius * scale).round() as u32);
                    let _ = x.conn.change_property32(
                        PropMode::REPLACE,
                        win,
                        atom,
                        AtomEnum::CARDINAL,
                        &region,
                    );
                    let _ = x.conn.flush();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    });
}

/// A normal toplevel application window, as listed in `_NET_CLIENT_LIST`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientWindow {
    pub id: u32,
    pub title: String,
    /// `WM_CLASS` with the NUL separator replaced by a space ("instance Class").
    pub class: String,
    pub pid: Option<u32>,
    pub minimized: bool,
}

/// Enumerate normal toplevel windows from `_NET_CLIENT_LIST` (stacking order
/// is not guaranteed), with their title, class, pid and minimized state.
pub fn client_windows() -> Vec<ClientWindow> {
    let Some(x) = X11::open() else {
        return Vec::new();
    };
    let Some(list_atom) = x.atom("_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    let reply = match x
        .conn
        .get_property(false, x.root, list_atom, AtomEnum::WINDOW, 0, u32::MAX)
    {
        Ok(c) => match c.reply() {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };
    let wins: Vec<u32> = reply.value32().map(|i| i.collect()).unwrap_or_default();

    wins.into_iter().map(|w| describe(&x, w)).collect()
}

/// Title, class, pid and minimized state of one window.
fn describe(x: &X11, w: u32) -> ClientWindow {
    let net_name = x.atom("_NET_WM_NAME");
    let utf8 = x.atom("UTF8_STRING");
    let net_pid = x.atom("_NET_WM_PID");
    let net_state = x.atom("_NET_WM_STATE");
    let hidden = x.atom("_NET_WM_STATE_HIDDEN");

    let title = net_name
        .zip(utf8)
        .and_then(|(prop, ty)| string_property(x, w, prop, ty))
        .or_else(|| string_property(x, w, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
        .unwrap_or_default();
    let class = string_property(x, w, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
        .map(|s| s.replace('\0', " ").trim().to_string())
        .unwrap_or_default();
    let pid = net_pid.and_then(|a| {
        x.conn
            .get_property(false, w, a, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    });
    let minimized = net_state.zip(hidden).is_some_and(|(state, hidden)| {
        x.conn
            .get_property(false, w, state, AtomEnum::ATOM, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|mut v| v.any(|a| a == hidden)))
            .unwrap_or(false)
    });
    ClientWindow {
        id: w,
        title,
        class,
        pid,
        minimized,
    }
}

/// Describe any window by id (managed or not). `None` if it doesn't exist.
pub fn window_info(id: u32) -> Option<ClientWindow> {
    let x = X11::open()?;
    x.conn.get_window_attributes(id).ok()?.reply().ok()?;
    Some(describe(&x, id))
}

/// Enumerate normal toplevel windows from `_NET_CLIENT_LIST`, returning
/// (window, title, wm_class) tuples. Used by the quit-app feature.
pub fn client_list() -> Vec<(u32, String, String)> {
    client_windows()
        .into_iter()
        .map(|w| (w.id, w.title, w.class))
        .collect()
}

/// Client windows bottom-to-top (`_NET_CLIENT_LIST_STACKING`).
pub fn stacking_order() -> Vec<u32> {
    let Some(x) = X11::open() else {
        return Vec::new();
    };
    let Some(atom) = x.atom("_NET_CLIENT_LIST_STACKING") else {
        return Vec::new();
    };
    x.conn
        .get_property(false, x.root, atom, AtomEnum::WINDOW, 0, u32::MAX)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect()))
        .unwrap_or_default()
}

/// A physical monitor (RandR 1.5 monitor object).
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub rect: Rect,
    pub primary: bool,
}

/// All active monitors, primary first. Falls back to the whole root window.
pub fn monitors() -> Vec<Monitor> {
    use x11rb::protocol::randr::ConnectionExt as _;
    let Some(x) = X11::open() else {
        return Vec::new();
    };
    let mut out: Vec<Monitor> = x
        .conn
        .randr_get_monitors(x.root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            r.monitors
                .iter()
                .map(|m| Monitor {
                    name: x
                        .conn
                        .get_atom_name(m.name)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|n| String::from_utf8_lossy(&n.name).to_string())
                        .unwrap_or_default(),
                    rect: Rect {
                        x: m.x as i32,
                        y: m.y as i32,
                        w: m.width as u32,
                        h: m.height as u32,
                    },
                    primary: m.primary,
                })
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty() {
        let screen = &x.conn.setup().roots[0];
        out.push(Monitor {
            name: "Screen".to_string(),
            rect: Rect {
                x: 0,
                y: 0,
                w: screen.width_in_pixels as u32,
                h: screen.height_in_pixels as u32,
            },
            primary: true,
        });
    }
    out.sort_by_key(|m| !m.primary);
    out
}

/// Toggle EWMH "show desktop" mode (minimize / restore every window).
pub fn toggle_showing_desktop() -> Option<()> {
    let x = X11::open()?;
    let atom = x.atom("_NET_SHOWING_DESKTOP")?;
    let current = x.window_property(x.root, atom).unwrap_or(0);
    x.send_client_message(x.root, atom, [u32::from(current == 0), 0, 0, 0, 0]);
    Some(())
}

fn string_property(x: &X11, win: u32, prop: Atom, ty: Atom) -> Option<String> {
    let reply = x
        .conn
        .get_property(false, win, prop, ty, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).to_string())
}

/// Politely close a window via `_NET_CLOSE_WINDOW`.
pub fn close_window(win: u32) -> Option<()> {
    let x = X11::open()?;
    let atom = x.atom("_NET_CLOSE_WINDOW")?;
    x.send_client_message(win, atom, [0, 2, 0, 0, 0]);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::rounded_region;

    #[test]
    fn rounded_region_covers_body_and_trims_corners() {
        let rects = rounded_region(100, 60, 10);
        let rows: Vec<&[u32]> = rects.chunks(4).collect();
        // First corner row is inset, the body spans the full width.
        assert!(rows[0][0] > 0);
        assert_eq!(rows.last().unwrap(), &[0, 10, 100, 40]);
        // Every rectangle stays inside the window.
        for r in rows {
            assert!(r[0] + r[2] <= 100 && r[1] + r[3] <= 60);
        }
    }

    #[test]
    fn rounded_region_without_radius_is_one_rect() {
        assert_eq!(rounded_region(50, 20, 0), vec![0, 0, 50, 20]);
    }
}
