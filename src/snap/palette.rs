//! Colour palette: the dominant colours of a capture, ready to copy as HEX,
//! RGB or HSL, one by one or all at once (plain list or CSS variables).
//!
//! Extraction is a median cut over at most ~40 000 sampled pixels, so even a
//! 4K region costs a few milliseconds and no extra memory worth mentioning.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use image::RgbaImage;

use super::beautify::rounded_rect;
use super::ui;

const MAX_SAMPLES: u64 = 40_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swatch {
    pub rgb: [u8; 3],
    /// Fraction of the image (0..1).
    pub share: f64,
}

impl Swatch {
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.rgb[0], self.rgb[1], self.rgb[2])
    }
    pub fn rgb_css(&self) -> String {
        format!("rgb({}, {}, {})", self.rgb[0], self.rgb[1], self.rgb[2])
    }
    pub fn hsl_css(&self) -> String {
        let (h, s, l) = hsl(self.rgb);
        format!("hsl({h:.0}, {s:.0}%, {l:.0}%)")
    }
    fn luma(&self) -> f64 {
        (0.2126 * f64::from(self.rgb[0])
            + 0.7152 * f64::from(self.rgb[1])
            + 0.0722 * f64::from(self.rgb[2]))
            / 255.0
    }
}

pub fn hsl([r, g, b]: [u8; 3]) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l * 100.0);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s * 100.0, l * 100.0)
}

/// The `max` most prominent colours of `img`, most common first.
pub fn extract(img: &RgbaImage, max: usize) -> Vec<Swatch> {
    let (w, h) = img.dimensions();
    let total = u64::from(w) * u64::from(h);
    if total == 0 || max == 0 {
        return Vec::new();
    }
    let step = ((total as f64 / MAX_SAMPLES as f64).sqrt().ceil() as u32).max(1);
    let mut px: Vec<[u8; 3]> = Vec::new();
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let p = img.get_pixel(x, y).0;
            if p[3] >= 128 {
                px.push([p[0], p[1], p[2]]);
            }
        }
    }
    let n = px.len() as f64;
    if px.is_empty() {
        return Vec::new();
    }

    // Median cut: split the box with the widest weighted range until there
    // are enough boxes, then average each box.
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![px];
    while boxes.len() < max * 3 {
        let Some((i, ch)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let (ch, range) = widest(b);
                (i, ch, f64::from(range) * (b.len() as f64).sqrt())
            })
            .filter(|(_, _, score)| *score > 0.0)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(i, ch, _)| (i, ch))
        else {
            break;
        };
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|p| p[ch]);
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }

    let mut swatches: Vec<Swatch> = boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let mut sum = [0u64; 3];
            for p in b {
                for c in 0..3 {
                    sum[c] += u64::from(p[c]);
                }
            }
            let len = b.len() as u64;
            Swatch {
                rgb: std::array::from_fn(|c| ((sum[c] + len / 2) / len) as u8),
                share: b.len() as f64 / n,
            }
        })
        .collect();
    swatches.sort_by(|a, b| b.share.total_cmp(&a.share));

    // Merge colours that look the same.
    let mut out: Vec<Swatch> = Vec::new();
    for s in swatches {
        if let Some(m) = out.iter_mut().find(|o| distance(o.rgb, s.rgb) < 24.0) {
            m.share += s.share;
        } else {
            out.push(s);
        }
    }
    out.sort_by(|a, b| b.share.total_cmp(&a.share));
    out.truncate(max);
    out
}

fn widest(b: &[[u8; 3]]) -> (usize, u8) {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for p in b {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    (0..3)
        .map(|c| (c, hi[c] - lo[c]))
        .max_by_key(|(_, r)| *r)
        .unwrap_or((0, 0))
}

fn distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    let d = |i: usize| f64::from(a[i]) - f64::from(b[i]);
    // Rough perceptual weighting (green matters most).
    (2.0 * d(0) * d(0) + 4.0 * d(1) * d(1) + 3.0 * d(2) * d(2)).sqrt() / 3.0
}

pub fn run_file(path: &Path) {
    match image::open(path) {
        Ok(img) => run_on_image(img.into_rgba8()),
        Err(e) => super::error_dialog(&format!("Cannot open {}: {e}", path.display())),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Hex,
    Rgb,
    Hsl,
}

fn format(s: &Swatch, f: Format) -> String {
    match f {
        Format::Hex => s.hex(),
        Format::Rgb => s.rgb_css(),
        Format::Hsl => s.hsl_css(),
    }
}

/// Show the palette of `img`. Blocks until the window is closed.
pub fn run_on_image(img: RgbaImage) {
    let swatches = extract(&img, 8);
    drop(img);

    let (window, header) = ui::panel_window(
        "Colours",
        Some(if swatches.is_empty() {
            "No colours found"
        } else {
            "Click a colour to copy it"
        }),
    );
    window.set_default_size(560, -1);
    let p = ui::Palette::load();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(4);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    window.add(&root);

    let fmt = Rc::new(Cell::new(Format::Hex));

    // Proportion strip.
    let strip = gtk::DrawingArea::new();
    strip.set_size_request(-1, 28);
    {
        let swatches = swatches.clone();
        let rim = p.rim;
        strip.connect_draw(move |a, cr| {
            let (w, h) = (
                f64::from(a.allocated_width()),
                f64::from(a.allocated_height()),
            );
            rounded_rect(cr, 0.0, 0.0, w, h, 8.0);
            cr.clip();
            let total: f64 = swatches.iter().map(|s| s.share).sum::<f64>().max(1e-9);
            let mut x = 0.0;
            for s in &swatches {
                let sw = w * s.share / total;
                cr.rectangle(x, 0.0, sw + 0.5, h);
                cr.set_source_rgb(
                    f64::from(s.rgb[0]) / 255.0,
                    f64::from(s.rgb[1]) / 255.0,
                    f64::from(s.rgb[2]) / 255.0,
                );
                let _ = cr.fill();
                x += sw;
            }
            cr.reset_clip();
            rounded_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
            ui::set(cr, rim);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            glib::Propagation::Stop
        });
    }
    root.pack_start(&strip, false, false, 0);

    // Swatch tiles.
    let tiles = gtk::FlowBox::new();
    tiles.set_selection_mode(gtk::SelectionMode::None);
    tiles.set_column_spacing(8);
    tiles.set_row_spacing(8);
    tiles.set_min_children_per_line(4);
    tiles.set_max_children_per_line(4);
    tiles.set_homogeneous(true);
    let mut value_labels: Vec<(gtk::Label, Swatch)> = Vec::new();
    for s in &swatches {
        let b = gtk::Button::new();
        b.style_context().add_class("flat");
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let chip = gtk::DrawingArea::new();
        chip.set_size_request(110, 56);
        {
            let (s, rim) = (*s, p.rim);
            chip.connect_draw(move |a, cr| {
                let (w, h) = (
                    f64::from(a.allocated_width()),
                    f64::from(a.allocated_height()),
                );
                rounded_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 10.0);
                cr.set_source_rgb(
                    f64::from(s.rgb[0]) / 255.0,
                    f64::from(s.rgb[1]) / 255.0,
                    f64::from(s.rgb[2]) / 255.0,
                );
                let _ = cr.fill_preserve();
                ui::set(cr, rim);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
                // Share, written on the colour itself.
                let ink = if s.luma() > 0.6 { 0.0 } else { 1.0 };
                cr.set_source_rgba(ink, ink, ink, 0.75);
                let pct = format!("{:.0}%", (s.share * 100.0).max(1.0));
                let pal = ui::Palette::load();
                let (tw, th) = ui::text_size(cr, &pal, &pct, 11.0, true);
                ui::text(cr, &pal, &pct, w - tw - 8.0, h - th - 6.0, 11.0, true);
                glib::Propagation::Stop
            });
        }
        let value = gtk::Label::new(Some(&s.hex()));
        value.set_selectable(false);
        bx.pack_start(&chip, false, false, 0);
        bx.pack_start(&value, false, false, 0);
        b.add(&bx);
        {
            let (s, fmt, header) = (*s, fmt.clone(), header.clone());
            b.connect_clicked(move |_| {
                let v = format(&s, fmt.get());
                super::copy_text(&v);
                header.set_subtitle(Some(&format!("Copied {v}")));
            });
        }
        value_labels.push((value, *s));
        tiles.add(&b);
    }
    root.pack_start(&tiles, false, false, 0);

    // Format + bulk copy.
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let (seg, _) = {
        let (fmt, value_labels) = (fmt.clone(), value_labels.clone());
        ui::segmented(&["HEX", "RGB", "HSL"], 0, move |i| {
            let f = [Format::Hex, Format::Rgb, Format::Hsl][i];
            fmt.set(f);
            for (l, s) in &value_labels {
                l.set_text(&format(s, f));
            }
        })
    };
    let copy_css = ui::button("Copy as CSS", false);
    let copy_all = ui::button("Copy All", true);
    bottom.pack_start(&seg, false, false, 0);
    bottom.pack_end(&copy_all, false, false, 0);
    bottom.pack_end(&copy_css, false, false, 0);
    root.pack_start(&bottom, false, false, 0);
    {
        let (swatches, fmt, header) = (swatches.clone(), fmt.clone(), header.clone());
        copy_all.connect_clicked(move |_| {
            let all: Vec<String> = swatches.iter().map(|s| format(s, fmt.get())).collect();
            super::copy_text(&all.join("\n"));
            header.set_subtitle(Some(&format!("Copied {} colours", all.len())));
        });
    }
    {
        let (swatches, fmt, header) = (swatches.clone(), fmt.clone(), header.clone());
        copy_css.connect_clicked(move |_| {
            let css: Vec<String> = swatches
                .iter()
                .enumerate()
                .map(|(i, s)| format!("  --color-{}: {};", i + 1, format(s, fmt.get())))
                .collect();
            super::copy_text(&format!(":root {{\n{}\n}}", css.join("\n")));
            header.set_subtitle(Some("Copied CSS variables"));
        });
    }
    if swatches.is_empty() {
        copy_all.set_sensitive(false);
        copy_css.set_sensitive(false);
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
    fn dominant_colours_and_shares() {
        // 3/4 red, 1/4 blue.
        let img = RgbaImage::from_fn(100, 100, |x, _| {
            if x < 75 {
                image::Rgba([220, 30, 30, 255])
            } else {
                image::Rgba([20, 60, 230, 255])
            }
        });
        let s = extract(&img, 8);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].rgb, [220, 30, 30]);
        assert!((s[0].share - 0.75).abs() < 0.03);
        assert_eq!(s[1].hex(), "#143CE6");
    }

    #[test]
    fn edge_cases() {
        assert!(extract(&RgbaImage::new(0, 0), 8).is_empty());
        let transparent = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
        assert!(extract(&transparent, 8).is_empty());
        let flat = RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        assert_eq!(extract(&flat, 8).len(), 1);
        // A gradient is capped at the requested count.
        let grad = RgbaImage::from_fn(256, 4, |x, _| image::Rgba([x as u8, 0, 255 - x as u8, 255]));
        assert!(extract(&grad, 8).len() <= 8);
    }

    #[test]
    fn colour_formats() {
        let s = Swatch {
            rgb: [10, 132, 255],
            share: 1.0,
        };
        assert_eq!(s.hex(), "#0A84FF");
        assert_eq!(s.rgb_css(), "rgb(10, 132, 255)");
        assert_eq!(s.hsl_css(), "hsl(210, 100%, 52%)");
        assert_eq!(hsl([128, 128, 128]).1, 0.0);
    }
}
