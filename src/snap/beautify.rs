//! "Beautify": put a capture on a gradient backdrop with padding, rounded
//! corners and a soft drop shadow — ready for docs, slides and social posts.
//! Also home of the box blur shared with the editor's censor tool.

use gtk::cairo::{self, Context, Format, ImageSurface, LinearGradient};

use super::grab::{Frame, bgra_to_rgba};

pub struct Style {
    pub name: &'static str,
    pub from: (f64, f64, f64),
    pub to: (f64, f64, f64),
}

const fn rgb(hex: u32) -> (f64, f64, f64) {
    (
        ((hex >> 16) & 0xff) as f64 / 255.0,
        ((hex >> 8) & 0xff) as f64 / 255.0,
        (hex & 0xff) as f64 / 255.0,
    )
}

pub const STYLES: [Style; 8] = [
    Style {
        name: "Sunset",
        from: rgb(0xff7e5f),
        to: rgb(0xfeb47b),
    },
    Style {
        name: "Ocean",
        from: rgb(0x2193b0),
        to: rgb(0x6dd5ed),
    },
    Style {
        name: "Grape",
        from: rgb(0x8e2de2),
        to: rgb(0x4a00e0),
    },
    Style {
        name: "Mint",
        from: rgb(0x11998e),
        to: rgb(0x38ef7d),
    },
    Style {
        name: "Peach",
        from: rgb(0xee9ca7),
        to: rgb(0xffdde1),
    },
    Style {
        name: "Night",
        from: rgb(0x232526),
        to: rgb(0x414345),
    },
    Style {
        name: "Rust",
        from: rgb(0xb7410e),
        to: rgb(0xf09819),
    },
    Style {
        name: "Sky",
        from: rgb(0x89f7fe),
        to: rgb(0x66a6ff),
    },
];

/// Rounded-rectangle path.
pub fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Apply style `idx` to `img`. `k` scales paddings for HiDPI captures.
pub fn apply(img: &image::RgbaImage, idx: usize, k: f64) -> image::RgbaImage {
    let style = &STYLES[idx % STYLES.len()];
    let (w, h) = (f64::from(img.width()), f64::from(img.height()));
    let pad = (w.min(h) * 0.08).clamp(32.0 * k, 96.0 * k).round();
    let radius = 12.0 * k;
    let (ow, oh) = ((w + 2.0 * pad) as i32, (h + 2.0 * pad) as i32);

    let render = || -> Result<image::RgbaImage, cairo::Error> {
        let out = ImageSurface::create(Format::ARgb32, ow, oh)?;
        let cr = Context::new(&out)?;

        let grad = LinearGradient::new(0.0, 0.0, f64::from(ow), f64::from(oh));
        grad.add_color_stop_rgb(0.0, style.from.0, style.from.1, style.from.2);
        grad.add_color_stop_rgb(1.0, style.to.0, style.to.1, style.to.2);
        cr.set_source(&grad)?;
        cr.paint()?;

        // Shadow: a dark rounded rect, blurred, offset slightly downwards.
        let blur = (18.0 * k) as i32;
        let mut shadow = ImageSurface::create(Format::ARgb32, ow, oh)?;
        {
            let sc = Context::new(&shadow)?;
            rounded_rect(&sc, pad, pad + 6.0 * k, w, h, radius);
            sc.set_source_rgba(0.0, 0.0, 0.0, 0.45);
            sc.fill()?;
        }
        {
            let stride = shadow.stride();
            let mut data = shadow.data().map_err(|_| cairo::Error::SurfaceFinished)?;
            box_blur(&mut data, ow, oh, stride, blur.max(1));
        }
        cr.set_source_surface(&shadow, 0.0, 0.0)?;
        cr.paint()?;

        let frame = Frame::from_rgba(img.clone());
        let stride = frame.stride();
        let src = ImageSurface::create_for_data(
            frame.data,
            Format::ARgb32,
            frame.width,
            frame.height,
            stride,
        )?;
        rounded_rect(&cr, pad, pad, w, h, radius);
        cr.clip();
        cr.set_source_surface(&src, pad, pad)?;
        cr.paint()?;
        drop(cr);

        out.flush();
        let mut rgba = None;
        out.with_data(|d| rgba = Some(bgra_to_rgba(d, ow, oh, out.stride())))
            .map_err(|_| cairo::Error::SurfaceFinished)?;
        Ok(rgba.unwrap_or_default())
    };
    render().unwrap_or_else(|_| img.clone())
}

/// In-place blur of a 4-channel buffer: three box passes ≈ a Gaussian, each
/// pass O(pixels) regardless of the radius (fast on slow CPUs).
pub fn box_blur(data: &mut [u8], w: i32, h: i32, stride: i32, radius: i32) {
    if w <= 0 || h <= 0 || radius <= 0 {
        return;
    }
    let (w, h, stride) = (w as usize, h as usize, stride as usize);
    let r = radius as usize;
    let mut line: Vec<[u32; 4]> = Vec::with_capacity(w.max(h));
    for _ in 0..3 {
        // Horizontal.
        for y in 0..h {
            line.clear();
            line.extend((0..w).map(|x| {
                let i = y * stride + x * 4;
                [
                    u32::from(data[i]),
                    u32::from(data[i + 1]),
                    u32::from(data[i + 2]),
                    u32::from(data[i + 3]),
                ]
            }));
            blur_line(&line, r, |x, px| {
                let i = y * stride + x * 4;
                data[i..i + 4].copy_from_slice(&px);
            });
        }
        // Vertical.
        for x in 0..w {
            line.clear();
            line.extend((0..h).map(|y| {
                let i = y * stride + x * 4;
                [
                    u32::from(data[i]),
                    u32::from(data[i + 1]),
                    u32::from(data[i + 2]),
                    u32::from(data[i + 3]),
                ]
            }));
            blur_line(&line, r, |y, px| {
                let i = y * stride + x * 4;
                data[i..i + 4].copy_from_slice(&px);
            });
        }
    }
}

/// Running-sum box filter over one row/column (edges clamped).
fn blur_line(src: &[[u32; 4]], r: usize, mut put: impl FnMut(usize, [u8; 4])) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let window = (2 * r + 1) as u32;
    let at = |i: isize| src[i.clamp(0, n as isize - 1) as usize];
    let mut sum = [0u32; 4];
    for i in -(r as isize)..=(r as isize) {
        let p = at(i);
        for c in 0..4 {
            sum[c] += p[c];
        }
    }
    for x in 0..n {
        put(x, std::array::from_fn(|c| (sum[c] / window) as u8));
        let add = at(x as isize + r as isize + 1);
        let sub = at(x as isize - r as isize);
        for c in 0..4 {
            sum[c] = sum[c] + add[c] - sub[c];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_flat_areas_and_softens_edges() {
        let (w, h) = (20, 4);
        let mut data = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 10..w {
                data[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[255; 4]);
            }
        }
        box_blur(&mut data, w as i32, h as i32, (w * 4) as i32, 2);
        let px = |x: usize| data[x * 4];
        assert_eq!(px(0), 0);
        assert_eq!(px(19), 255);
        assert!(px(9) > 0 && px(9) < 255);
        assert!(px(10) > 0 && px(10) < 255);
    }

    #[test]
    fn beautify_adds_padding() {
        let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([10, 20, 30, 255]));
        let out = apply(&img, 1, 1.0);
        assert!(out.width() > 200 && out.height() > 100);
        // The capture itself sits in the middle, untouched.
        let c = out.get_pixel(out.width() / 2, out.height() / 2).0;
        assert_eq!(c, [10, 20, 30, 255]);
    }
}
