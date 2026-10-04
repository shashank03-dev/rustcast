//! The capture overlay and annotation editor.
//!
//! One GTK window, one `DrawingArea`, everything drawn with cairo: the frozen
//! screen, the dimmed surroundings, the selection, annotations and the toolbars.
//! There is a single full-resolution copy of the screen (the cairo surface the
//! frame was grabbed into); everything else is computed on the fly.
//!
//! All geometry is kept in *image pixels* (the real resolution of the capture)
//! and converted from window coordinates at the edges, so HiDPI and fractional
//! scaling export at full quality and annotations look identical on screen and
//! in the saved file.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::cairo::{self, Context, Filter, Format, ImageSurface, Operator, SurfacePattern};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use image::RgbaImage;

use super::beautify::{self, rounded_rect};
use super::grab::{self, Frame, bgra_to_rgba};
use super::ocr::Layout;
use super::ui::{self, Icon, Palette};
use crate::config::ScreenshotConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// Select a region (or click a window), then annotate.
    Area,
    /// Like `Area`, starting with window picking.
    Window,
    /// Select a region, then copy/save immediately.
    Quick,
    /// Select a region, then read its text.
    Ocr,
    /// Select a region, then show its colour palette.
    Palette,
    /// The whole monitor under the pointer, no UI.
    Fullscreen,
    /// Annotate an existing image.
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Arrow,
    Line,
    Rect,
    Ellipse,
    Pen,
    Marker,
    Text,
    Number,
    Censor,
    Spotlight,
    Picker,
}

const TOOLS: [(Tool, &str, char); 12] = [
    (Tool::Select, "Select & move", 'v'),
    (Tool::Arrow, "Arrow", 'a'),
    (Tool::Line, "Line", 'l'),
    (Tool::Rect, "Rectangle", 'r'),
    (Tool::Ellipse, "Ellipse", 'o'),
    (Tool::Pen, "Pen", 'p'),
    (Tool::Marker, "Highlighter", 'm'),
    (Tool::Text, "Text", 't'),
    (Tool::Number, "Numbered step", 'n'),
    (Tool::Censor, "Censor (pixelate / blur / solid)", 'b'),
    (Tool::Spotlight, "Spotlight", 'h'),
    (Tool::Picker, "Color picker", 'i'),
];

type Rgb = (f64, f64, f64);

const COLORS: [(Rgb, &str); 8] = [
    ((1.0, 0.231, 0.188), "Red"),
    ((1.0, 0.584, 0.0), "Orange"),
    ((1.0, 0.8, 0.0), "Yellow"),
    ((0.204, 0.78, 0.349), "Green"),
    ((0.0, 0.478, 1.0), "Blue"),
    ((0.686, 0.322, 0.871), "Purple"),
    ((0.11, 0.11, 0.118), "Black"),
    ((1.0, 1.0, 1.0), "White"),
];

/// Stroke widths in logical pixels (S / M / L).
const SIZES: [f64; 3] = [2.5, 4.5, 8.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CensorMode {
    Pixelate,
    Blur,
    Solid,
}

impl CensorMode {
    fn next(self) -> Self {
        match self {
            CensorMode::Pixelate => CensorMode::Blur,
            CensorMode::Blur => CensorMode::Solid,
            CensorMode::Solid => CensorMode::Pixelate,
        }
    }
    fn label(self) -> &'static str {
        match self {
            CensorMode::Pixelate => "Pixelate",
            CensorMode::Blur => "Blur",
            CensorMode::Solid => "Solid",
        }
    }
}

type P = (f64, f64);

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct R {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl R {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        R { x, y, w, h }
    }
    fn from_pts(a: P, b: P) -> Self {
        R::new(
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        )
    }
    fn contains(&self, p: P) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }
    fn inflate(&self, d: f64) -> R {
        R::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    /// Clamp into `0..w` × `0..h` and snap to whole pixels.
    fn clamp_to(&self, w: f64, h: f64) -> R {
        let x0 = self.x.clamp(0.0, w).round();
        let y0 = self.y.clamp(0.0, h).round();
        let x1 = self.right().clamp(0.0, w).round();
        let y1 = self.bottom().clamp(0.0, h).round();
        R::new(x0, y0, x1 - x0, y1 - y0)
    }
}

#[derive(Clone)]
enum Shape {
    Arrow(P, P),
    Line(P, P),
    Rect(R),
    Ellipse(R),
    Pen(Vec<P>),
    Marker(Vec<P>),
    Text {
        at: P,
        text: String,
        size: f64,
        extent: P,
    },
    Number {
        at: P,
        n: u32,
    },
    Censor {
        r: R,
        mode: CensorMode,
        blurred: Option<ImageSurface>,
    },
    Spotlight(R),
}

#[derive(Clone)]
struct Ann {
    shape: Shape,
    color: Rgb,
    /// Stroke width in image pixels.
    width: f64,
    fill: bool,
}

impl Ann {
    fn bounds(&self, k: f64) -> R {
        let pts_bounds = |pts: &[P]| {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in pts {
                x0 = x0.min(p.0);
                y0 = y0.min(p.1);
                x1 = x1.max(p.0);
                y1 = y1.max(p.1);
            }
            R::new(x0, y0, x1 - x0, y1 - y0)
        };
        match &self.shape {
            Shape::Arrow(a, b) | Shape::Line(a, b) => R::from_pts(*a, *b),
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Spotlight(r) => *r,
            Shape::Censor { r, .. } => *r,
            Shape::Pen(p) | Shape::Marker(p) => pts_bounds(p),
            Shape::Text { at, extent, .. } => R::new(at.0, at.1, extent.0, extent.1),
            Shape::Number { at, .. } => {
                let r = number_radius(self.width, k);
                R::new(at.0 - r, at.1 - r, 2.0 * r, 2.0 * r)
            }
        }
    }

    fn hit(&self, p: P, k: f64) -> bool {
        let tol = self.width / 2.0 + 6.0 * k;
        let near_seg = |a: P, b: P| dist_to_segment(p, a, b) <= tol;
        match &self.shape {
            Shape::Arrow(a, b) | Shape::Line(a, b) => near_seg(*a, *b),
            Shape::Pen(pts) | Shape::Marker(pts) => {
                let tol = if matches!(self.shape, Shape::Marker(_)) {
                    tol + self.width * 1.5
                } else {
                    tol
                };
                pts.windows(2)
                    .any(|w| dist_to_segment(p, w[0], w[1]) <= tol)
                    || pts.first().is_some_and(|q| dist(p, *q) <= tol)
            }
            Shape::Rect(r) | Shape::Ellipse(r) if !self.fill => {
                r.inflate(tol).contains(p) && !r.inflate(-tol).contains(p)
            }
            _ => self.bounds(k).inflate(4.0 * k).contains(p),
        }
    }

    fn translate(&mut self, dx: f64, dy: f64) {
        let mv = |p: &mut P| {
            p.0 += dx;
            p.1 += dy;
        };
        match &mut self.shape {
            Shape::Arrow(a, b) | Shape::Line(a, b) => {
                mv(a);
                mv(b);
            }
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Spotlight(r) => {
                r.x += dx;
                r.y += dy;
            }
            Shape::Censor { r, blurred, .. } => {
                r.x += dx;
                r.y += dy;
                *blurred = None;
            }
            Shape::Pen(pts) | Shape::Marker(pts) => pts.iter_mut().for_each(mv),
            Shape::Text { at, .. } | Shape::Number { at, .. } => mv(at),
        }
    }

    /// Too small to keep (an accidental click with a drawing tool).
    fn degenerate(&self) -> bool {
        match &self.shape {
            Shape::Arrow(a, b) | Shape::Line(a, b) => dist(*a, *b) < 3.0,
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Spotlight(r) => r.w < 3.0 || r.h < 3.0,
            Shape::Censor { r, .. } => r.w < 3.0 || r.h < 3.0,
            Shape::Pen(p) | Shape::Marker(p) => p.len() < 2,
            Shape::Text { text, .. } => text.trim().is_empty(),
            Shape::Number { .. } => false,
        }
    }
}

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn dist_to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

fn number_radius(width: f64, k: f64) -> f64 {
    11.0 * k + width * 1.2
}

/// Snap `b` to 45° steps around `a` (Shift while drawing lines/arrows).
fn snap45(a: P, b: P) -> P {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let step = std::f64::consts::FRAC_PI_4;
    let ang = (dy.atan2(dx) / step).round() * step;
    (a.0 + len * ang.cos(), a.1 + len * ang.sin())
}

/// Make the rect from `a` to `b` a square (Shift while drawing shapes).
fn square(a: P, b: P) -> P {
    let side = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    (
        a.0 + side * (b.0 - a.0).signum(),
        a.1 + side * (b.1 - a.1).signum(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Btn {
    Tool(Tool),
    Color(usize),
    Size(usize),
    Fill,
    Censor,
    Undo,
    Redo,
    Ocr,
    Palette,
    Pin,
    Beautify,
    Save,
    Copy,
    Close,
}

impl Btn {
    fn tooltip(self, ed: &Editor) -> String {
        match self {
            Btn::Tool(t) => TOOLS
                .iter()
                .find(|(tool, ..)| *tool == t)
                .map(|(_, name, key)| format!("{name}  ({})", key.to_ascii_uppercase()))
                .unwrap_or_default(),
            Btn::Color(i) => format!("{}  ({})", COLORS[i].1, i + 1),
            Btn::Size(i) => ["Thin", "Medium", "Thick"][i].to_string(),
            Btn::Fill => if ed.fill { "Filled" } else { "Outline" }.to_string(),
            Btn::Censor => format!("Censor mode: {}", ed.censor.label()),
            Btn::Undo => "Undo  (Ctrl+Z)".into(),
            Btn::Redo => "Redo  (Ctrl+Shift+Z)".into(),
            Btn::Ocr => "Copy text in selection — OCR  (Ctrl+T)".into(),
            Btn::Palette => "Colour palette of selection  (Ctrl+K)".into(),
            Btn::Pin => "Pin to screen  (Ctrl+P)".into(),
            Btn::Beautify => match ed.beautify {
                None => "Beautify: background & shadow  (Ctrl+B)".into(),
                Some(i) => format!(
                    "Beautify: {}  — click for next style",
                    beautify::STYLES[i].name
                ),
            },
            Btn::Save => "Save  (Ctrl+S · Ctrl+Shift+S: Save As)".into(),
            Btn::Copy => "Copy  (Ctrl+C · Enter)".into(),
            Btn::Close => "Cancel  (Esc)".into(),
        }
    }
}

enum Drag {
    None,
    /// Selecting a region; `moved` turns a click into a drag.
    NewSel {
        start: P,
        moved: bool,
    },
    MoveSel {
        start: P,
        orig: R,
    },
    Resize {
        handle: usize,
        orig: R,
    },
    /// Drawing an annotation; `anchor` is where the drag started.
    Draw {
        ann: Ann,
        anchor: P,
    },
    MoveAnn {
        idx: usize,
        last: P,
    },
}

struct TextEdit {
    at: P,
    text: String,
    size: f64,
    color: Rgb,
    /// Index of the annotation being re-edited.
    replacing: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Copy,
    Save,
    SaveAs,
    Pin,
    Ocr,
    Palette,
}

struct Outcome {
    kind: Kind,
    img: RgbaImage,
}

struct Editor {
    flavor: Flavor,
    cfg: ScreenshotConfig,
    bg: ImageSurface,
    iw: f64,
    ih: f64,
    /// Image pixels per window pixel, and where the image sits in the window.
    s: f64,
    ox: f64,
    oy: f64,
    /// Annotation scale: logical → image pixels.
    k: f64,
    sel: Option<R>,
    windows: Vec<R>,
    snap: bool,
    tool: Tool,
    color: Rgb,
    size: usize,
    fill: bool,
    censor: CensorMode,
    anns: Vec<Ann>,
    undo: Vec<Vec<Ann>>,
    redo: Vec<Vec<Ann>>,
    drag: Drag,
    shift: bool,
    pointer: Option<P>,
    text: Option<TextEdit>,
    selected: Option<usize>,
    counter: u32,
    beautify: Option<usize>,
    buttons: Vec<(R, Btn)>,
    toast: Option<(String, Instant)>,
    cursor: &'static str,
    outcome: Option<Outcome>,
    close: bool,
    pal: Palette,
    /// Forced OCR layout (Text / Code / Table), `None` = automatic.
    ocr_layout: Option<Layout>,
}

const BTN: f64 = 30.0;
const BAR_PAD: f64 = 5.0;
const HANDLE: f64 = 5.0;

impl Editor {
    fn new(flavor: Flavor, cfg: ScreenshotConfig, bg: ImageSurface, k: f64) -> Self {
        let (iw, ih) = (f64::from(bg.width()), f64::from(bg.height()));
        Editor {
            flavor,
            snap: cfg.window_snap || flavor == Flavor::Window,
            cfg,
            bg,
            iw,
            ih,
            s: k,
            ox: 0.0,
            oy: 0.0,
            k,
            sel: None,
            windows: Vec::new(),
            tool: Tool::Arrow,
            color: COLORS[0].0,
            size: 1,
            fill: false,
            censor: CensorMode::Pixelate,
            anns: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            drag: Drag::None,
            shift: false,
            pointer: None,
            text: None,
            selected: None,
            counter: 0,
            beautify: None,
            buttons: Vec::new(),
            toast: None,
            cursor: "crosshair",
            outcome: None,
            close: false,
            pal: Palette::load(),
            ocr_layout: None,
        }
    }

    /// Flavors that finish as soon as a region is chosen.
    fn instant(&self) -> bool {
        matches!(self.flavor, Flavor::Quick | Flavor::Ocr | Flavor::Palette)
    }

    /// What an instant flavor does with its region.
    fn instant_kind(&self) -> Kind {
        match self.flavor {
            Flavor::Ocr => Kind::Ocr,
            Flavor::Palette => Kind::Palette,
            _ if self.cfg.enter_action.eq_ignore_ascii_case("save") => Kind::Save,
            _ => Kind::Copy,
        }
    }

    fn to_img(&self, p: P) -> P {
        ((p.0 - self.ox) * self.s, (p.1 - self.oy) * self.s)
    }

    fn to_win(&self, p: P) -> P {
        (p.0 / self.s + self.ox, p.1 / self.s + self.oy)
    }

    fn rect_to_win(&self, r: R) -> R {
        let (x, y) = self.to_win((r.x, r.y));
        R::new(x, y, r.w / self.s, r.h / self.s)
    }

    fn stroke(&self) -> f64 {
        SIZES[self.size] * self.k
    }

    fn whole(&self) -> R {
        R::new(0.0, 0.0, self.iw, self.ih)
    }

    fn picking(&self) -> bool {
        self.sel.is_none() || matches!(self.drag, Drag::NewSel { .. })
    }

    fn show_toolbars(&self) -> bool {
        self.sel.is_some() && !matches!(self.drag, Drag::NewSel { .. }) && !self.instant()
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn window_at(&self, p: P) -> Option<R> {
        if !self.snap {
            return None;
        }
        self.windows.iter().find(|w| w.contains(p)).copied()
    }

    fn push_undo(&mut self) {
        self.undo.push(self.anns.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn do_undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.anns, prev));
            self.selected = None;
        }
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.anns, next));
            self.selected = None;
        }
    }

    fn pixel(&self, p: P) -> Option<(u8, u8, u8)> {
        let (x, y) = (p.0.floor() as i32, p.1.floor() as i32);
        if x < 0 || y < 0 || x >= self.bg.width() || y >= self.bg.height() {
            return None;
        }
        let mut out = None;
        let stride = self.bg.stride();
        let _ = self.bg.with_data(|d| {
            let i = (y * stride + x * 4) as usize;
            out = Some((d[i + 2], d[i + 1], d[i]));
        });
        out
    }

    // ── Selection handles ────────────────────────────────────────────────

    /// Handle positions (window px): corners and edge midpoints, clockwise from top-left.
    fn handles(&self) -> Vec<P> {
        let Some(sel) = self.sel else {
            return Vec::new();
        };
        let r = self.rect_to_win(sel);
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        vec![
            (x0, y0),
            (mx, y0),
            (x1, y0),
            (x1, my),
            (x1, y1),
            (mx, y1),
            (x0, y1),
            (x0, my),
        ]
    }

    fn handle_at(&self, w: P) -> Option<usize> {
        self.handles()
            .iter()
            .position(|h| (h.0 - w.0).abs() <= HANDLE + 4.0 && (h.1 - w.1).abs() <= HANDLE + 4.0)
    }

    fn resize(&self, handle: usize, orig: R, p: P) -> R {
        let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
        match handle {
            0 => (x0, y0) = p,
            1 => y0 = p.1,
            2 => (x1, y0) = p,
            3 => x1 = p.0,
            4 => (x1, y1) = p,
            5 => y1 = p.1,
            6 => (x0, y1) = p,
            _ => x0 = p.0,
        }
        R::from_pts((x0, y0), (x1, y1)).clamp_to(self.iw, self.ih)
    }

    // ── Input ────────────────────────────────────────────────────────────

    fn press(&mut self, w: P, button: u32) {
        if let Some(btn) = self
            .buttons
            .iter()
            .find(|(r, _)| r.contains(w))
            .map(|(_, b)| *b)
        {
            self.click_button(btn);
            return;
        }
        if button == 3 {
            if !matches!(self.drag, Drag::None) {
                self.drag = Drag::None;
            } else if self.flavor != Flavor::Edit && self.anns.is_empty() && self.text.is_none() {
                self.sel = None;
            }
            return;
        }
        if button != 1 {
            return;
        }
        if self.text.is_some() {
            self.commit_text();
            return;
        }
        let p = self.to_img(w);
        if self.sel.is_none() || self.instant() {
            self.drag = Drag::NewSel {
                start: p,
                moved: false,
            };
            return;
        }
        if let Some(handle) = self.handle_at(w) {
            self.drag = Drag::Resize {
                handle,
                orig: self.sel.unwrap_or_default(),
            };
            return;
        }
        let sel = self.sel.unwrap_or_default();
        let (k, width, color, fill) = (self.k, self.stroke(), self.color, self.fill);
        let new = |shape| Drag::Draw {
            ann: Ann {
                shape,
                color,
                width,
                fill,
            },
            anchor: p,
        };
        match self.tool {
            Tool::Select => {
                if let Some(idx) = self.anns.iter().rposition(|a| a.hit(p, k)) {
                    self.push_undo();
                    self.selected = Some(idx);
                    self.drag = Drag::MoveAnn { idx, last: p };
                } else if sel.contains(p) {
                    self.selected = None;
                    self.drag = Drag::MoveSel {
                        start: p,
                        orig: sel,
                    };
                } else {
                    self.selected = None;
                    self.drag = Drag::NewSel {
                        start: p,
                        moved: false,
                    };
                }
            }
            Tool::Picker => {
                if let Some((r, g, b)) = self.pixel(p) {
                    self.color = (
                        f64::from(r) / 255.0,
                        f64::from(g) / 255.0,
                        f64::from(b) / 255.0,
                    );
                    let hex = format!("#{r:02X}{g:02X}{b:02X}");
                    super::copy_text(&hex);
                    self.toast(format!("Copied {hex}"));
                }
            }
            Tool::Text => {
                let hit = self
                    .anns
                    .iter()
                    .rposition(|a| matches!(a.shape, Shape::Text { .. }) && a.hit(p, k));
                if let Some(idx) = hit
                    && let Shape::Text { at, text, size, .. } = &self.anns[idx].shape
                {
                    self.text = Some(TextEdit {
                        at: *at,
                        text: text.clone(),
                        size: *size,
                        color: self.anns[idx].color,
                        replacing: Some(idx),
                    });
                } else {
                    self.text = Some(TextEdit {
                        at: p,
                        text: String::new(),
                        size: (14.0 + 6.0 * self.size as f64) * k,
                        color,
                        replacing: None,
                    });
                }
            }
            Tool::Number => {
                self.push_undo();
                self.counter += 1;
                self.anns.push(Ann {
                    shape: Shape::Number {
                        at: p,
                        n: self.counter,
                    },
                    color,
                    width,
                    fill,
                });
            }
            Tool::Arrow => self.drag = new(Shape::Arrow(p, p)),
            Tool::Line => self.drag = new(Shape::Line(p, p)),
            Tool::Rect => self.drag = new(Shape::Rect(R::new(p.0, p.1, 0.0, 0.0))),
            Tool::Ellipse => self.drag = new(Shape::Ellipse(R::new(p.0, p.1, 0.0, 0.0))),
            Tool::Pen => self.drag = new(Shape::Pen(vec![p])),
            Tool::Marker => self.drag = new(Shape::Marker(vec![p])),
            Tool::Censor => {
                self.drag = new(Shape::Censor {
                    r: R::new(p.0, p.1, 0.0, 0.0),
                    mode: self.censor,
                    blurred: None,
                })
            }
            Tool::Spotlight => self.drag = new(Shape::Spotlight(R::new(p.0, p.1, 0.0, 0.0))),
        }
    }

    fn motion(&mut self, w: P) {
        self.pointer = Some(w);
        let p = self.to_img(w);
        let shift = self.shift;
        let (iw, ih) = (self.iw, self.ih);
        match &mut self.drag {
            Drag::None => {}
            Drag::NewSel { start, moved } => {
                let start = *start;
                if !*moved && dist(start, p) / self.s > 3.0 {
                    *moved = true;
                }
                if *moved {
                    let end = if shift { square(start, p) } else { p };
                    self.sel = Some(R::from_pts(start, end).clamp_to(iw, ih));
                }
            }
            Drag::MoveSel { start, orig } => {
                let (dx, dy) = (p.0 - start.0, p.1 - start.1);
                let x = (orig.x + dx).clamp(0.0, iw - orig.w).round();
                let y = (orig.y + dy).clamp(0.0, ih - orig.h).round();
                self.sel = Some(R::new(x, y, orig.w, orig.h));
            }
            Drag::Resize { handle, orig } => {
                let (handle, orig) = (*handle, *orig);
                self.sel = Some(self.resize(handle, orig, p));
            }
            Drag::Draw { ann, anchor } => {
                let a = *anchor;
                match &mut ann.shape {
                    Shape::Arrow(_, b) | Shape::Line(_, b) => {
                        *b = if shift { snap45(a, p) } else { p }
                    }
                    Shape::Rect(r)
                    | Shape::Ellipse(r)
                    | Shape::Spotlight(r)
                    | Shape::Censor { r, .. } => {
                        *r = R::from_pts(a, if shift { square(a, p) } else { p })
                    }
                    Shape::Pen(pts) | Shape::Marker(pts) => {
                        if pts.last().is_none_or(|l| dist(*l, p) >= 1.5) {
                            pts.push(p);
                        }
                    }
                    _ => {}
                }
            }
            Drag::MoveAnn { idx, last } => {
                let (dx, dy) = (p.0 - last.0, p.1 - last.1);
                *last = p;
                let idx = *idx;
                if let Some(a) = self.anns.get_mut(idx) {
                    a.translate(dx, dy);
                }
            }
        }
    }

    /// Returns true when the capture should finish right away (quick/OCR).
    fn release(&mut self, w: P) -> bool {
        let p = self.to_img(w);
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::NewSel { moved, .. } => {
                let tiny = self.sel.is_none_or(|s| s.w < 2.0 || s.h < 2.0);
                if !moved || tiny {
                    // A click: the window under the pointer, else the whole screen.
                    let target = self.window_at(p).unwrap_or(self.whole());
                    self.sel = Some(target.clamp_to(self.iw, self.ih));
                }
                self.instant()
            }
            Drag::Draw { mut ann, .. } => {
                if !ann.degenerate() {
                    if let Shape::Censor {
                        r,
                        mode: CensorMode::Blur,
                        blurred,
                    } = &mut ann.shape
                    {
                        *blurred = blur_patch(&self.bg, *r, self.k);
                    }
                    self.push_undo();
                    self.anns.push(ann);
                }
                false
            }
            Drag::MoveAnn { idx, .. } => {
                let k = self.k;
                if let Some(Ann {
                    shape:
                        Shape::Censor {
                            r,
                            mode: CensorMode::Blur,
                            blurred,
                        },
                    ..
                }) = self.anns.get_mut(idx)
                {
                    *blurred = blur_patch(&self.bg, *r, k);
                }
                false
            }
            _ => false,
        }
    }

    fn click_button(&mut self, btn: Btn) {
        match btn {
            Btn::Tool(t) => self.set_tool(t),
            Btn::Color(i) => self.set_color(i),
            Btn::Size(i) => self.size = i,
            Btn::Fill => self.fill = !self.fill,
            Btn::Censor => self.censor = self.censor.next(),
            Btn::Undo => self.do_undo(),
            Btn::Redo => self.do_redo(),
            Btn::Ocr => self.finish(Kind::Ocr),
            Btn::Palette => self.finish(Kind::Palette),
            Btn::Pin => self.finish(Kind::Pin),
            Btn::Beautify => self.cycle_beautify(),
            Btn::Save => self.finish(Kind::Save),
            Btn::Copy => self.finish(Kind::Copy),
            Btn::Close => self.close = true,
        }
    }

    fn set_tool(&mut self, t: Tool) {
        self.commit_text();
        self.tool = t;
        self.selected = None;
    }

    fn set_color(&mut self, i: usize) {
        self.color = COLORS[i].0;
        if let Some(t) = &mut self.text {
            t.color = self.color;
        } else if let Some(a) = self.selected.and_then(|i| self.anns.get(i)).cloned() {
            // Recolour the selected annotation.
            self.push_undo();
            if let Some(idx) = self.selected {
                self.anns[idx] = Ann {
                    color: self.color,
                    ..a
                };
            }
        }
    }

    fn cycle_beautify(&mut self) {
        self.beautify = match self.beautify {
            None => Some(0),
            Some(i) if i + 1 < beautify::STYLES.len() => Some(i + 1),
            Some(_) => None,
        };
        match self.beautify {
            Some(i) => self.toast(format!(
                "Beautify: {} ({}/{})",
                beautify::STYLES[i].name,
                i + 1,
                beautify::STYLES.len()
            )),
            None => self.toast("Beautify off"),
        }
    }

    fn commit_text(&mut self) {
        let Some(edit) = self.text.take() else {
            return;
        };
        let extent = measure_text(&edit.text, edit.size);
        let ann = Ann {
            shape: Shape::Text {
                at: edit.at,
                text: edit.text,
                size: edit.size,
                extent,
            },
            color: edit.color,
            width: self.stroke(),
            fill: false,
        };
        self.push_undo();
        match edit.replacing {
            Some(idx) if ann.degenerate() => {
                self.anns.remove(idx);
            }
            Some(idx) => self.anns[idx] = ann,
            None if !ann.degenerate() => self.anns.push(ann),
            None => {
                self.undo.pop();
            }
        }
    }

    /// Build the output image and close the overlay.
    fn finish(&mut self, kind: Kind) {
        self.commit_text();
        let Some(sel) = self.sel else {
            return;
        };
        // OCR reads the screen itself; annotations would only confuse it.
        let img = if matches!(kind, Kind::Ocr | Kind::Palette) {
            crop_surface(&self.bg, sel)
        } else {
            self.export(sel).map(|img| match self.beautify {
                Some(style) => beautify::apply(&img, style, self.k),
                None => img,
            })
        };
        if let Some(img) = img {
            self.outcome = Some(Outcome { kind, img });
        }
        self.close = true;
    }

    /// Render the selection with all annotations at full resolution.
    fn export(&self, sel: R) -> Option<RgbaImage> {
        let (w, h) = (sel.w as i32, sel.h as i32);
        if w <= 0 || h <= 0 {
            return None;
        }
        let out = ImageSurface::create(Format::ARgb32, w, h).ok()?;
        {
            let cr = Context::new(&out).ok()?;
            cr.translate(-sel.x, -sel.y);
            cr.set_source_surface(&self.bg, 0.0, 0.0).ok()?;
            cr.paint().ok()?;
            self.render_annotations(&cr, sel, false);
        }
        out.flush();
        let mut img = None;
        out.with_data(|d| img = Some(bgra_to_rgba(d, w, h, out.stride())))
            .ok()?;
        img
    }

    fn key(&mut self, ev: &gdk::EventKey) -> bool {
        use gdk::keys::constants as k;
        let key = ev.keyval();
        let ctrl = ev.state().contains(gdk::ModifierType::CONTROL_MASK);
        let shift = ev.state().contains(gdk::ModifierType::SHIFT_MASK);

        if let Some(edit) = &mut self.text {
            match key {
                k::Escape => {
                    self.text = None;
                }
                k::Return | k::KP_Enter if shift => edit.text.push('\n'),
                k::Return | k::KP_Enter => self.commit_text(),
                k::BackSpace => {
                    edit.text.pop();
                }
                _ if ctrl && key.to_lower() == k::v => {
                    let cb = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
                    if let Some(t) = cb.wait_for_text() {
                        edit.text.push_str(&t);
                    }
                }
                _ if !ctrl => {
                    if let Some(c) = key.to_unicode().filter(|c| !c.is_control()) {
                        edit.text.push(c);
                    }
                }
                _ => {}
            }
            return true;
        }

        match key {
            k::Escape => {
                // Esc drops an annotation being drawn; otherwise it cancels.
                match std::mem::replace(&mut self.drag, Drag::None) {
                    Drag::Draw { .. } => {}
                    // Put a dragged annotation back where it was.
                    Drag::MoveAnn { .. } => self.do_undo(),
                    _ => self.close = true,
                }
            }
            k::Return | k::KP_Enter => {
                if self.sel.is_some() {
                    if self.cfg.enter_action.eq_ignore_ascii_case("save") {
                        self.finish(Kind::Save);
                    } else {
                        self.finish(Kind::Copy);
                    }
                }
            }
            k::Delete | k::BackSpace => {
                let idx = self.selected.take().or(self.anns.len().checked_sub(1));
                if let Some(idx) = idx.filter(|i| *i < self.anns.len()) {
                    self.push_undo();
                    self.anns.remove(idx);
                }
            }
            k::Tab => {
                self.snap = !self.snap;
                self.toast(if self.snap {
                    "Window snapping on"
                } else {
                    "Window snapping off"
                });
            }
            k::Left | k::Right | k::Up | k::Down => {
                if let Some(sel) = self.sel {
                    let step = if shift { 10.0 } else { 1.0 };
                    let (dx, dy) = match key {
                        k::Left => (-step, 0.0),
                        k::Right => (step, 0.0),
                        k::Up => (0.0, -step),
                        _ => (0.0, step),
                    };
                    let x = (sel.x + dx).clamp(0.0, self.iw - sel.w);
                    let y = (sel.y + dy).clamp(0.0, self.ih - sel.h);
                    self.sel = Some(R::new(x, y, sel.w, sel.h));
                }
            }
            _ if ctrl => match key.to_lower() {
                k::c => self.finish(Kind::Copy),
                k::s if shift => self.finish(Kind::SaveAs),
                k::s => self.finish(Kind::Save),
                k::z if shift => self.do_redo(),
                k::z => self.do_undo(),
                k::y => self.do_redo(),
                k::t => self.finish(Kind::Ocr),
                k::k => self.finish(Kind::Palette),
                k::p => self.finish(Kind::Pin),
                k::b => self.cycle_beautify(),
                _ => return false,
            },
            _ => {
                let Some(c) = key.to_lower().to_unicode() else {
                    return false;
                };
                if c == 'f' && self.flavor != Flavor::Edit {
                    self.sel = Some(self.whole());
                    if self.instant() {
                        self.finish(self.instant_kind());
                    }
                } else if let Some(d) = c.to_digit(10).filter(|d| (1..=8).contains(d)) {
                    self.set_color(d as usize - 1);
                } else if let Some((t, ..)) = TOOLS.iter().find(|(_, _, key)| *key == c) {
                    self.set_tool(*t);
                } else {
                    return false;
                }
            }
        }
        true
    }

    // ── Drawing ──────────────────────────────────────────────────────────

    fn draw(&mut self, cr: &Context, aw: f64, ah: f64) {
        if self.flavor == Flavor::Edit {
            // Room for the action bar beside the image and the tool bar below it.
            let (mx, my) = (72.0, 64.0);
            self.s = (self.iw / (aw - 2.0 * mx).max(50.0))
                .max(self.ih / (ah - 2.0 * my).max(50.0))
                .max(0.5);
            self.ox = ((aw - self.iw / self.s) / 2.0).round();
            self.oy = ((ah - self.ih / self.s) / 2.0).round();
            cr.set_source_rgb(0.12, 0.12, 0.13);
            let _ = cr.paint();
        }

        // Frozen screen + annotations, in image space.
        let _ = cr.save();
        cr.translate(self.ox, self.oy);
        cr.scale(1.0 / self.s, 1.0 / self.s);
        cr.rectangle(0.0, 0.0, self.iw, self.ih);
        cr.clip();
        let pattern = SurfacePattern::create(&self.bg);
        pattern.set_filter(if (self.s - 1.0).abs() < 1e-6 {
            Filter::Fast
        } else {
            Filter::Good
        });
        let _ = cr.set_source(&pattern);
        let _ = cr.paint();
        if let Some(sel) = self.sel {
            self.render_annotations(cr, sel, true);
        }
        // Dim everything that is not (going to be) captured.
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, self.iw, self.ih);
        let hover_win = if self.picking() && !self.dragging_sel() {
            self.pointer.and_then(|w| self.window_at(self.to_img(w)))
        } else {
            None
        };
        if let Some(sel) = self.sel.filter(|s| s.w > 0.0 && s.h > 0.0) {
            cr.rectangle(sel.x, sel.y, sel.w, sel.h);
        } else if let Some(win) = hover_win {
            cr.rectangle(win.x, win.y, win.w, win.h);
        }
        cr.set_source_rgba(
            0.0,
            0.0,
            0.0,
            if self.flavor == Flavor::Edit {
                0.55
            } else {
                0.42
            },
        );
        let _ = cr.fill();
        cr.set_fill_rule(cairo::FillRule::Winding);
        let _ = cr.restore();

        // Chrome, in window space.
        cr.set_antialias(cairo::Antialias::Best);
        if let Some(sel) = self.sel {
            self.draw_selection(cr, sel);
        } else if let Some(win) = hover_win {
            let r = self.rect_to_win(win);
            cr.rectangle(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0);
            ui::set(cr, self.pal.accent);
            cr.set_line_width(2.5);
            let _ = cr.stroke();
        }

        self.buttons.clear();
        if self.show_toolbars() {
            self.draw_toolbars(cr, aw, ah);
        }

        let loupe = self.cfg.show_magnifier
            && self.flavor != Flavor::Edit
            && (self.picking() || matches!(self.drag, Drag::Resize { .. }));
        if loupe && let Some(w) = self.pointer {
            self.draw_crosshair(cr, w, aw, ah);
            self.draw_loupe(cr, w, aw, ah);
        }

        if self.sel.is_none() && !matches!(self.drag, Drag::NewSel { moved: true, .. }) {
            let hint = match self.flavor {
                Flavor::Ocr => match self.ocr_layout {
                    Some(Layout::Code) => "Select the code to copy  ·  Esc to cancel",
                    Some(Layout::Table) => "Select the table to copy  ·  Esc to cancel",
                    _ => "Select the text to copy  ·  Esc to cancel",
                },
                Flavor::Palette => "Select an area to pick its colours  ·  Esc to cancel",
                Flavor::Window => "Click a window  ·  drag to select an area  ·  Esc to cancel",
                _ if self.windows.is_empty() || !self.snap => {
                    "Drag to select  ·  click for the whole screen  ·  Esc to cancel"
                }
                _ => "Drag to select  ·  click a window  ·  F full screen  ·  Esc to cancel",
            };
            ui::pill(cr, &self.pal, hint, aw / 2.0, 36.0, 14.0, true);
        }

        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_millis(1800) {
                ui::pill(cr, &self.pal, msg, aw / 2.0, 36.0, 14.0, true);
            } else {
                self.toast = None;
            }
        }

        if matches!(self.drag, Drag::None)
            && let Some(w) = self.pointer
            && let Some((_, btn)) = self.buttons.iter().find(|(r, _)| r.contains(w))
        {
            let text = btn.tooltip(self);
            let (bx, by) = (w.0, w.1 - 28.0);
            ui::pill(
                cr,
                &self.pal,
                &text,
                bx.clamp(120.0, aw - 120.0),
                by.max(16.0),
                12.0,
                true,
            );
        }
    }

    fn dragging_sel(&self) -> bool {
        matches!(self.drag, Drag::NewSel { moved: true, .. })
    }

    fn draw_selection(&self, cr: &Context, sel: R) {
        let r = self.rect_to_win(sel);
        cr.rectangle(r.x - 0.5, r.y - 0.5, r.w + 1.0, r.h + 1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let busy = matches!(self.drag, Drag::Draw { .. } | Drag::MoveAnn { .. });
        if !busy && self.flavor != Flavor::Ocr && self.flavor != Flavor::Quick {
            for h in self.handles() {
                cr.arc(h.0, h.1, HANDLE, 0.0, std::f64::consts::TAU);
                cr.set_source_rgb(1.0, 1.0, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.48, 1.0, 1.0);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        }

        let label = format!("{} × {}", sel.w as i64, sel.h as i64);
        let y = if r.y > 30.0 { r.y - 16.0 } else { r.y + 16.0 };
        let (tw, _) = ui::text_size(cr, &self.pal, &label, 12.0, false);
        ui::pill(cr, &self.pal, &label, r.x + tw / 2.0 + 10.0, y, 12.0, false);
    }

    fn draw_crosshair(&self, cr: &Context, w: P, aw: f64, ah: f64) {
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        cr.move_to(0.0, w.1.floor() + 0.5);
        cr.line_to(aw, w.1.floor() + 0.5);
        cr.move_to(w.0.floor() + 0.5, 0.0);
        cr.line_to(w.0.floor() + 0.5, ah);
        let _ = cr.stroke();
    }

    fn draw_loupe(&self, cr: &Context, w: P, aw: f64, ah: f64) {
        const SIZE: f64 = 120.0;
        const ZOOM: f64 = 10.0;
        let p = self.to_img(w);
        let (px, py) = (p.0.floor(), p.1.floor());
        let mut cx = w.0 + 28.0 + SIZE / 2.0;
        let mut cy = w.1 + 28.0 + SIZE / 2.0;
        if cx + SIZE / 2.0 > aw - 8.0 {
            cx = w.0 - 28.0 - SIZE / 2.0;
        }
        if cy + SIZE / 2.0 + 30.0 > ah - 8.0 {
            cy = w.1 - 28.0 - SIZE / 2.0 - 30.0;
        }

        let _ = cr.save();
        cr.arc(cx, cy, SIZE / 2.0, 0.0, std::f64::consts::TAU);
        cr.clip();
        cr.set_source_rgb(0.1, 0.1, 0.1);
        let _ = cr.paint();
        cr.translate(cx, cy);
        cr.scale(ZOOM, ZOOM);
        cr.translate(-px - 0.5, -py - 0.5);
        let pattern = SurfacePattern::create(&self.bg);
        pattern.set_filter(Filter::Nearest);
        let _ = cr.set_source(&pattern);
        let _ = cr.paint();
        let _ = cr.restore();

        // Centre pixel outline + ring.
        cr.rectangle(cx - ZOOM / 2.0, cy - ZOOM / 2.0, ZOOM, ZOOM);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        cr.arc(cx, cy, SIZE / 2.0, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
        cr.set_line_width(2.0);
        let _ = cr.stroke();

        let color = self
            .pixel(p)
            .map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}"))
            .unwrap_or_default();
        let label = format!("{}, {}   {color}", px.max(0.0), py.max(0.0));
        ui::pill(
            cr,
            &self.pal,
            &label,
            cx,
            cy + SIZE / 2.0 + 16.0,
            11.0,
            false,
        );
    }

    fn draw_toolbars(&mut self, cr: &Context, aw: f64, ah: f64) {
        let Some(sel) = self.sel else { return };
        let r = self.rect_to_win(sel);

        // Horizontal tool bar: tools | colours | sizes | contextual option.
        let mut items: Vec<Btn> = TOOLS.iter().map(|(t, ..)| Btn::Tool(*t)).collect();
        let sep1 = items.len();
        items.extend((0..COLORS.len()).map(Btn::Color));
        let sep2 = items.len();
        items.extend((0..SIZES.len()).map(Btn::Size));
        match self.tool {
            Tool::Rect | Tool::Ellipse => items.push(Btn::Fill),
            Tool::Censor => items.push(Btn::Censor),
            _ => {}
        }
        let seps = [sep1, sep2];
        let bar_w = items.len() as f64 * BTN + 2.0 * BAR_PAD + seps.len() as f64 * 9.0;
        let bar_h = BTN + 2.0 * BAR_PAD;
        let mut y = r.bottom() + 10.0;
        if y + bar_h > ah - 6.0 {
            y = r.y - 10.0 - bar_h;
        }
        if y < 6.0 {
            y = r.bottom() - bar_h - 10.0;
        }
        let x = (r.x + r.w / 2.0 - bar_w / 2.0).clamp(6.0, (aw - bar_w - 6.0).max(6.0));
        bar_background(cr, &self.pal, R::new(x, y, bar_w, bar_h));
        let mut bx = x + BAR_PAD;
        for (i, btn) in items.iter().enumerate() {
            if seps.contains(&i) {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
                cr.rectangle(bx + 3.5, y + 9.0, 1.0, bar_h - 18.0);
                let _ = cr.fill();
                bx += 9.0;
            }
            let rect = R::new(bx, y + BAR_PAD, BTN, BTN);
            self.draw_button(cr, rect, *btn);
            self.buttons.push((rect, *btn));
            bx += BTN;
        }

        // Vertical action bar beside the selection.
        let actions = [
            Btn::Undo,
            Btn::Redo,
            Btn::Ocr,
            Btn::Palette,
            Btn::Pin,
            Btn::Beautify,
            Btn::Save,
            Btn::Copy,
            Btn::Close,
        ];
        let col_w = BTN + 2.0 * BAR_PAD;
        let col_h = actions.len() as f64 * BTN + 2.0 * BAR_PAD;
        let mut cx = r.right() + 10.0;
        if cx + col_w > aw - 6.0 {
            cx = r.x - 10.0 - col_w;
        }
        if cx < 6.0 {
            cx = r.right() - col_w - 10.0;
        }
        let cy = (r.bottom() - col_h).clamp(6.0, (ah - col_h - 6.0).max(6.0));
        bar_background(cr, &self.pal, R::new(cx, cy, col_w, col_h));
        for (i, btn) in actions.iter().enumerate() {
            let rect = R::new(cx + BAR_PAD, cy + BAR_PAD + i as f64 * BTN, BTN, BTN);
            self.draw_button(cr, rect, *btn);
            self.buttons.push((rect, *btn));
        }
    }

    fn draw_button(&self, cr: &Context, r: R, btn: Btn) {
        let hovered = self.pointer.is_some_and(|p| r.contains(p));
        let active = match btn {
            Btn::Tool(t) => t == self.tool,
            Btn::Size(i) => i == self.size,
            Btn::Fill => self.fill,
            Btn::Beautify => self.beautify.is_some(),
            _ => false,
        };
        if active || hovered {
            rounded_rect(cr, r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h - 4.0, 6.0);
            if active {
                ui::set(cr, self.pal.accent);
            } else {
                ui::set(cr, self.pal.fill(ui::SECONDARY_FILL));
            }
            let _ = cr.fill();
        }
        let disabled = match btn {
            Btn::Undo => self.undo.is_empty(),
            Btn::Redo => self.redo.is_empty(),
            _ => false,
        };
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        if active {
            cr.set_source_rgb(1.0, 1.0, 1.0);
        } else {
            ui::set(
                cr,
                self.pal
                    .label(if disabled { ui::TERTIARY } else { ui::PRIMARY }),
            );
        }
        cr.set_line_width(1.6);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        draw_icon(cr, btn, cx, cy, self);
    }

    /// Draw all annotations (plus the one being drawn and the text being
    /// typed, when `live`), clipped to the selection.
    fn render_annotations(&self, cr: &Context, sel: R, live: bool) {
        let _ = cr.save();
        cr.rectangle(sel.x, sel.y, sel.w, sel.h);
        cr.clip();
        let replacing = self.text.as_ref().and_then(|t| t.replacing);
        let mut spots: Vec<R> = Vec::new();
        let mut draw = |ann: &Ann| {
            if let Shape::Spotlight(r) = ann.shape {
                spots.push(r);
            } else {
                draw_ann(cr, ann, &self.bg, self.k);
            }
        };
        for (i, ann) in self.anns.iter().enumerate() {
            if Some(i) != replacing {
                draw(ann);
            }
        }
        if live && let Drag::Draw { ann, .. } = &self.drag {
            draw(ann);
        }
        if !spots.is_empty() {
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(sel.x, sel.y, sel.w, sel.h);
            for s in &spots {
                cr.rectangle(s.x, s.y, s.w, s.h);
            }
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
            let _ = cr.fill();
            cr.set_fill_rule(cairo::FillRule::Winding);
        }
        if live && let Some(edit) = &self.text {
            let text = if edit.text.is_empty() {
                " "
            } else {
                &edit.text
            };
            draw_text(cr, edit.at, text, edit.size, edit.color);
            let (w, h) = measure_text(&edit.text, edit.size);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(1.5 * self.k);
            cr.move_to(
                edit.at.0 + w + 2.0 * self.k,
                edit.at.1 + h.max(edit.size) * 0.1,
            );
            cr.line_to(
                edit.at.0 + w + 2.0 * self.k,
                edit.at.1 + h.max(edit.size * 1.2),
            );
            let _ = cr.stroke();
            cr.set_dash(&[4.0 * self.k, 3.0 * self.k], 0.0);
            cr.rectangle(
                edit.at.0 - 4.0 * self.k,
                edit.at.1 - 2.0 * self.k,
                w.max(edit.size) + 10.0 * self.k,
                h.max(edit.size * 1.2) + 4.0 * self.k,
            );
            cr.set_line_width(1.0 * self.k);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        if live && let Some(ann) = self.selected.and_then(|i| self.anns.get(i)) {
            let b = ann.bounds(self.k).inflate(ann.width / 2.0 + 4.0 * self.k);
            cr.set_dash(&[5.0 * self.k, 4.0 * self.k], 0.0);
            ui::set(cr, self.pal.accent);
            cr.set_line_width(1.5 * self.k);
            cr.rectangle(b.x, b.y, b.w, b.h);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        let _ = cr.restore();
    }
}

// ── Annotation rendering ────────────────────────────────────────────────────

fn set_rgb(cr: &Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

fn draw_ann(cr: &Context, ann: &Ann, bg: &ImageSurface, k: f64) {
    let w = ann.width;
    // Text layout leaves a current point behind; never connect to it.
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(w);
    set_rgb(cr, ann.color, 1.0);
    match &ann.shape {
        Shape::Line(a, b) => {
            cr.move_to(a.0, a.1);
            cr.line_to(b.0, b.1);
            let _ = cr.stroke();
        }
        Shape::Arrow(a, b) => {
            let len = dist(*a, *b);
            if len < 1.0 {
                return;
            }
            let head = (w * 3.2 + 9.0 * k).min(len * 0.7);
            let ang = (b.1 - a.1).atan2(b.0 - a.0);
            let spread = 0.45;
            let base = (b.0 - head * 0.8 * ang.cos(), b.1 - head * 0.8 * ang.sin());
            cr.move_to(a.0, a.1);
            cr.line_to(base.0, base.1);
            let _ = cr.stroke();
            cr.move_to(b.0, b.1);
            cr.line_to(
                b.0 - head * (ang - spread).cos(),
                b.1 - head * (ang - spread).sin(),
            );
            cr.line_to(
                b.0 - head * (ang + spread).cos(),
                b.1 - head * (ang + spread).sin(),
            );
            cr.close_path();
            let _ = cr.fill_preserve();
            cr.set_line_width(w * 0.6);
            let _ = cr.stroke();
        }
        Shape::Rect(r) => {
            rounded_rect(cr, r.x, r.y, r.w, r.h, 2.0 * k);
            if ann.fill {
                set_rgb(cr, ann.color, 0.35);
                let _ = cr.fill_preserve();
                set_rgb(cr, ann.color, 1.0);
            }
            let _ = cr.stroke();
        }
        Shape::Ellipse(r) => {
            if r.w < 1.0 || r.h < 1.0 {
                return;
            }
            let _ = cr.save();
            cr.translate(r.x + r.w / 2.0, r.y + r.h / 2.0);
            cr.scale(r.w / 2.0, r.h / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            let _ = cr.restore();
            if ann.fill {
                set_rgb(cr, ann.color, 0.35);
                let _ = cr.fill_preserve();
                set_rgb(cr, ann.color, 1.0);
            }
            let _ = cr.stroke();
        }
        Shape::Pen(pts) => {
            smooth_path(cr, pts);
            let _ = cr.stroke();
        }
        Shape::Marker(pts) => {
            let _ = cr.save();
            cr.set_operator(Operator::Multiply);
            cr.set_line_cap(cairo::LineCap::Square);
            cr.set_line_width(w * 3.0 + 8.0 * k);
            set_rgb(cr, ann.color, 0.45);
            smooth_path(cr, pts);
            let _ = cr.stroke();
            let _ = cr.restore();
        }
        Shape::Text { at, text, size, .. } => draw_text(cr, *at, text, *size, ann.color),
        Shape::Number { at, n } => {
            let r = number_radius(w, k);
            cr.arc(at.0, at.1, r, 0.0, std::f64::consts::TAU);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            cr.set_line_width(1.5 * k);
            let _ = cr.stroke();
            let light = ann.color.0 * 0.3 + ann.color.1 * 0.59 + ann.color.2 * 0.11 > 0.7;
            let layout = pangocairo::functions::create_layout(cr);
            let mut font = gtk::pango::FontDescription::from_string("Sans Bold");
            font.set_absolute_size(r * 1.15 * f64::from(gtk::pango::SCALE));
            layout.set_font_description(Some(&font));
            layout.set_text(&n.to_string());
            let (_, logical) = layout.pixel_extents();
            cr.move_to(
                at.0 - f64::from(logical.width()) / 2.0,
                at.1 - f64::from(logical.height()) / 2.0,
            );
            if light {
                cr.set_source_rgb(0.0, 0.0, 0.0);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            pangocairo::functions::show_layout(cr, &layout);
        }
        Shape::Censor { r, mode, blurred } => match mode {
            CensorMode::Solid => {
                cr.rectangle(r.x, r.y, r.w, r.h);
                let _ = cr.fill();
            }
            CensorMode::Blur if blurred.is_some() => {
                if let Some(patch) = blurred {
                    let _ = cr.set_source_surface(patch, r.x.floor(), r.y.floor());
                    cr.rectangle(r.x, r.y, r.w, r.h);
                    let _ = cr.fill();
                }
            }
            // Pixelate — also the live preview for blur while dragging.
            _ => pixelate(cr, bg, *r, k),
        },
        Shape::Spotlight(_) => {}
    }
}

/// Smooth freehand path through `pts` (quadratic curves via midpoints).
fn smooth_path(cr: &Context, pts: &[P]) {
    let Some(first) = pts.first() else { return };
    cr.move_to(first.0, first.1);
    if pts.len() == 1 {
        cr.line_to(first.0 + 0.01, first.1);
        return;
    }
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (x0, y0) = cr.current_point().unwrap_or(a);
        // Quadratic (a as control) expressed as a cubic.
        cr.curve_to(
            x0 + 2.0 / 3.0 * (a.0 - x0),
            y0 + 2.0 / 3.0 * (a.1 - y0),
            mid.0 + 2.0 / 3.0 * (a.0 - mid.0),
            mid.1 + 2.0 / 3.0 * (a.1 - mid.1),
            mid.0,
            mid.1,
        );
    }
    if let Some(last) = pts.last() {
        cr.line_to(last.0, last.1);
    }
}

fn text_layout(cr: &Context, text: &str, size: f64) -> gtk::pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = gtk::pango::FontDescription::from_string("Sans Bold");
    font.set_absolute_size(size * f64::from(gtk::pango::SCALE));
    layout.set_font_description(Some(&font));
    layout.set_text(text);
    layout
}

fn draw_text(cr: &Context, at: P, text: &str, size: f64, color: Rgb) {
    let layout = text_layout(cr, text, size);
    // A thin contrasting outline keeps text readable on any background.
    let light = color.0 * 0.3 + color.1 * 0.59 + color.2 * 0.11 > 0.6;
    cr.move_to(at.0, at.1);
    pangocairo::functions::layout_path(cr, &layout);
    if light {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.75);
    }
    cr.set_line_width((size / 9.0).max(1.5));
    let _ = cr.stroke();
    cr.move_to(at.0, at.1);
    set_rgb(cr, color, 1.0);
    pangocairo::functions::show_layout(cr, &layout);
}

fn measure_text(text: &str, size: f64) -> P {
    let Ok(surface) = ImageSurface::create(Format::A8, 1, 1) else {
        return (0.0, 0.0);
    };
    let Ok(cr) = Context::new(&surface) else {
        return (0.0, 0.0);
    };
    if text.is_empty() {
        return (0.0, size * 1.2);
    }
    let (w, h) = text_layout(&cr, text, size).pixel_size();
    (f64::from(w), f64::from(h))
}

/// Mosaic: draw the area downscaled, then scaled back up without smoothing.
fn pixelate(cr: &Context, bg: &ImageSurface, r: R, k: f64) {
    if r.w < 1.0 || r.h < 1.0 {
        return;
    }
    let block = (r.w.min(r.h) / 6.0).clamp(7.0 * k, 16.0 * k);
    let (sw, sh) = (
        (r.w / block).ceil().max(1.0) as i32,
        (r.h / block).ceil().max(1.0) as i32,
    );
    let Ok(small) = ImageSurface::create(Format::ARgb32, sw, sh) else {
        return;
    };
    if let Ok(sc) = Context::new(&small) {
        sc.scale(f64::from(sw) / r.w, f64::from(sh) / r.h);
        sc.translate(-r.x, -r.y);
        let pattern = SurfacePattern::create(bg);
        pattern.set_filter(Filter::Good);
        let _ = sc.set_source(&pattern);
        let _ = sc.paint();
    }
    let _ = cr.save();
    cr.rectangle(r.x, r.y, r.w, r.h);
    cr.clip();
    cr.translate(r.x, r.y);
    cr.scale(r.w / f64::from(sw), r.h / f64::from(sh));
    let pattern = SurfacePattern::create(&small);
    pattern.set_filter(Filter::Nearest);
    let _ = cr.set_source(&pattern);
    let _ = cr.paint();
    let _ = cr.restore();
}

/// A heavily blurred copy of `r` (computed once, when the censor is placed).
fn blur_patch(bg: &ImageSurface, r: R, k: f64) -> Option<ImageSurface> {
    let r = r.clamp_to(f64::from(bg.width()), f64::from(bg.height()));
    let (w, h) = (r.w as i32, r.h as i32);
    if w < 1 || h < 1 {
        return None;
    }
    let mut patch = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    {
        let cr = Context::new(&patch).ok()?;
        cr.set_source_surface(bg, -r.x, -r.y).ok()?;
        cr.paint().ok()?;
    }
    let stride = patch.stride();
    {
        let mut data = patch.data().ok()?;
        let radius = ((w.min(h) as f64 / 8.0).clamp(4.0 * k, 14.0 * k)) as i32;
        beautify::box_blur(&mut data, w, h, stride, radius.max(2));
    }
    Some(patch)
}

fn crop_surface(bg: &ImageSurface, r: R) -> Option<RgbaImage> {
    let (x, y, w, h) = (r.x as i32, r.y as i32, r.w as i32, r.h as i32);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut out = None;
    let stride = bg.stride();
    bg.with_data(|d| {
        let start = (y * stride + x * 4) as usize;
        out = Some(bgra_to_rgba(&d[start..], w, h, stride));
    })
    .ok()?;
    out
}

// ── Chrome ──────────────────────────────────────────────────────────────────

fn bar_background(cr: &Context, p: &Palette, r: R) {
    rounded_rect(cr, r.x, r.y, r.w, r.h, 12.0);
    ui::set(cr, p.hud);
    let _ = cr.fill_preserve();
    ui::set(cr, p.rim);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

/// Toolbar icons, drawn as small vector glyphs centred on `(cx, cy)`.
fn draw_icon(cr: &Context, btn: Btn, cx: f64, cy: f64, ed: &Editor) {
    use std::f64::consts::TAU;
    let stroke = || {
        let _ = cr.stroke();
    };
    cr.new_path();
    match btn {
        Btn::Tool(Tool::Select) => {
            cr.move_to(cx - 5.0, cy - 8.0);
            cr.line_to(cx - 5.0, cy + 7.0);
            cr.line_to(cx - 1.5, cy + 3.5);
            cr.line_to(cx + 1.5, cy + 9.0);
            cr.line_to(cx + 3.5, cy + 8.0);
            cr.line_to(cx + 0.8, cy + 2.5);
            cr.line_to(cx + 6.0, cy + 2.5);
            cr.close_path();
            stroke();
        }
        Btn::Tool(Tool::Arrow) => {
            cr.move_to(cx - 7.0, cy + 7.0);
            cr.line_to(cx + 6.0, cy - 6.0);
            stroke();
            cr.move_to(cx + 7.0, cy - 7.0);
            cr.line_to(cx - 0.5, cy - 5.0);
            cr.line_to(cx + 5.0, cy + 0.5);
            cr.close_path();
            let _ = cr.fill();
        }
        Btn::Tool(Tool::Line) => {
            cr.move_to(cx - 7.0, cy + 7.0);
            cr.line_to(cx + 7.0, cy - 7.0);
            stroke();
        }
        Btn::Tool(Tool::Rect) => {
            rounded_rect(cr, cx - 8.0, cy - 6.0, 16.0, 12.0, 2.0);
            stroke();
        }
        Btn::Tool(Tool::Ellipse) => {
            cr.arc(cx, cy, 7.5, 0.0, TAU);
            stroke();
        }
        Btn::Tool(Tool::Pen) => {
            cr.move_to(cx - 8.0, cy + 4.0);
            cr.curve_to(cx - 4.0, cy - 8.0, cx, cy + 10.0, cx + 4.0, cy - 2.0);
            cr.curve_to(cx + 5.0, cy - 5.0, cx + 7.0, cy - 6.0, cx + 8.0, cy - 6.0);
            stroke();
        }
        Btn::Tool(Tool::Marker) => {
            let _ = cr.save();
            cr.set_source_rgba(1.0, 0.85, 0.1, 0.9);
            cr.set_line_width(6.0);
            cr.set_line_cap(cairo::LineCap::Butt);
            cr.move_to(cx - 8.0, cy + 2.0);
            cr.line_to(cx + 8.0, cy + 2.0);
            stroke();
            let _ = cr.restore();
            cr.move_to(cx - 8.0, cy - 5.0);
            cr.line_to(cx + 8.0, cy - 5.0);
            stroke();
        }
        Btn::Tool(Tool::Text) => {
            cr.move_to(cx - 6.5, cy - 7.0);
            cr.line_to(cx + 6.5, cy - 7.0);
            cr.move_to(cx, cy - 7.0);
            cr.line_to(cx, cy + 8.0);
            stroke();
        }
        Btn::Tool(Tool::Number) => {
            cr.arc(cx, cy, 8.0, 0.0, TAU);
            stroke();
            glyph(cr, "1", cx, cy, 10.0);
        }
        Btn::Tool(Tool::Censor) => {
            for i in 0..3 {
                for j in 0..3 {
                    if (i + j) % 2 == 0 {
                        cr.rectangle(
                            cx - 7.5 + i as f64 * 5.0,
                            cy - 7.5 + j as f64 * 5.0,
                            5.0,
                            5.0,
                        );
                    }
                }
            }
            let _ = cr.fill();
            cr.rectangle(cx - 7.5, cy - 7.5, 15.0, 15.0);
            cr.set_line_width(1.0);
            stroke();
        }
        Btn::Tool(Tool::Spotlight) => {
            cr.rectangle(cx - 8.0, cy - 7.0, 16.0, 14.0);
            stroke();
            cr.arc(cx, cy, 3.5, 0.0, TAU);
            let _ = cr.fill();
        }
        Btn::Tool(Tool::Picker) => {
            cr.move_to(cx - 7.0, cy + 7.0);
            cr.line_to(cx + 2.0, cy - 2.0);
            stroke();
            cr.arc(cx + 4.0, cy - 4.0, 3.5, 0.0, TAU);
            let _ = cr.fill();
        }
        Btn::Color(i) => {
            let c = COLORS[i].0;
            cr.arc(cx, cy, 7.5, 0.0, TAU);
            set_rgb(cr, c, 1.0);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
            cr.set_line_width(1.0);
            stroke();
            if c == ed.color {
                cr.arc(cx, cy, 10.5, 0.0, TAU);
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.set_line_width(2.0);
                stroke();
            }
        }
        Btn::Size(i) => {
            cr.arc(cx, cy, [2.0, 3.5, 5.5][i], 0.0, TAU);
            let _ = cr.fill();
        }
        Btn::Fill => {
            rounded_rect(cr, cx - 7.0, cy - 6.0, 14.0, 12.0, 2.0);
            if ed.fill {
                let _ = cr.fill();
            } else {
                stroke();
            }
        }
        Btn::Censor => glyph(cr, &ed.censor.label()[..1], cx, cy, 13.0),
        Btn::Undo => ui::icon(cr, Icon::Undo, cx, cy),
        Btn::Redo => ui::icon(cr, Icon::Redo, cx, cy),
        Btn::Ocr => ui::icon(cr, Icon::Text, cx, cy),
        Btn::Palette => ui::icon(cr, Icon::Palette, cx, cy),
        Btn::Pin => ui::icon(cr, Icon::Pin, cx, cy),
        Btn::Beautify => ui::icon(cr, Icon::Beautify, cx, cy),
        Btn::Save => ui::icon(cr, Icon::Save, cx, cy),
        Btn::Copy => ui::icon(cr, Icon::Copy, cx, cy),
        Btn::Close => ui::icon(cr, Icon::Close, cx, cy),
    }
}

thread_local! {
    static GLYPH_FONT: String = Palette::load().font;
}

fn glyph(cr: &Context, text: &str, cx: f64, cy: f64, size: f64) {
    let layout = pangocairo::functions::create_layout(cr);
    let font = GLYPH_FONT.with(|f| {
        let mut d = gtk::pango::FontDescription::from_string(&format!("{} Bold", f));
        d.set_absolute_size(size * f64::from(gtk::pango::SCALE));
        d
    });
    layout.set_font_description(Some(&font));
    layout.set_text(text);
    let (_, logical) = layout.pixel_extents();
    cr.move_to(
        cx - f64::from(logical.width()) / 2.0,
        cy - f64::from(logical.height()) / 2.0,
    );
    pangocairo::functions::show_layout(cr, &layout);
}

// ── Windows & lifecycle ─────────────────────────────────────────────────────

/// Capture modes: freeze the screen and run the overlay on the monitor under
/// the pointer.
///
/// `window_id` (X11) captures that window: it is raised first and the
/// overlay opens with it already selected, ready to annotate.
pub fn run_capture(
    flavor: Flavor,
    cfg: ScreenshotConfig,
    ocr_layout: Option<Layout>,
    window_id: Option<u32>,
) {
    let wayland = crate::recorder::portal::is_wayland_session();
    if wayland && flavor == Flavor::Window {
        // Wayland hides window positions from apps; the desktop's own picker
        // (via the portal) lets the user choose the window instead.
        let _guard = super::CaptureGuard::begin();
        match grab::portal_interactive() {
            Ok(frame) => {
                drop(_guard);
                if let Some(bg) = frame_surface(frame) {
                    run_editor_window(bg, "Annotate — Window", cfg);
                }
            }
            Err(e) => log::info!("window capture cancelled: {e}"),
        }
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };

    // Bring the requested window up first, then capture its monitor.
    let target = window_id.and_then(|xid| {
        crate::platform::linux::x11::focus_window(xid);
        crate::platform::linux::x11::raise_window(xid);
        std::thread::sleep(Duration::from_millis(450));
        grab::window_rect(xid)
    });
    if window_id.is_some() && target.is_none() {
        super::error_dialog("That window is gone or not visible any more.");
        return;
    }
    let pointer = match target {
        Some((x, y, w, h)) => {
            let sf = display
                .monitor(0)
                .map(|m| m.scale_factor())
                .unwrap_or(1)
                .max(1);
            ((x + w / 2) / sf, (y + h / 2) / sf)
        }
        None => display
            .default_seat()
            .and_then(|s| s.pointer())
            .map(|p| {
                let (_, x, y) = p.position();
                (x, y)
            })
            .unwrap_or((0, 0)),
    };
    let Some(monitor) = display
        .monitor_at_point(pointer.0, pointer.1)
        .or_else(|| display.primary_monitor())
    else {
        return;
    };
    let geo = monitor.geometry();
    let sf = f64::from(monitor.scale_factor().max(1));

    // Bounding box of all monitors (logical) — what the grab covers.
    let (mut bx0, mut by0, mut bx1, mut by1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for i in 0..display.n_monitors() {
        if let Some(m) = display.monitor(i) {
            let g = m.geometry();
            bx0 = bx0.min(g.x());
            by0 = by0.min(g.y());
            bx1 = bx1.max(g.x() + g.width());
            by1 = by1.max(g.y() + g.height());
        }
    }

    // Thumbnails stay hidden for the whole capture session (they float above
    // everything, the overlay included).
    let session = super::CaptureGuard::begin();
    let frame = grab::grab_desktop();
    let frame = match frame {
        Ok(f) => f,
        Err(e) => {
            super::error_dialog(&e);
            return;
        }
    };
    let rx = f64::from(frame.width) / (f64::from(bx1 - bx0) * sf);
    let ry = f64::from(frame.height) / (f64::from(by1 - by0) * sf);
    let crop_x = (f64::from(geo.x() - bx0) * sf * rx).round();
    let crop_y = (f64::from(geo.y() - by0) * sf * ry).round();
    let frame = frame.crop(
        crop_x as i32,
        crop_y as i32,
        (f64::from(geo.width()) * sf * rx).round() as i32,
        (f64::from(geo.height()) * sf * ry).round() as i32,
    );
    let k = f64::from(frame.width) / f64::from(geo.width());

    let to_image = |(x, y, w, h): (i32, i32, i32, i32)| {
        R::new(
            (f64::from(x) - f64::from(bx0) * sf) * rx - crop_x,
            (f64::from(y) - f64::from(by0) * sf) * ry - crop_y,
            f64::from(w) * rx,
            f64::from(h) * ry,
        )
    };
    let windows: Vec<R> = if wayland {
        Vec::new()
    } else {
        grab::visible_windows().into_iter().map(to_image).collect()
    };
    let preselect = target.map(to_image);

    let Some(bg) = frame_surface(frame) else {
        return;
    };

    if flavor == Flavor::Fullscreen {
        let mut ed = Editor::new(flavor, cfg.clone(), bg, k);
        let whole = ed.whole();
        ed.sel = Some(whole);
        let kind = if cfg.enter_action.eq_ignore_ascii_case("save") {
            Kind::Save
        } else {
            Kind::Copy
        };
        let outcome = ed.export(whole).map(|img| Outcome { kind, img });
        drop(ed);
        drop(session);
        if let Some(o) = outcome {
            perform(o, cfg, None);
        }
        return;
    }

    let mut ed = Editor::new(flavor, cfg.clone(), bg, k);
    ed.windows = windows;
    ed.ocr_layout = ocr_layout;
    if let Some(r) = preselect {
        let (iw, ih) = (ed.iw, ed.ih);
        ed.sel = Some(r.clamp_to(iw, ih)).filter(|r| r.w >= 2.0 && r.h >= 2.0);
    }

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("RustCast Capture");
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_keep_above(true);
    window.move_(geo.x(), geo.y());
    window.set_default_size(geo.width(), geo.height());
    window.fullscreen();

    let outcome = run_window(window, ed);
    drop(session);
    if let Some(o) = outcome {
        perform(o, cfg, ocr_layout);
    }
}

/// Edit mode: annotate an existing image in a normal window.
pub fn run_file(path: &Path, cfg: ScreenshotConfig) {
    let img = match image::open(path) {
        Ok(i) => i.into_rgba8(),
        Err(e) => {
            super::error_dialog(&format!("Cannot open {}: {e}", path.display()));
            return;
        }
    };
    let Some(bg) = frame_surface(Frame::from_rgba(img)) else {
        return;
    };
    let title = format!(
        "Annotate — {}",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    );
    run_editor_window(bg, &title, cfg);
}

/// The editor in a normal, resizable window around an image.
fn run_editor_window(bg: ImageSurface, title: &str, cfg: ScreenshotConfig) {
    let mut ed = Editor::new(Flavor::Edit, cfg.clone(), bg, 1.0);
    ed.sel = Some(ed.whole());
    ed.snap = false;

    let (mw, mh) = gdk::Display::default()
        .and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)))
        .map(|m| {
            (
                f64::from(m.workarea().width()),
                f64::from(m.workarea().height()),
            )
        })
        .unwrap_or((1600.0, 900.0));
    let w = (ed.iw + 144.0).clamp(760.0, mw * 0.9);
    let h = (ed.ih + 128.0).clamp(520.0, mh * 0.9);

    ui::install_css();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.style_context().add_class("rustcast");
    let header = gtk::HeaderBar::new();
    header.set_title(Some(title));
    header.set_show_close_button(true);
    header.set_decoration_layout(Some(":close"));
    window.set_titlebar(Some(&header));
    window.set_title(title);
    window.set_default_size(w as i32, h as i32);
    window.set_position(gtk::WindowPosition::Center);

    if let Some(o) = run_window(window, ed) {
        perform(o, cfg, None);
    }
}

fn frame_surface(frame: Frame) -> Option<ImageSurface> {
    let stride = frame.stride();
    ImageSurface::create_for_data(
        frame.data,
        Format::ARgb32,
        frame.width,
        frame.height,
        stride,
    )
    .ok()
}

/// Wire the editor to a window, run GTK until it closes, return the outcome.
fn run_window(window: gtk::Window, ed: Editor) -> Option<Outcome> {
    let ed = Rc::new(RefCell::new(ed));
    let area = gtk::DrawingArea::new();
    area.set_can_focus(true);
    area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK
            | gdk::EventMask::KEY_PRESS_MASK,
    );
    window.add(&area);

    let check_close = {
        let window = window.clone();
        move |ed: &Rc<RefCell<Editor>>| {
            if ed.borrow().close {
                window.hide();
                window.close();
            }
        }
    };

    {
        let ed = ed.clone();
        area.connect_draw(move |a, cr| {
            let (w, h) = (
                f64::from(a.allocated_width()),
                f64::from(a.allocated_height()),
            );
            ed.borrow_mut().draw(cr, w, h);
            glib::Propagation::Stop
        });
    }
    {
        let (ed, check_close) = (ed.clone(), check_close.clone());
        area.connect_button_press_event(move |a, ev| {
            if ev.event_type() == gdk::EventType::ButtonPress {
                let mut e = ed.borrow_mut();
                e.shift = ev.state().contains(gdk::ModifierType::SHIFT_MASK);
                let p = ev.position();
                e.pointer = Some(p);
                e.press(p, ev.button());
            }
            check_close(&ed);
            a.queue_draw();
            glib::Propagation::Stop
        });
    }
    {
        let (ed, check_close) = (ed.clone(), check_close.clone());
        area.connect_button_release_event(move |a, ev| {
            let finish = {
                let mut e = ed.borrow_mut();
                e.shift = ev.state().contains(gdk::ModifierType::SHIFT_MASK);
                e.release(ev.position())
            };
            if finish {
                let mut e = ed.borrow_mut();
                let kind = e.instant_kind();
                e.finish(kind);
            }
            check_close(&ed);
            a.queue_draw();
            glib::Propagation::Stop
        });
    }
    {
        let ed = ed.clone();
        area.connect_motion_notify_event(move |a, ev| {
            let mut e = ed.borrow_mut();
            e.shift = ev.state().contains(gdk::ModifierType::SHIFT_MASK);
            e.motion(ev.position());
            let cursor = e.cursor_for(ev.position());
            if cursor != e.cursor {
                e.cursor = cursor;
                if let Some(win) = a.window() {
                    let c = gdk::Cursor::from_name(&win.display(), cursor);
                    win.set_cursor(c.as_ref());
                }
            }
            a.queue_draw();
            glib::Propagation::Stop
        });
    }
    {
        let ed = ed.clone();
        area.connect_leave_notify_event(move |a, _| {
            ed.borrow_mut().pointer = None;
            a.queue_draw();
            glib::Propagation::Proceed
        });
    }
    {
        let (ed, check_close, area) = (ed.clone(), check_close.clone(), area.clone());
        window.connect_key_press_event(move |_, ev| {
            let handled = ed.borrow_mut().key(ev);
            check_close(&ed);
            area.queue_draw();
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    // Repaint once toasts expire.
    {
        let area = area.clone();
        let ed = ed.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if ed.borrow().toast.is_some() {
                area.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }

    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    window.present();
    area.grab_focus();
    take_focus(&window);
    if let Some(win) = area.window() {
        let c = gdk::Cursor::from_name(&win.display(), "crosshair");
        win.set_cursor(c.as_ref());
    }
    gtk::main();

    ed.borrow_mut().outcome.take()
}

impl Editor {
    fn cursor_for(&self, w: P) -> &'static str {
        if self.buttons.iter().any(|(r, _)| r.contains(w)) {
            return "default";
        }
        match self.drag {
            Drag::MoveSel { .. } | Drag::MoveAnn { .. } => return "grabbing",
            Drag::Draw { .. } | Drag::NewSel { .. } => return "crosshair",
            _ => {}
        }
        if self.sel.is_some() && !self.instant() {
            if let Some(h) = self.handle_at(w) {
                return [
                    "nw-resize",
                    "n-resize",
                    "ne-resize",
                    "e-resize",
                    "se-resize",
                    "s-resize",
                    "sw-resize",
                    "w-resize",
                ][h];
            }
            let p = self.to_img(w);
            match self.tool {
                Tool::Select if self.anns.iter().any(|a| a.hit(p, self.k)) => return "grab",
                Tool::Select if self.sel.is_some_and(|s| s.contains(p)) => return "move",
                Tool::Text => return "text",
                _ => {}
            }
        }
        "crosshair"
    }
}

/// Ask the window manager to focus the overlay. On GNOME a window opened by a
/// background process may otherwise come up without keyboard focus.
fn take_focus(window: &gtk::Window) {
    let window = window.clone();
    glib::timeout_add_local_once(Duration::from_millis(60), move || {
        window.present_with_time(0);
        let me = std::process::id();
        if let Some(w) = crate::platform::linux::x11::client_windows()
            .into_iter()
            .find(|w| w.pid == Some(me))
        {
            crate::platform::linux::x11::focus_window(w.id);
        }
    });
}

/// Carry out what the user chose, after the overlay is gone.
fn perform(outcome: Outcome, cfg: ScreenshotConfig, ocr_layout: Option<Layout>) {
    let Outcome { kind, img } = outcome;
    match kind {
        Kind::Copy => {
            let dir = crate::persist::screenshots_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(super::timestamp_name("rustcast", "png"));
            if super::write_image_to(&img, &path, &cfg) {
                drop(img);
                super::copy_image(&path);
                crate::platform::linux::overlay::show_thumbnail(path.clone());
                if !super::main_instance_running() {
                    super::linger_for_clipboard();
                }
            }
        }
        Kind::Save => match super::write_image(&img, &cfg.save_dir(), &cfg) {
            Some(path) => crate::platform::linux::overlay::show_thumbnail(path),
            None => super::error_dialog(&format!("Could not save to {}", cfg.save_dir().display())),
        },
        Kind::SaveAs => {
            let dialog = gtk::FileChooserNative::new(
                Some("Save Screenshot"),
                None::<&gtk::Window>,
                gtk::FileChooserAction::Save,
                Some("Save"),
                Some("Cancel"),
            );
            dialog.set_do_overwrite_confirmation(true);
            let dir = cfg.save_dir();
            let _ = std::fs::create_dir_all(&dir);
            dialog.set_current_folder(&dir);
            dialog.set_current_name(&super::timestamp_name("rustcast", cfg.extension()));
            if dialog.run() == gtk::ResponseType::Accept
                && let Some(path) = dialog.filename()
            {
                let path = if path.extension().is_none() {
                    path.with_extension(cfg.extension())
                } else {
                    path
                };
                if super::write_image_to(&img, &path, &cfg) {
                    crate::platform::linux::overlay::show_thumbnail(path);
                } else {
                    super::error_dialog(&format!("Could not save {}", path.display()));
                }
            }
        }
        Kind::Pin => {
            let dir = pins_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(super::timestamp_name("pin", "png"));
            if super::write_image_to(&img, &path, &cfg) {
                super::spawn(&["pin", &path.to_string_lossy()]);
            }
        }
        Kind::Ocr => super::ocr_window::run_on_image(img, cfg, ocr_layout),
        Kind::Palette => super::palette::run_on_image(img),
    }
}

fn pins_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rustcast/pins")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_normalise_and_clamp() {
        let r = R::from_pts((10.0, 20.0), (4.0, 2.0));
        assert_eq!(r, R::new(4.0, 2.0, 6.0, 18.0));
        let c = R::new(-5.0, 5.0, 50.0, 10.0).clamp_to(30.0, 12.0);
        assert_eq!(c, R::new(0.0, 5.0, 30.0, 7.0));
    }

    #[test]
    fn shift_constraints() {
        let p = snap45((0.0, 0.0), (10.0, 1.0));
        assert!((p.1).abs() < 1e-9 && (p.0 - 10.05).abs() < 0.1);
        assert_eq!(square((0.0, 0.0), (10.0, -4.0)), (10.0, -10.0));
    }

    #[test]
    fn segment_distance() {
        assert_eq!(dist_to_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 3.0);
        assert_eq!(dist_to_segment((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 5.0);
    }

    fn editor(w: i32, h: i32) -> Editor {
        let bg = ImageSurface::create(Format::ARgb32, w, h).unwrap();
        Editor::new(Flavor::Area, ScreenshotConfig::default(), bg, 1.0)
    }

    #[test]
    fn click_without_drag_selects_window_or_screen() {
        let mut ed = editor(200, 100);
        ed.windows = vec![R::new(10.0, 10.0, 50.0, 40.0)];
        ed.press((20.0, 20.0), 1);
        ed.release((20.0, 20.0));
        assert_eq!(ed.sel, Some(R::new(10.0, 10.0, 50.0, 40.0)));

        let mut ed = editor(200, 100);
        ed.press((150.0, 80.0), 1);
        ed.release((150.0, 80.0));
        assert_eq!(ed.sel, Some(R::new(0.0, 0.0, 200.0, 100.0)));
    }

    #[test]
    fn drawing_and_undo_redo() {
        let mut ed = editor(200, 100);
        ed.sel = Some(ed.whole());
        ed.tool = Tool::Arrow;
        ed.press((10.0, 10.0), 1);
        ed.motion((60.0, 40.0));
        ed.release((60.0, 40.0));
        assert_eq!(ed.anns.len(), 1);
        ed.do_undo();
        assert!(ed.anns.is_empty());
        ed.do_redo();
        assert_eq!(ed.anns.len(), 1);

        // A click with a drawing tool does not leave a dot behind.
        ed.press((90.0, 90.0), 1);
        ed.release((90.0, 90.0));
        assert_eq!(ed.anns.len(), 1);
    }

    #[test]
    fn export_includes_annotations_at_full_size() {
        let mut ed = editor(100, 60);
        ed.sel = Some(R::new(10.0, 10.0, 40.0, 30.0));
        ed.anns.push(Ann {
            shape: Shape::Censor {
                r: R::new(10.0, 10.0, 40.0, 30.0),
                mode: CensorMode::Solid,
                blurred: None,
            },
            color: (1.0, 0.0, 0.0),
            width: 2.0,
            fill: false,
        });
        let img = ed.export(ed.sel.unwrap()).unwrap();
        assert_eq!(img.dimensions(), (40, 30));
        assert_eq!(img.get_pixel(20, 15).0, [255, 0, 0, 255]);
    }

    #[test]
    fn numbered_steps_count_up() {
        let mut ed = editor(100, 100);
        ed.sel = Some(ed.whole());
        ed.tool = Tool::Number;
        ed.press((10.0, 10.0), 1);
        ed.release((10.0, 10.0));
        ed.press((50.0, 50.0), 1);
        ed.release((50.0, 50.0));
        let ns: Vec<u32> = ed
            .anns
            .iter()
            .filter_map(|a| match a.shape {
                Shape::Number { n, .. } => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(ns, vec![1, 2]);
    }
}
