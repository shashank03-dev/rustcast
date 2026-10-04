//! The floating screenshot thumbnail (macshot / macOS style), drawn with cairo
//! in RustCast's own material.
//!
//! Runs in the `--overlay` subprocess (see [`super::overlay`]). The window is
//! an override-redirect popup, so it lands exactly in the bottom-left corner
//! of the monitor under the pointer — above docks and panels (the monitor's
//! work area), on every workspace — no matter what the window manager would
//! do with a normal window. It slides in from the left edge; several captures
//! stack upwards in their own slots.
//!
//! Interaction:
//! - drag the card → drops `text/uri-list` (a `file://` path) + `image/png`
//!   into any app (terminals, browsers, chats, file managers)
//! - click → open it in the annotation editor
//! - hover → Copy · Save · Annotate · Pin · Copy Text · More, and ×
//! - right-click → every action (Copy Path, Copy Code / Table, Colours,
//!   Compare with Previous, Show in Folder, Move to Trash …)
//! - stays while hovered; otherwise dismisses after `thumbnail_seconds`
//!   (0 = stay until closed); hides itself while a new capture is taken so it
//!   never ends up in the next screenshot

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::cairo::{self, Context, Format, ImageSurface};
use gtk::gdk;
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::glib;
use gtk::prelude::*;

use crate::snap::beautify::{box_blur, rounded_rect};
use crate::snap::ui::{self, Icon, Palette};

/// Card size limits; the card takes the image's shape within them. The
/// minimum width fits the hover buttons.
const CARD_MIN_W: f64 = 216.0;
const CARD_MAX_W: f64 = 260.0;
const CARD_MAX_H: f64 = 168.0;
const CARD_MIN_H: f64 = 84.0;
const RADIUS: f64 = 12.0;
/// Room around the card for its shadow.
const SHADOW: f64 = 16.0;
const EDGE: i32 = 18;
const GAP: i32 = 10;
const BTN: f64 = 32.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Copy,
    Save,
    Edit,
    Pin,
    Text,
    More,
    Close,
}

impl Action {
    fn icon(self) -> Icon {
        match self {
            Action::Copy => Icon::Copy,
            Action::Save => Icon::Save,
            Action::Edit => Icon::Edit,
            Action::Pin => Icon::Pin,
            Action::Text => Icon::Text,
            Action::More => Icon::More,
            Action::Close => Icon::Close,
        }
    }
    fn label(self, saved: bool) -> &'static str {
        match self {
            Action::Copy => "Copy",
            Action::Save if saved => "Show in Folder",
            Action::Save => "Save",
            Action::Edit => "Annotate",
            Action::Pin => "Pin to Screen",
            Action::Text => "Copy Text",
            Action::More => "More",
            Action::Close => "Close",
        }
    }
}

const ROW: [Action; 6] = [
    Action::Copy,
    Action::Save,
    Action::Edit,
    Action::Pin,
    Action::Text,
    Action::More,
];

struct Thumb {
    path: PathBuf,
    image: Option<Pixbuf>,
    shadow: Option<ImageSurface>,
    card_w: f64,
    card_h: f64,
    hover: Option<(f64, f64)>,
    pressed_on: Option<Action>,
    feedback: Option<(String, Instant)>,
    dragging: bool,
    /// Already in the user's screenshot folder (Save → Show in Folder).
    saved: bool,
    /// Room for the shadow (0 without a compositor: no transparency).
    margin: f64,
    radius: f64,
}

impl Thumb {
    fn card(&self) -> (f64, f64, f64, f64) {
        (self.margin, self.margin, self.card_w, self.card_h)
    }

    fn buttons(&self) -> Vec<((f64, f64, f64, f64), Action)> {
        let (x, y, w, h) = self.card();
        let n = ROW.len() as f64;
        let gap = 4.0;
        let total = n * BTN + (n - 1.0) * gap;
        let bx = x + (w - total) / 2.0;
        // Below the × even on the shortest card, roughly centred otherwise.
        let by = y + (h / 2.0 - BTN / 2.0 - 6.0).max(34.0);
        let mut out: Vec<_> = ROW
            .iter()
            .enumerate()
            .map(|(i, a)| ((bx + i as f64 * (BTN + gap), by, BTN, BTN), *a))
            .collect();
        out.push(((x + 7.0, y + 7.0, 22.0, 22.0), Action::Close));
        out
    }

    fn action_at(&self, p: (f64, f64)) -> Option<Action> {
        self.buttons()
            .into_iter()
            .find(|((x, y, w, h), _)| p.0 >= *x && p.0 <= x + w && p.1 >= *y && p.1 <= y + h)
            .map(|(_, a)| a)
    }

    fn on_card(&self, p: (f64, f64)) -> bool {
        let (x, y, w, h) = self.card();
        p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h
    }

    fn draw(&mut self, cr: &Context, pal: &Palette) {
        cr.set_operator(cairo::Operator::Source);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = cr.paint();
        cr.set_operator(cairo::Operator::Over);
        let (x, y, w, h) = self.card();

        if self.shadow.is_none() && self.margin > 0.0 {
            self.shadow = shadow_surface(w, h);
        }
        if let Some(s) = &self.shadow {
            let _ = cr.set_source_surface(s, 0.0, 2.0);
            let _ = cr.paint();
        }

        // Card: image on the window material, contained, never upscaled blurry.
        let _ = cr.save();
        rounded_rect(cr, x, y, w, h, self.radius);
        cr.clip();
        ui::set(cr, pal.window);
        let _ = cr.paint();
        if let Some(img) = &self.image {
            let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
            let s = (w / iw).min(h / ih);
            let (dw, dh) = (iw * s, ih * s);
            let _ = cr.save();
            cr.translate(x + (w - dw) / 2.0, y + (h - dh) / 2.0);
            cr.scale(s, s);
            cr.set_source_pixbuf(img, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        }
        let hovered = self.hover.is_some_and(|p| self.on_card(p)) && !self.dragging;
        if hovered {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.42);
            let _ = cr.paint();
        }
        let _ = cr.restore();

        rounded_rect(cr, x + 0.5, y + 0.5, w - 1.0, h - 1.0, self.radius);
        ui::set(cr, pal.rim);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let feedback = self
            .feedback
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_millis(1300))
            .map(|(m, _)| m.clone());
        if let Some(msg) = feedback {
            // Confirmation: a check and what happened.
            rounded_rect(cr, x, y, w, h, self.radius);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.5);
            let _ = cr.save();
            cr.translate(x + w / 2.0, y + h / 2.0 - 12.0);
            cr.scale(1.6, 1.6);
            ui::icon(cr, Icon::Check, 0.0, 0.0);
            let _ = cr.restore();
            let (tw, _) = ui::text_size(cr, pal, &msg, 13.0, true);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            ui::text(
                cr,
                pal,
                &msg,
                x + (w - tw) / 2.0,
                y + h / 2.0 + 8.0,
                13.0,
                true,
            );
            return;
        }
        self.feedback = None;

        if !hovered {
            return;
        }
        let hot = self.hover.and_then(|p| self.action_at(p));
        for ((bx, by, bw, bh), a) in self.buttons() {
            let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
            cr.arc(cx, cy, bw / 2.0, 0.0, std::f64::consts::TAU);
            if hot == Some(a) {
                ui::set(cr, pal.accent);
            } else {
                cr.set_source_rgba(0.12, 0.12, 0.13, 0.82);
            }
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(if a == Action::Close { 1.8 } else { 1.6 });
            let _ = cr.save();
            if a == Action::Close {
                cr.translate(cx, cy);
                cr.scale(0.75, 0.75);
                ui::icon(cr, a.icon(), 0.0, 0.0);
            } else {
                ui::icon(cr, a.icon(), cx, cy);
            }
            let _ = cr.restore();
        }
        // The hovered action's name (or the hint) under the buttons.
        let caption = match hot {
            Some(a) => a.label(self.saved),
            None => "Drag anywhere · click to annotate",
        };
        let (tw, th) = ui::text_size(cr, pal, caption, 11.5, true);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        let row_bottom = self.buttons()[0].0.1 + BTN;
        let ty = (row_bottom + 3.0).min(y + h - th - 3.0);
        ui::text(cr, pal, caption, x + (w - tw) / 2.0, ty, 11.5, true);
    }
}

/// The card's size for an `iw`×`ih` image: the image's own shape, kept
/// between the limits (wide or tall extremes are letterboxed).
fn card_size(iw: f64, ih: f64) -> (f64, f64) {
    let (iw, ih) = (iw.max(1.0), ih.max(1.0));
    let mut w = CARD_MAX_W.min(CARD_MAX_H * iw / ih).max(CARD_MIN_W);
    let h = (w * ih / iw).clamp(CARD_MIN_H, CARD_MAX_H);
    if h >= CARD_MAX_H {
        w = (h * iw / ih).clamp(CARD_MIN_W, CARD_MAX_W);
    }
    (w.round(), h.round())
}

/// A soft drop shadow for a `w`×`h` card, sized for the whole window.
fn shadow_surface(w: f64, h: f64) -> Option<ImageSurface> {
    let (sw, sh) = ((w + 2.0 * SHADOW) as i32, (h + 2.0 * SHADOW) as i32);
    let mut s = ImageSurface::create(Format::ARgb32, sw, sh).ok()?;
    {
        let cr = Context::new(&s).ok()?;
        rounded_rect(&cr, SHADOW, SHADOW + 3.0, w, h, RADIUS);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.38);
        cr.fill().ok()?;
    }
    let stride = s.stride();
    {
        let mut d = s.data().ok()?;
        box_blur(&mut d, sw, sh, stride, 6);
    }
    Some(s)
}

// ── Slots: one per thumbnail on screen, stacked upwards ─────────────────────

fn slots_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("rustcast-thumbs")
}

fn pid_alive(pid: u32) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|c| String::from_utf8_lossy(&c).contains("--overlay"))
        .unwrap_or(false)
}

/// Claim the lowest free slot (a slot whose owner exited is free again).
fn claim_slot() -> usize {
    let dir = slots_dir();
    let _ = std::fs::create_dir_all(&dir);
    for i in 0..32 {
        let f = dir.join(format!("slot-{i}"));
        let taken = std::fs::read_to_string(&f)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .is_some_and(pid_alive);
        if !taken {
            let _ = std::fs::write(&f, std::process::id().to_string());
            return i;
        }
    }
    0
}

fn release_slot(slot: usize) {
    let f = slots_dir().join(format!("slot-{slot}"));
    if std::fs::read_to_string(&f).is_ok_and(|s| s.trim() == std::process::id().to_string()) {
        let _ = std::fs::remove_file(f);
    }
}

/// True while any thumbnail is on screen.
pub fn any_visible() -> bool {
    std::fs::read_dir(slots_dir())
        .map(|d| {
            d.flatten().any(|e| {
                std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .is_some_and(pid_alive)
            })
        })
        .unwrap_or(false)
}

fn is_saved(path: &Path) -> bool {
    let cache = crate::persist::screenshots_dir();
    !path.starts_with(&cache)
}

pub fn run(path: PathBuf) {
    if gtk::init().is_err() {
        eprintln!("rustcast overlay: GTK init failed");
        return;
    }
    let pal = Palette::load();
    let cfg = crate::snap::load_config().screenshot;

    let Some(display) = gdk::Display::default() else {
        return;
    };
    let monitor = display
        .default_seat()
        .and_then(|s| s.pointer())
        .map(|p| p.position())
        .and_then(|(_, x, y)| display.monitor_at_point(x, y))
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0));
    let scale = monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .unwrap_or(1)
        .max(1);

    // Load at the size it is shown (sharp on HiDPI, small in memory).
    let Ok(full) = Pixbuf::from_file(&path) else {
        eprintln!("rustcast overlay: cannot load {}", path.display());
        return;
    };
    let (iw, ih) = (f64::from(full.width()), f64::from(full.height()));
    let (card_w, card_h) = card_size(iw, ih);
    let fit = (card_w / iw).min(card_h / ih).min(1.0) * f64::from(scale);
    let image = if fit < 1.0 {
        full.scale_simple(
            ((iw * fit).round() as i32).max(1),
            ((ih * fit).round() as i32).max(1),
            InterpType::Bilinear,
        )
    } else {
        Some(full)
    };

    // Shadow and rounded corners need a compositor (real transparency).
    let composited = gdk::Screen::default().is_some_and(|s| s.is_composited());
    let margin = if composited { SHADOW } else { 0.0 };
    let radius = if composited { RADIUS } else { 3.0 };

    let slot = claim_slot();
    let (win_w, win_h) = (
        (card_w + 2.0 * margin) as i32,
        (card_h + 2.0 * margin) as i32,
    );
    let area = monitor
        .as_ref()
        .map(|m| m.workarea())
        .unwrap_or_else(|| gdk::Rectangle::new(0, 0, 1280, 800));
    let per_slot = CARD_MAX_H as i32 + GAP;
    let fit_slots = ((area.height() - 2 * EDGE) / per_slot).max(1) as usize;
    let target_x = area.x() + EDGE - margin as i32;
    let target_y = area.y() + area.height()
        - EDGE
        - card_h as i32
        - margin as i32
        - (slot % fit_slots) as i32 * per_slot;

    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.set_default_size(win_w, win_h);
    window.set_size_request(win_w, win_h);
    window.move_(target_x - win_w, target_y);

    let st = Rc::new(RefCell::new(Thumb {
        saved: is_saved(&path),
        path: path.clone(),
        image,
        shadow: None,
        card_h,
        hover: None,
        pressed_on: None,
        feedback: None,
        dragging: false,
        card_w,
        margin,
        radius,
    }));

    let canvas = gtk::DrawingArea::new();
    canvas.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::ENTER_NOTIFY_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    window.add(&canvas);
    {
        let (st, pal) = (st.clone(), pal.clone());
        canvas.connect_draw(move |_, cr| {
            st.borrow_mut().draw(cr, &pal);
            glib::Propagation::Stop
        });
    }

    // Drag source: the file and the PNG bytes.
    let targets = [
        gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::OTHER_APP, 0),
        gtk::TargetEntry::new("image/png", gtk::TargetFlags::OTHER_APP, 1),
    ];
    canvas.drag_source_set(
        gdk::ModifierType::BUTTON1_MASK,
        &targets,
        gdk::DragAction::COPY,
    );
    if let Ok(icon) = Pixbuf::from_file_at_scale(&path, 160, 160, true) {
        canvas.drag_source_set_icon_pixbuf(&icon);
    }
    {
        let path = path.clone();
        canvas.connect_drag_data_get(move |_w, _ctx, sel, info, _time| match info {
            0 => {
                let uri = format!("file://{}", path.display());
                let _ = sel.set_uris(&[&uri]);
            }
            1 => {
                if let Ok(bytes) = std::fs::read(&path) {
                    sel.set(&gdk::Atom::intern("image/png"), 8, &bytes);
                }
            }
            _ => {}
        });
    }
    let drag_failed = Rc::new(Cell::new(false));
    {
        let st = st.clone();
        let failed = drag_failed.clone();
        canvas.connect_drag_begin(move |_, _| {
            failed.set(false);
            let mut s = st.borrow_mut();
            s.dragging = true;
            s.pressed_on = None;
        });
    }
    {
        let failed = drag_failed.clone();
        canvas.connect_drag_failed(move |_, _, _| {
            failed.set(true);
            glib::Propagation::Proceed
        });
    }

    let dismiss = slide_out_fn(&window, target_x, target_y, win_w);
    {
        let (st, dismiss, failed) = (st.clone(), dismiss.clone(), drag_failed.clone());
        canvas.connect_drag_end(move |c, _| {
            st.borrow_mut().dragging = false;
            if failed.get() {
                c.queue_draw();
            } else {
                // Dropped somewhere: done.
                dismiss();
            }
        });
    }

    // Clicks.
    {
        let st = st.clone();
        canvas.connect_button_press_event(move |_, ev| {
            if ev.button() == 1 {
                let mut s = st.borrow_mut();
                let p = ev.position();
                s.pressed_on = s.action_at(p).or(s.on_card(p).then_some(Action::Edit));
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (st, window, dismiss) = (st.clone(), window.clone(), dismiss.clone());
        canvas.connect_button_release_event(move |c, ev| {
            let p = ev.position();
            if ev.button() == 3 {
                let path = st.borrow().path.clone();
                menu(&window, &path, ev, dismiss.clone());
                return glib::Propagation::Stop;
            }
            if ev.button() != 1 {
                return glib::Propagation::Proceed;
            }
            let (pressed, now) = {
                let s = st.borrow();
                (
                    s.pressed_on,
                    s.action_at(p).or(s.on_card(p).then_some(Action::Edit)),
                )
            };
            st.borrow_mut().pressed_on = None;
            if pressed.is_none() || pressed != now {
                return glib::Propagation::Proceed;
            }
            let path = st.borrow().path.clone();
            let saved = st.borrow().saved;
            let feedback = |msg: &str| {
                st.borrow_mut().feedback = Some((msg.to_string(), Instant::now()));
                c.queue_draw();
            };
            match pressed {
                Some(Action::Copy) => {
                    crate::snap::copy_image(&path);
                    feedback("Copied");
                }
                Some(Action::Save) if saved => {
                    show_in_folder(&path);
                }
                Some(Action::Save) => match save_copy(&path) {
                    Some(new) => {
                        {
                            let mut s = st.borrow_mut();
                            s.path = new;
                            s.saved = true;
                        }
                        feedback("Saved");
                    }
                    None => feedback("Could not save"),
                },
                Some(Action::Edit) => {
                    crate::snap::spawn(&["edit", &path.to_string_lossy()]);
                    dismiss();
                }
                Some(Action::Pin) => {
                    crate::snap::spawn(&["pin", &path.to_string_lossy()]);
                    dismiss();
                }
                Some(Action::Text) => {
                    crate::snap::spawn(&["ocr-file", &path.to_string_lossy()]);
                    dismiss();
                }
                Some(Action::More) => menu(&window, &path, ev, dismiss.clone()),
                Some(Action::Close) => dismiss(),
                None => {}
            }
            glib::Propagation::Stop
        });
    }

    // Hover tracking + auto-dismiss.
    let last_hover = Rc::new(Cell::new(Instant::now()));
    {
        let (st, last_hover) = (st.clone(), last_hover.clone());
        canvas.connect_motion_notify_event(move |c, ev| {
            st.borrow_mut().hover = Some(ev.position());
            last_hover.set(Instant::now());
            if let Some(win) = c.window() {
                let over_button = st.borrow().action_at(ev.position()).is_some();
                let name = if over_button { "pointer" } else { "grab" };
                win.set_cursor(gdk::Cursor::from_name(&win.display(), name).as_ref());
            }
            c.queue_draw();
            glib::Propagation::Proceed
        });
    }
    {
        let (st, last_hover) = (st.clone(), last_hover.clone());
        canvas.connect_leave_notify_event(move |c, ev| {
            // Leaving into our own popup menu is an "inferior" crossing.
            if ev.detail() != gdk::NotifyType::Inferior {
                st.borrow_mut().hover = None;
                last_hover.set(Instant::now());
                c.queue_draw();
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (st, dismiss, window, canvas) =
            (st.clone(), dismiss.clone(), window.clone(), canvas.clone());
        let stay = Duration::from_secs(u64::from(cfg.thumbnail_seconds));
        let hidden_for_capture = Cell::new(false);
        let started = Instant::now();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            // Get out of the way while a new screenshot is being taken.
            let capturing = crate::snap::capture_in_progress();
            if capturing != hidden_for_capture.get() {
                hidden_for_capture.set(capturing);
                if capturing {
                    window.hide();
                } else {
                    window.show_all();
                }
            }
            let s = st.borrow();
            let busy = s.hover.is_some()
                || s.dragging
                || MENU_OPEN.with(Cell::get)
                || s.feedback.is_some();
            if s.feedback.is_some() {
                canvas.queue_draw();
            }
            drop(s);
            if busy || capturing {
                last_hover.set(Instant::now());
            }
            let idle = last_hover.get().elapsed().min(started.elapsed());
            if !stay.is_zero() && idle >= stay {
                dismiss();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    window.connect_destroy(move |_| {
        release_slot(slot);
        gtk::main_quit();
    });
    window.show_all();
    slide_in(&window, target_x, target_y, win_w);
    gtk::main();
    release_slot(slot);
}

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn slide_in(window: &gtk::Window, x: i32, y: i32, w: i32) {
    let window = window.clone();
    let start = Instant::now();
    glib::timeout_add_local(Duration::from_millis(12), move || {
        let t = start.elapsed().as_secs_f64() / 0.24;
        let dx = (f64::from(w + 24) * (1.0 - ease_out(t))).round() as i32;
        window.move_(x - dx, y);
        if t >= 1.0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Slide back out to the left, then close (idempotent).
fn slide_out_fn(window: &gtk::Window, x: i32, y: i32, w: i32) -> Rc<dyn Fn()> {
    let window = window.clone();
    let leaving = Rc::new(Cell::new(false));
    Rc::new(move || {
        if leaving.replace(true) {
            return;
        }
        let window = window.clone();
        let start = Instant::now();
        glib::timeout_add_local(Duration::from_millis(12), move || {
            let t = start.elapsed().as_secs_f64() / 0.18;
            let dx = (f64::from(w + 24) * ease_out(t)).round() as i32;
            window.move_(x - dx, y);
            if t >= 1.0 {
                window.close();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    })
}

thread_local! {
    static MENU_OPEN: Cell<bool> = const { Cell::new(false) };
}

fn show_in_folder(path: &Path) {
    // Ask the file manager to select the file; fall back to opening the folder.
    let uri = format!("file://{}", path.display());
    let shown = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("array:string:{uri}"),
            "string:",
        ])
        .status()
        .is_ok_and(|s| s.success());
    if !shown && let Some(dir) = path.parent() {
        crate::snap::open_url(&dir.to_string_lossy());
    }
}

/// Copy a cached capture into the screenshot folder (configured format).
fn save_copy(path: &Path) -> Option<PathBuf> {
    let cfg = crate::snap::load_config().screenshot;
    let img = image::open(path).ok()?.into_rgba8();
    crate::snap::write_image(&img, &cfg.save_dir(), &cfg)
}

/// Move to the desktop trash (recoverable); delete only if there is none.
fn trash(path: &Path) {
    let trashed = std::process::Command::new("gio")
        .arg("trash")
        .arg(path)
        .status()
        .is_ok_and(|s| s.success());
    if !trashed {
        let _ = std::fs::remove_file(path);
    }
}

/// The full actions menu (right-click or "More").
fn menu(window: &gtk::Window, path: &Path, ev: &gdk::EventButton, dismiss: Rc<dyn Fn()>) {
    let menu = ui::menu();
    let p = path.to_string_lossy().to_string();
    let add = |label: &str, f: Box<dyn Fn()>, close: bool| {
        let item = gtk::MenuItem::with_label(label);
        let dismiss = dismiss.clone();
        item.connect_activate(move |_| {
            f();
            if close {
                // Leave a clipboard owner a moment before exiting.
                let dismiss = dismiss.clone();
                glib::timeout_add_local_once(Duration::from_millis(250), move || dismiss());
            }
        });
        menu.append(&item);
    };
    let spawn = |args: Vec<String>| -> Box<dyn Fn()> {
        Box::new(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            crate::snap::spawn(&a);
        })
    };
    let path_buf = path.to_path_buf();
    add(
        "Copy Image",
        {
            let path = path_buf.clone();
            Box::new(move || crate::snap::copy_image(&path))
        },
        false,
    );
    add(
        "Copy Path",
        {
            let p = p.clone();
            Box::new(move || crate::snap::copy_text(&p))
        },
        false,
    );
    menu.append(&gtk::SeparatorMenuItem::new());
    add("Annotate…", spawn(vec!["edit".into(), p.clone()]), true);
    add("Pin to Screen", spawn(vec!["pin".into(), p.clone()]), true);
    menu.append(&gtk::SeparatorMenuItem::new());
    add("Copy Text", spawn(vec!["ocr-file".into(), p.clone()]), true);
    add(
        "Copy Code",
        spawn(vec![
            "ocr-file".into(),
            p.clone(),
            "--as".into(),
            "code".into(),
        ]),
        true,
    );
    add(
        "Copy Table",
        spawn(vec![
            "ocr-file".into(),
            p.clone(),
            "--as".into(),
            "table".into(),
        ]),
        true,
    );
    add(
        "Extract Colours",
        spawn(vec!["palette-file".into(), p.clone()]),
        true,
    );
    add(
        "Compare with Previous",
        spawn(vec!["compare".into(), p.clone()]),
        true,
    );
    menu.append(&gtk::SeparatorMenuItem::new());
    add(
        "Show in Folder",
        {
            let path = path_buf.clone();
            Box::new(move || show_in_folder(&path))
        },
        false,
    );
    add(
        "Move to Trash",
        {
            let path = path_buf.clone();
            Box::new(move || trash(&path))
        },
        true,
    );
    MENU_OPEN.with(|m| m.set(true));
    menu.connect_deactivate(|_| MENU_OPEN.with(|m| m.set(false)));
    menu.set_attach_widget(Some(window));
    menu.show_all();
    menu.popup_at_pointer(Some(ev));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_takes_the_image_shape_within_limits() {
        // 16:9 screen: no letterboxing.
        let (w, h) = card_size(1920.0, 1080.0);
        assert!((w / h - 16.0 / 9.0).abs() < 0.02 && h <= CARD_MAX_H);
        // Very wide: minimum height, max width.
        assert_eq!(card_size(4000.0, 100.0), (CARD_MAX_W, CARD_MIN_H));
        // Very tall: minimum width (buttons fit), maximum height.
        assert_eq!(card_size(100.0, 4000.0), (CARD_MIN_W, CARD_MAX_H));
        // Degenerate sizes never panic.
        let _ = card_size(0.0, 0.0);
    }
}
