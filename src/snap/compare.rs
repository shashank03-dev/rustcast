//! Before / after: compare two screenshots.
//!
//! - **Slider** — the two images on top of each other with a draggable divider.
//! - **Side by side** — next to each other, labelled.
//! - **Differences** — the "after" image dimmed with every changed area marked
//!   and numbered, plus how much changed.
//!
//! The current view can be copied or saved as an image (at full resolution).
//! Without arguments the two most recent captures are compared; either side
//! can be swapped for any image on disk.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::cairo::{Context, Filter, Format, ImageSurface, SurfacePattern};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use image::RgbaImage;

use super::grab::{Frame, bgra_to_rgba};
use super::ui::{self, Palette};

/// Pixels whose channels differ by at most this much count as equal
/// (absorbs JPEG noise and font antialiasing jitter).
const THRESHOLD: u8 = 24;
/// Changes are tracked per CELL × CELL block.
const CELL: u32 = 8;

/// Where two images differ.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
    pub cells: Vec<bool>,
    pub changed_pixels: u64,
    /// Changed areas as `(x, y, w, h)` in pixels, largest first.
    pub regions: Vec<(u32, u32, u32, u32)>,
}

impl Diff {
    pub fn percent(&self) -> f64 {
        let total = u64::from(self.width) * u64::from(self.height);
        if total == 0 {
            0.0
        } else {
            self.changed_pixels as f64 * 100.0 / total as f64
        }
    }
}

/// Compare two images aligned at their top-left corners. Areas covered by
/// only one of them count as changed.
pub fn diff(a: &RgbaImage, b: &RgbaImage) -> Diff {
    let width = a.width().max(b.width());
    let height = a.height().max(b.height());
    let cols = width.div_ceil(CELL);
    let rows = height.div_ceil(CELL);
    let mut cells = vec![false; (cols * rows) as usize];
    let mut changed_pixels = 0u64;
    for y in 0..height {
        for x in 0..width {
            let pa = (x < a.width() && y < a.height()).then(|| a.get_pixel(x, y).0);
            let pb = (x < b.width() && y < b.height()).then(|| b.get_pixel(x, y).0);
            let changed = match (pa, pb) {
                (Some(p), Some(q)) => (0..4).any(|c| p[c].abs_diff(q[c]) > THRESHOLD),
                _ => true,
            };
            if changed {
                changed_pixels += 1;
                cells[((y / CELL) * cols + x / CELL) as usize] = true;
            }
        }
    }
    let regions = regions(&cells, cols, rows, width, height);
    Diff {
        width,
        height,
        cols,
        rows,
        cells,
        changed_pixels,
        regions,
    }
}

/// Group changed cells into boxes; cells up to two apart join the same box.
fn regions(cells: &[bool], cols: u32, rows: u32, w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    let mut seen = vec![false; cells.len()];
    let mut out = Vec::new();
    const REACH: i64 = 2;
    for start in 0..cells.len() {
        if !cells[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (cx, cy) = (i as u32 % cols, i as u32 / cols);
            x0 = x0.min(cx);
            y0 = y0.min(cy);
            x1 = x1.max(cx);
            y1 = y1.max(cy);
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let (nx, ny) = (i64::from(cx) + dx, i64::from(cy) + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(cols) || ny >= i64::from(rows) {
                        continue;
                    }
                    let j = (ny as u32 * cols + nx as u32) as usize;
                    if cells[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let x = x0 * CELL;
        let y = y0 * CELL;
        out.push((
            x,
            y,
            ((x1 + 1) * CELL).min(w) - x,
            ((y1 + 1) * CELL).min(h) - y,
        ));
    }
    out.sort_by_key(|r| std::cmp::Reverse(u64::from(r.2) * u64::from(r.3)));
    out
}

/// The `n` most recent captures (newest first) from RustCast's screenshot
/// folders and the usual desktop screenshot folders.
pub fn recent_captures(n: usize) -> Vec<PathBuf> {
    let cfg = super::load_config().screenshot;
    let mut dirs = vec![crate::persist::screenshots_dir(), cfg.save_dir()];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Pictures/Screenshots"));
        dirs.push(home.join("Pictures"));
    }
    dirs.sort();
    dirs.dedup();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().to_string();
            let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
            if name.starts_with('.') || !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, p))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.dedup_by(|a, b| a.1 == b.1);
    files.into_iter().take(n).map(|(_, p)| p).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Slider,
    SideBySide,
    Differences,
}

struct Side {
    path: PathBuf,
    surf: ImageSurface,
    w: f64,
    h: f64,
}

struct Cmp {
    before: Option<Side>,
    after: Option<Side>,
    diff: Option<Diff>,
    mode: Mode,
    /// Divider position, 0..1 of the width.
    split: f64,
    dragging: bool,
    /// Where the content was last drawn: (x, y, scale, content width).
    placed: (f64, f64, f64, f64),
}

fn load_side(path: &Path) -> Result<(Side, RgbaImage), String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_rgba8();
    let frame = Frame::from_rgba(img.clone());
    let stride = frame.stride();
    let (w, h) = (frame.width, frame.height);
    let surf = ImageSurface::create_for_data(frame.data, Format::ARgb32, w, h, stride)
        .map_err(|e| e.to_string())?;
    Ok((
        Side {
            path: path.to_path_buf(),
            surf,
            w: f64::from(w),
            h: f64::from(h),
        },
        img,
    ))
}

impl Cmp {
    /// Load both sides from disk and recompute the difference.
    fn load(&mut self, before: Option<&Path>, after: Option<&Path>) -> Result<(), String> {
        let a = before.map(load_side).transpose()?;
        let b = after.map(load_side).transpose()?;
        self.diff = match (&a, &b) {
            (Some((_, ia)), Some((_, ib))) => Some(diff(ia, ib)),
            _ => None,
        };
        self.before = a.map(|(s, _)| s);
        self.after = b.map(|(s, _)| s);
        Ok(())
    }

    fn content_size(&self) -> (f64, f64) {
        let (aw, ah) = self
            .before
            .as_ref()
            .map(|s| (s.w, s.h))
            .unwrap_or((0.0, 0.0));
        let (bw, bh) = self
            .after
            .as_ref()
            .map(|s| (s.w, s.h))
            .unwrap_or((0.0, 0.0));
        match self.mode {
            Mode::SideBySide => (aw + bw + gap(aw.max(bw)), ah.max(bh)),
            _ => (aw.max(bw), ah.max(bh)),
        }
    }

    /// Draw the comparison with its top-left at the origin, at `scale`.
    fn paint(&self, cr: &Context, p: &Palette, scale: f64) {
        let (Some(a), Some(b)) = (&self.before, &self.after) else {
            return;
        };
        let image = |surf: &ImageSurface, x: f64| {
            let _ = cr.save();
            cr.translate(x, 0.0);
            cr.scale(scale, scale);
            let pat = SurfacePattern::create(surf);
            pat.set_filter(if (scale - 1.0).abs() < 1e-6 {
                Filter::Fast
            } else {
                Filter::Good
            });
            let _ = cr.set_source(&pat);
            cr.rectangle(0.0, 0.0, f64::from(surf.width()), f64::from(surf.height()));
            let _ = cr.fill();
            let _ = cr.restore();
        };
        let (cw, ch) = self.content_size();
        let (cw, ch) = (cw * scale, ch * scale);
        match self.mode {
            Mode::SideBySide => {
                image(&a.surf, 0.0);
                let x = (a.w + gap(a.w.max(b.w))) * scale;
                image(&b.surf, x);
                corner_label(cr, p, "Before", 0.0);
                corner_label(cr, p, "After", x);
            }
            Mode::Slider => {
                image(&b.surf, 0.0);
                let sx = (cw * self.split).round();
                let _ = cr.save();
                cr.rectangle(0.0, 0.0, sx, ch);
                cr.clip();
                image(&a.surf, 0.0);
                let _ = cr.restore();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                cr.rectangle(sx - 1.0, 0.0, 2.0, ch);
                let _ = cr.fill();
                // Knob.
                let ky = ch / 2.0;
                cr.arc(sx, ky, 15.0, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.2);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
                cr.set_source_rgba(0.1, 0.1, 0.1, 0.85);
                cr.set_line_width(2.0);
                cr.move_to(sx - 4.0, ky - 5.0);
                cr.line_to(sx - 8.0, ky);
                cr.line_to(sx - 4.0, ky + 5.0);
                cr.move_to(sx + 4.0, ky - 5.0);
                cr.line_to(sx + 8.0, ky);
                cr.line_to(sx + 4.0, ky + 5.0);
                let _ = cr.stroke();
                if sx > 80.0 {
                    corner_label(cr, p, "Before", 0.0);
                }
                if cw - sx > 80.0 {
                    let (tw, _) = ui::text_size(cr, p, "After", 11.0, false);
                    corner_label(cr, p, "After", cw - tw - 28.0);
                }
            }
            Mode::Differences => {
                image(&b.surf, 0.0);
                cr.rectangle(0.0, 0.0, cw, ch);
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
                let _ = cr.fill();
                if let Some(d) = &self.diff {
                    let cell = f64::from(CELL) * scale;
                    for (i, changed) in d.cells.iter().enumerate() {
                        if *changed {
                            let (x, y) = (i as u32 % d.cols, i as u32 / d.cols);
                            cr.rectangle(f64::from(x) * cell, f64::from(y) * cell, cell, cell);
                        }
                    }
                    cr.set_source_rgba(1.0, 0.23, 0.19, 0.45);
                    let _ = cr.fill();
                    cr.set_line_width(2.0);
                    for (n, (x, y, w, h)) in d.regions.iter().enumerate().take(99) {
                        let (x, y, w, h) = (
                            f64::from(*x) * scale - 3.0,
                            f64::from(*y) * scale - 3.0,
                            f64::from(*w) * scale + 6.0,
                            f64::from(*h) * scale + 6.0,
                        );
                        super::beautify::rounded_rect(cr, x, y, w, h, 4.0);
                        cr.set_source_rgba(1.0, 0.27, 0.23, 1.0);
                        let _ = cr.stroke();
                        let label = (n + 1).to_string();
                        let (tw, th) = ui::text_size(cr, p, &label, 11.0, true);
                        let (bw, bh) = (tw.max(th) + 8.0, th + 4.0);
                        super::beautify::rounded_rect(
                            cr,
                            x,
                            (y - bh - 2.0).max(0.0),
                            bw,
                            bh,
                            bh / 2.0,
                        );
                        let _ = cr.fill();
                        cr.set_source_rgb(1.0, 1.0, 1.0);
                        ui::text(
                            cr,
                            p,
                            &label,
                            x + (bw - tw) / 2.0,
                            (y - bh - 2.0).max(0.0) + 2.0,
                            11.0,
                            true,
                        );
                    }
                }
            }
        }
    }

    fn summary(&self) -> String {
        match (&self.before, &self.after, &self.diff) {
            (Some(_), Some(_), Some(d)) if d.changed_pixels == 0 => "Identical".to_string(),
            (Some(a), Some(b), Some(d)) => {
                let size = if (a.w, a.h) != (b.w, b.h) {
                    format!(" · sizes differ ({}×{} → {}×{})", a.w, a.h, b.w, b.h)
                } else {
                    String::new()
                };
                let n = d.regions.len();
                format!(
                    "{n} changed area{} · {:.1}% of pixels{size}",
                    if n == 1 { "" } else { "s" },
                    d.percent()
                )
            }
            _ => "Choose two images to compare".to_string(),
        }
    }

    /// Render the current view at full resolution.
    fn export(&self, p: &Palette) -> Option<RgbaImage> {
        let (w, h) = self.content_size();
        let (w, h) = (w.ceil() as i32, h.ceil() as i32);
        if w <= 0 || h <= 0 {
            return None;
        }
        let surf = ImageSurface::create(Format::ARgb32, w, h).ok()?;
        {
            let cr = Context::new(&surf).ok()?;
            cr.set_source_rgb(p.window.0, p.window.1, p.window.2);
            let _ = cr.paint();
            self.paint(&cr, p, 1.0);
        }
        surf.flush();
        let mut out = None;
        surf.with_data(|d| out = Some(bgra_to_rgba(d, w, h, surf.stride())))
            .ok()?;
        out
    }
}

/// A small label pill in the top-left corner of an image drawn at `x`.
fn corner_label(cr: &Context, p: &Palette, text: &str, x: f64) {
    let (tw, _) = ui::text_size(cr, p, text, 11.0, false);
    ui::pill(cr, p, text, x + 10.0 + tw / 2.0 + 8.0, 18.0, 11.0, false);
}

fn gap(w: f64) -> f64 {
    (w * 0.03).clamp(16.0, 48.0)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Open the comparison window. `before` / `after` default to the two most
/// recent captures. Blocks until the window is closed.
pub fn run(before: Option<PathBuf>, after: Option<PathBuf>) {
    let (before, after) = match (before, after) {
        (Some(a), Some(b)) => (Some(a), Some(b)),
        (one, None) | (None, one) => {
            let recent = recent_captures(12);
            let after = one.or_else(|| recent.first().cloned());
            // "Previous" = the newest other capture that really differs (the
            // same shot is often in both the cache and the screenshot folder).
            let after_bytes = after.as_ref().and_then(|a| std::fs::read(a).ok());
            let before = recent.into_iter().find(|r| {
                Some(r) != after.as_ref() && std::fs::read(r).ok().as_ref() != after_bytes.as_ref()
            });
            (before, after)
        }
    };

    let p = Palette::load();
    let (window, header) = ui::panel_window("Compare", None);
    window.set_default_size(980, 680);

    let st = Rc::new(RefCell::new(Cmp {
        before: None,
        after: None,
        diff: None,
        mode: Mode::Slider,
        split: 0.5,
        dragging: false,
        placed: (0.0, 0.0, 1.0, 0.0),
    }));
    let load_error = st
        .borrow_mut()
        .load(before.as_deref(), after.as_deref())
        .err();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(4);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    window.add(&root);

    let canvas = gtk::DrawingArea::new();
    canvas.set_size_request(640, 380);
    canvas.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK,
    );
    let summary = gtk::Label::new(None);
    summary.style_context().add_class("dim");
    summary.set_xalign(1.0);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let refresh = {
        let (st, canvas, summary, header) =
            (st.clone(), canvas.clone(), summary.clone(), header.clone());
        move || {
            let s = st.borrow();
            summary.set_text(&s.summary());
            header.set_subtitle(Some(&match (&s.before, &s.after) {
                (Some(a), Some(b)) => {
                    format!("{}  →  {}", file_name(&a.path), file_name(&b.path))
                }
                _ => "Pick a before and an after image".to_string(),
            }));
            canvas.queue_draw();
        }
    };

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let (seg, _) = {
        let (st, refresh) = (st.clone(), refresh.clone());
        ui::segmented(&["Slider", "Side by Side", "Differences"], 0, move |i| {
            st.borrow_mut().mode = [Mode::Slider, Mode::SideBySide, Mode::Differences][i];
            refresh();
        })
    };
    top.pack_start(&seg, false, false, 0);
    top.pack_end(&summary, true, true, 0);
    root.pack_start(&top, false, false, 0);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.style_context().add_class("card");
    card.pack_start(&canvas, true, true, 0);
    root.pack_start(&card, true, true, 0);

    {
        let (st, p) = (st.clone(), p.clone());
        canvas.connect_draw(move |a, cr| {
            let (aw, ah) = (
                f64::from(a.allocated_width()),
                f64::from(a.allocated_height()),
            );
            let mut s = st.borrow_mut();
            let (cw, ch) = s.content_size();
            if cw <= 0.0 || ch <= 0.0 {
                ui::set(cr, p.label(ui::SECONDARY));
                let msg = "Choose a Before and an After image below";
                let (tw, th) = ui::text_size(cr, &p, msg, 14.0, false);
                ui::text(cr, &p, msg, (aw - tw) / 2.0, (ah - th) / 2.0, 14.0, false);
                return glib::Propagation::Stop;
            }
            let m = 16.0;
            let scale = ((aw - 2.0 * m) / cw).min((ah - 2.0 * m) / ch).min(1.0);
            let (x, y) = (
                ((aw - cw * scale) / 2.0).round(),
                ((ah - ch * scale) / 2.0).round(),
            );
            s.placed = (x, y, scale, cw * scale);
            cr.translate(x, y);
            s.paint(cr, &p, scale);
            glib::Propagation::Stop
        });
    }
    let set_split = {
        let st = st.clone();
        move |x: f64| {
            let mut s = st.borrow_mut();
            let (px, _, _, w) = s.placed;
            if w > 0.0 {
                s.split = ((x - px) / w).clamp(0.0, 1.0);
            }
        }
    };
    {
        let (st, set_split) = (st.clone(), set_split.clone());
        canvas.connect_button_press_event(move |a, ev| {
            if ev.button() == 1 && st.borrow().mode == Mode::Slider {
                st.borrow_mut().dragging = true;
                set_split(ev.position().0);
                a.queue_draw();
            }
            glib::Propagation::Stop
        });
    }
    {
        let st = st.clone();
        canvas.connect_button_release_event(move |_, _| {
            st.borrow_mut().dragging = false;
            glib::Propagation::Stop
        });
    }
    {
        let st = st.clone();
        canvas.connect_motion_notify_event(move |a, ev| {
            let dragging = st.borrow().dragging;
            if dragging {
                set_split(ev.position().0);
                a.queue_draw();
            }
            if let Some(win) = a.window() {
                let name = if st.borrow().mode == Mode::Slider {
                    "col-resize"
                } else {
                    "default"
                };
                win.set_cursor(gdk::Cursor::from_name(&win.display(), name).as_ref());
            }
            glib::Propagation::Stop
        });
    }

    // Bottom: choose / swap · copy / save.
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let choose_before = ui::button("Before…", false);
    let choose_after = ui::button("After…", false);
    let swap = ui::button("Swap", false);
    let save = ui::button("Save…", false);
    let copy = ui::button("Copy Image", true);
    bottom.pack_start(&choose_before, false, false, 0);
    bottom.pack_start(&choose_after, false, false, 0);
    bottom.pack_start(&swap, false, false, 0);
    bottom.pack_end(&copy, false, false, 0);
    bottom.pack_end(&save, false, false, 0);
    root.pack_start(&bottom, false, false, 0);

    let pick = {
        let window = window.clone();
        move |title: &str, current: Option<PathBuf>| -> Option<PathBuf> {
            let dialog = gtk::FileChooserNative::new(
                Some(title),
                Some(&window),
                gtk::FileChooserAction::Open,
                Some("Choose"),
                Some("Cancel"),
            );
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for m in ["image/png", "image/jpeg", "image/webp"] {
                filter.add_mime_type(m);
            }
            dialog.add_filter(filter);
            if let Some(dir) = current.as_ref().and_then(|c| c.parent()) {
                dialog.set_current_folder(dir);
            }
            (dialog.run() == gtk::ResponseType::Accept)
                .then(|| dialog.filename())
                .flatten()
        }
    };
    let reload = {
        let (st, refresh, header) = (st.clone(), refresh.clone(), header.clone());
        move |before: Option<PathBuf>, after: Option<PathBuf>| {
            let result = st.borrow_mut().load(before.as_deref(), after.as_deref());
            refresh();
            if let Err(e) = result {
                header.set_subtitle(Some(&format!("Could not open {e}")));
            }
        }
    };
    let paths = {
        let st = st.clone();
        move || {
            let s = st.borrow();
            (
                s.before.as_ref().map(|x| x.path.clone()),
                s.after.as_ref().map(|x| x.path.clone()),
            )
        }
    };
    {
        let (pick, reload, paths) = (pick.clone(), reload.clone(), paths.clone());
        choose_before.connect_clicked(move |_| {
            let (b, a) = paths();
            if let Some(new) = pick("Choose the Before Image", b.clone().or(a.clone())) {
                reload(Some(new), a);
            }
        });
    }
    {
        let (pick, reload, paths) = (pick.clone(), reload.clone(), paths.clone());
        choose_after.connect_clicked(move |_| {
            let (b, a) = paths();
            if let Some(new) = pick("Choose the After Image", a.clone().or(b.clone())) {
                reload(b, Some(new));
            }
        });
    }
    {
        let (st, refresh) = (st.clone(), refresh.clone());
        swap.connect_clicked(move |_| {
            {
                let mut s = st.borrow_mut();
                let s = &mut *s;
                std::mem::swap(&mut s.before, &mut s.after);
            }
            refresh();
        });
    }
    {
        let (st, header, p) = (st.clone(), header.clone(), p.clone());
        copy.connect_clicked(move |_| {
            let Some(img) = st.borrow().export(&p) else {
                return;
            };
            let cfg = super::load_config().screenshot;
            let dir = crate::persist::screenshots_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(super::timestamp_name("compare", "png"));
            if super::write_image_to(&img, &path, &cfg) {
                super::copy_image(&path);
                header.set_subtitle(Some("Copied comparison image"));
            }
        });
    }
    {
        let (st, header, p, window) = (st.clone(), header.clone(), p.clone(), window.clone());
        save.connect_clicked(move |_| {
            let Some(img) = st.borrow().export(&p) else {
                return;
            };
            let cfg = super::load_config().screenshot;
            let dialog = gtk::FileChooserNative::new(
                Some("Save Comparison"),
                Some(&window),
                gtk::FileChooserAction::Save,
                Some("Save"),
                Some("Cancel"),
            );
            dialog.set_do_overwrite_confirmation(true);
            let dir = cfg.save_dir();
            let _ = std::fs::create_dir_all(&dir);
            dialog.set_current_folder(&dir);
            dialog.set_current_name(&super::timestamp_name("compare", "png"));
            if dialog.run() == gtk::ResponseType::Accept
                && let Some(path) = dialog.filename()
            {
                let path = if path.extension().is_none() {
                    path.with_extension("png")
                } else {
                    path
                };
                if super::write_image_to(&img, &path, &cfg) {
                    header.set_subtitle(Some(&format!("Saved {}", file_name(&path))));
                }
            }
        });
    }

    refresh();
    if let Some(e) = load_error {
        header.set_subtitle(Some(&format!("Could not open {e}")));
    }
    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    window.present();
    gtk::main();
    if !super::main_instance_running() {
        super::linger_for_clipboard();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_no_regions() {
        let a = RgbaImage::from_pixel(40, 30, image::Rgba([10, 10, 10, 255]));
        let d = diff(&a, &a);
        assert_eq!(d.changed_pixels, 0);
        assert!(d.regions.is_empty());
        assert_eq!(d.percent(), 0.0);
    }

    #[test]
    fn small_noise_is_ignored_and_real_changes_are_boxed() {
        let a = RgbaImage::from_pixel(100, 100, image::Rgba([200, 200, 200, 255]));
        let mut b = a.clone();
        // JPEG-like noise everywhere.
        for p in b.pixels_mut() {
            p.0[0] = 210;
        }
        // Two separate changes.
        for y in 10..20 {
            for x in 10..20 {
                b.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        for y in 70..80 {
            for x in 60..90 {
                b.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let d = diff(&a, &b);
        assert_eq!(d.changed_pixels, 100 + 300);
        assert_eq!(d.regions.len(), 2);
        // Largest first, cell-aligned, covering the change.
        let (x, y, w, h) = d.regions[0];
        assert!(x <= 60 && y <= 70 && x + w >= 90 && y + h >= 80);
    }

    #[test]
    fn different_sizes_count_the_extra_area() {
        let a = RgbaImage::from_pixel(10, 10, image::Rgba([1, 1, 1, 255]));
        let b = RgbaImage::from_pixel(20, 10, image::Rgba([1, 1, 1, 255]));
        let d = diff(&a, &b);
        assert_eq!((d.width, d.height), (20, 10));
        assert_eq!(d.changed_pixels, 100);
        assert_eq!(d.regions, vec![(8, 0, 12, 10)]);
    }
}
