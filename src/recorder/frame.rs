//! Frame processing for the recorder: fitting captured window images into the
//! fixed-size video canvas and drawing the mouse cursor.
//!
//! All buffers are tightly-packed 32-bit `BGRx` (X11 ZPixmap byte order on
//! little-endian machines), which ffmpeg ingests directly as `bgr0`.

use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;

/// Bytes per pixel of every buffer handled here.
pub const BPP: usize = 4;

/// A fixed-size BGRx video frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Canvas {
    /// A black canvas. Dimensions are rounded down to even numbers because
    /// yuv420p (what the encoder produces) cannot represent odd sizes.
    pub fn new(width: u32, height: u32) -> Self {
        let (width, height) = (even(width), even(height));
        Canvas {
            width,
            height,
            data: vec![0; width as usize * height as usize * BPP],
        }
    }
}

/// Round down to an even number, never below 2.
pub fn even(v: u32) -> u32 {
    (v & !1).max(2)
}

/// Where a `src_w`×`src_h` image lands inside a `dst_w`×`dst_h` canvas when it
/// is scaled to fit while keeping its aspect ratio (letterbox / pillarbox).
/// Returns `(x, y, w, h)` of the image area inside the canvas.
pub fn fit_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    // Compare aspect ratios with integer cross-multiplication (no rounding).
    let (w, h) = if (src_w as u64) * (dst_h as u64) >= (src_h as u64) * (dst_w as u64) {
        // Source is wider: full width, reduced height.
        let h = ((src_h as u64 * dst_w as u64) / src_w as u64).max(1) as u32;
        (dst_w, h.min(dst_h))
    } else {
        let w = ((src_w as u64 * dst_h as u64) / src_h as u64).max(1) as u32;
        (w.min(dst_w), dst_h)
    };
    ((dst_w - w) / 2, (dst_h - h) / 2, w, h)
}

/// Precomputed bilinear sample positions along one axis: for each destination
/// coordinate, the two source coordinates and the 8-bit weight of the second.
fn axis_taps(src_len: u32, dst_len: u32) -> Vec<(u32, u32, u32)> {
    let scale = src_len as f64 / dst_len as f64;
    (0..dst_len)
        .map(|d| {
            let s = ((d as f64 + 0.5) * scale - 0.5).max(0.0);
            let i0 = (s.floor() as u32).min(src_len - 1);
            let i1 = (i0 + 1).min(src_len - 1);
            let w = ((s - i0 as f64) * 256.0).round().clamp(0.0, 256.0) as u32;
            (i0, i1, w)
        })
        .collect()
}

/// Scale `src` (a `src_w`×`src_h` BGRx image with `src_stride` bytes per row)
/// into `canvas`, letterboxed and centred, with bilinear filtering. Pixels
/// outside the image area are painted black. Same-size copies take a fast path.
pub fn blit_fit(src: &[u8], src_w: u32, src_h: u32, src_stride: usize, canvas: &mut Canvas) {
    let (cw, ch) = (canvas.width, canvas.height);
    let row_bytes = cw as usize * BPP;
    if src_w == 0
        || src_h == 0
        || src_stride < src_w as usize * BPP
        || src.len() < src_stride * (src_h as usize - 1) + src_w as usize * BPP
    {
        canvas.data.fill(0);
        return;
    }

    // Fast path: identical size → straight row copies.
    if src_w == cw && src_h == ch {
        canvas
            .data
            .par_chunks_mut(row_bytes)
            .enumerate()
            .for_each(|(y, row)| {
                let start = y * src_stride;
                row.copy_from_slice(&src[start..start + row_bytes]);
            });
        return;
    }

    let (ox, oy, fw, fh) = fit_rect(src_w, src_h, cw, ch);
    let xs = axis_taps(src_w, fw);
    let ys = axis_taps(src_h, fh);

    canvas
        .data
        .par_chunks_mut(row_bytes)
        .enumerate()
        .for_each(|(y, row)| {
            let y = y as u32;
            if y < oy || y >= oy + fh {
                row.fill(0);
                return;
            }
            let (y0, y1, wy) = ys[(y - oy) as usize];
            let r0 = &src[y0 as usize * src_stride..];
            let r1 = &src[y1 as usize * src_stride..];

            row[..ox as usize * BPP].fill(0);
            row[(ox + fw) as usize * BPP..].fill(0);

            for (i, &(x0, x1, wx)) in xs.iter().enumerate() {
                let d = (ox as usize + i) * BPP;
                let (a, b) = (x0 as usize * BPP, x1 as usize * BPP);
                for c in 0..3 {
                    let top = r0[a + c] as u32 * (256 - wx) + r0[b + c] as u32 * wx;
                    let bot = r1[a + c] as u32 * (256 - wx) + r1[b + c] as u32 * wx;
                    let v = (top * (256 - wy) + bot * wy + (1 << 15)) >> 16;
                    row[d + c] = v.min(255) as u8;
                }
                row[d + 3] = 255;
            }
        });
}

/// Alpha-blend a premultiplied ARGB cursor image (as returned by XFixes) onto a
/// BGRx image, with the cursor's top-left at `(x, y)` in image coordinates.
/// Parts of the cursor outside the image are clipped.
#[allow(clippy::too_many_arguments)]
pub fn blend_cursor(
    img: &mut [u8],
    img_w: u32,
    img_h: u32,
    stride: usize,
    cursor: &[u32],
    cur_w: u32,
    cur_h: u32,
    x: i32,
    y: i32,
) {
    if cursor.len() < (cur_w * cur_h) as usize {
        return;
    }
    for cy in 0..cur_h as i32 {
        let iy = y + cy;
        if iy < 0 || iy >= img_h as i32 {
            continue;
        }
        for cx in 0..cur_w as i32 {
            let ix = x + cx;
            if ix < 0 || ix >= img_w as i32 {
                continue;
            }
            let argb = cursor[(cy as u32 * cur_w + cx as u32) as usize];
            let a = argb >> 24;
            if a == 0 {
                continue;
            }
            let off = iy as usize * stride + ix as usize * BPP;
            let Some(px) = img.get_mut(off..off + 3) else {
                continue;
            };
            let src = [argb & 0xff, (argb >> 8) & 0xff, (argb >> 16) & 0xff];
            for c in 0..3 {
                // Premultiplied "over": dst = src + dst * (1 - a)
                let v = src[c] + (px[c] as u32 * (255 - a) + 127) / 255;
                px[c] = v.min(255) as u8;
            }
        }
    }
}

/// An integer rectangle in canvas pixels (may extend past the canvas).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectI {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        RectI { x, y, w, h }
    }

    pub fn intersect(&self, o: &RectI) -> Option<RectI> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        (x1 > x0 && y1 > y0).then(|| RectI::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// Signed distance from point `(px, py)` to a rounded rectangle (negative inside).
fn rounded_rect_distance(px: f32, py: f32, r: &RectI, radius: f32) -> f32 {
    let (hw, hh) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    let radius = radius.min(hw).min(hh).max(0.0);
    let (cx, cy) = (r.x as f32 + hw, r.y as f32 + hh);
    let qx = (px - cx).abs() - (hw - radius);
    let qy = (py - cy).abs() - (hh - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Paint a soft drop shadow for a window drawn at `dst` (clipped to `clip`).
/// Only the area outside `dst` is touched — the window covers the rest.
pub fn draw_shadow(
    canvas: &mut Canvas,
    dst: RectI,
    clip: RectI,
    radius: f32,
    blur: f32,
    opacity: f32,
) {
    let offset_y = (blur / 3.0).round() as i32;
    let shadow = RectI::new(dst.x, dst.y + offset_y, dst.w, dst.h);
    let b = blur.ceil() as i32;
    let area = RectI::new(
        shadow.x - b,
        shadow.y - b,
        shadow.w + 2 * b,
        shadow.h + 2 * b,
    );
    let canvas_rect = RectI::new(0, 0, canvas.width as i32, canvas.height as i32);
    let Some(area) = area
        .intersect(&clip)
        .and_then(|a| a.intersect(&canvas_rect))
    else {
        return;
    };
    let row_bytes = canvas.width as usize * BPP;
    canvas
        .data
        .par_chunks_mut(row_bytes)
        .enumerate()
        .skip(area.y as usize)
        .take(area.h as usize)
        .for_each(|(y, row)| {
            let py = y as f32 + 0.5;
            for x in area.x..area.x + area.w {
                let px = x as f32 + 0.5;
                if rounded_rect_distance(px, py, &dst, radius) < 0.0 {
                    continue;
                }
                let d = rounded_rect_distance(px, py, &shadow, radius);
                let t = (d / blur).clamp(0.0, 1.0);
                // smooth falloff
                let a = opacity * (1.0 - t * t * (3.0 - 2.0 * t));
                if a <= 0.004 {
                    continue;
                }
                let o = x as usize * BPP;
                for c in 0..3 {
                    row[o + c] = (row[o + c] as f32 * (1.0 - a)) as u8;
                }
            }
        });
}

/// Draw `src` scaled into `dst` (bilinear), clipped to `clip`, with
/// anti-aliased rounded corners of `radius` pixels.
#[allow(clippy::too_many_arguments)]
pub fn blit_into(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    src_stride: usize,
    canvas: &mut Canvas,
    dst: RectI,
    clip: RectI,
    radius: f32,
) {
    if src_w == 0
        || src_h == 0
        || dst.w <= 0
        || dst.h <= 0
        || src.len() < src_stride * (src_h as usize - 1) + src_w as usize * BPP
    {
        return;
    }
    let canvas_rect = RectI::new(0, 0, canvas.width as i32, canvas.height as i32);
    let Some(area) = dst.intersect(&clip).and_then(|a| a.intersect(&canvas_rect)) else {
        return;
    };
    let xs = axis_taps(src_w, dst.w as u32);
    let ys = axis_taps(src_h, dst.h as u32);
    let row_bytes = canvas.width as usize * BPP;
    let corner = radius.min(dst.w as f32 / 2.0).min(dst.h as f32 / 2.0);

    canvas
        .data
        .par_chunks_mut(row_bytes)
        .enumerate()
        .skip(area.y as usize)
        .take(area.h as usize)
        .for_each(|(y, row)| {
            let (y0, y1, wy) = ys[(y as i32 - dst.y) as usize];
            let r0 = &src[y0 as usize * src_stride..];
            let r1 = &src[y1 as usize * src_stride..];
            let py = y as f32 + 0.5;
            let near_y = py < (dst.y as f32 + corner) || py > (dst.y + dst.h) as f32 - corner;
            for x in area.x..area.x + area.w {
                // Coverage for rounded corners (1.0 away from the corners).
                let coverage = if corner > 0.0 && near_y {
                    let px = x as f32 + 0.5;
                    (0.5 - rounded_rect_distance(px, py, &dst, corner)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if coverage <= 0.0 {
                    continue;
                }
                let (x0, x1, wx) = xs[(x - dst.x) as usize];
                let (a, b) = (x0 as usize * BPP, x1 as usize * BPP);
                let o = x as usize * BPP;
                for c in 0..3 {
                    let top = r0[a + c] as u32 * (256 - wx) + r0[b + c] as u32 * wx;
                    let bot = r1[a + c] as u32 * (256 - wx) + r1[b + c] as u32 * wx;
                    let v = ((top * (256 - wy) + bot * wy + (1 << 15)) >> 16).min(255) as f32;
                    row[o + c] = (v * coverage + row[o + c] as f32 * (1.0 - coverage)) as u8;
                }
                row[o + 3] = 255;
            }
        });
}

/// Picture-in-picture layout: tiles for `sizes` (window sizes) stacked upward
/// from the bottom-right corner of the `area` (the recorded image), each at
/// most ~30% of the area, keeping aspect ratios.
pub fn pip_slots(area: RectI, sizes: &[(u32, u32)]) -> Vec<RectI> {
    let margin = (area.w.min(area.h) as f32 * 0.03).round().max(8.0) as i32;
    let max_w = (area.w as f32 * 0.30) as i32;
    let max_h = ((area.h - margin) / sizes.len().max(3) as i32 - margin).max(16);
    let mut slots = Vec::new();
    let mut bottom = area.y + area.h - margin;
    for &(w, h) in sizes {
        let (_, _, fw, fh) = fit_rect(w.max(1), h.max(1), max_w.max(16) as u32, max_h as u32);
        let (fw, fh) = (fw as i32, fh as i32);
        let slot = RectI::new(area.x + area.w - margin - fw, bottom - fh, fw, fh);
        bottom = slot.y - margin;
        slots.push(slot);
    }
    slots
}

/// Copy the `rect` area from `src` into `dst`; both are `stride`-wide BGRx
/// images of the same size. Parts of `rect` outside the image are ignored.
pub fn copy_rect(dst: &mut [u8], src: &[u8], width: u32, height: u32, rect: RectI) {
    let stride = width as usize * BPP;
    let Some(r) = rect.intersect(&RectI::new(0, 0, width as i32, height as i32)) else {
        return;
    };
    if dst.len() < stride * height as usize || src.len() < stride * height as usize {
        return;
    }
    for y in r.y..r.y + r.h {
        let a = y as usize * stride + r.x as usize * BPP;
        let b = a + r.w as usize * BPP;
        dst[a..b].copy_from_slice(&src[a..b]);
    }
}

/// Fill the `rect` area of a BGRx image with one colour.
pub fn fill_rect(dst: &mut [u8], width: u32, height: u32, rect: RectI, bgr: [u8; 3]) {
    let stride = width as usize * BPP;
    let Some(r) = rect.intersect(&RectI::new(0, 0, width as i32, height as i32)) else {
        return;
    };
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let o = y as usize * stride + x as usize * BPP;
            if let Some(px) = dst.get_mut(o..o + 3) {
                px.copy_from_slice(&bgr);
            }
        }
    }
}

/// Draw a `w`×`h` patch (tightly packed, 32 bits per pixel) at `(x, y)` of a
/// BGRx image, clipped. With `alpha` the patch is premultiplied ARGB (depth-32
/// windows: rounded corners, client-side shadows) and is blended "over";
/// otherwise it is copied.
#[allow(clippy::too_many_arguments)]
pub fn draw_patch(
    dst: &mut [u8],
    width: u32,
    height: u32,
    patch: &[u8],
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    alpha: bool,
) {
    let stride = width as usize * BPP;
    let target = RectI::new(x, y, w as i32, h as i32);
    let Some(r) = target.intersect(&RectI::new(0, 0, width as i32, height as i32)) else {
        return;
    };
    if patch.len() < w as usize * h as usize * BPP {
        return;
    }
    for row in r.y..r.y + r.h {
        let sy = (row - y) as usize;
        for col in r.x..r.x + r.w {
            let sx = (col - x) as usize;
            let s = (sy * w as usize + sx) * BPP;
            let d = row as usize * stride + col as usize * BPP;
            if alpha {
                let a = patch[s + 3] as u32;
                if a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let v = patch[s + c] as u32 + (dst[d + c] as u32 * (255 - a) + 127) / 255;
                    dst[d + c] = v.min(255) as u8;
                }
            } else {
                dst[d..d + 3].copy_from_slice(&patch[s..s + 3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgr: [u8; 3]) -> Vec<u8> {
        (0..w * h)
            .flat_map(|_| [bgr[0], bgr[1], bgr[2], 0])
            .collect()
    }

    #[test]
    fn fit_rect_letterboxes_and_pillarboxes() {
        // 4:3 into 16:9 → pillarbox (bars left/right)
        assert_eq!(fit_rect(800, 600, 1920, 1080), (240, 0, 1440, 1080));
        // 21:9-ish into 16:9 → letterbox (bars top/bottom)
        assert_eq!(fit_rect(2560, 1080, 1920, 1080), (0, 135, 1920, 810));
        // Same aspect → fills the canvas
        assert_eq!(fit_rect(1280, 720, 1920, 1080), (0, 0, 1920, 1080));
        assert_eq!(fit_rect(0, 10, 100, 100), (0, 0, 0, 0));
    }

    #[test]
    fn canvas_dimensions_are_even() {
        let c = Canvas::new(801, 601);
        assert_eq!((c.width, c.height), (800, 600));
        assert_eq!(c.data.len(), 800 * 600 * 4);
    }

    #[test]
    fn same_size_blit_is_an_exact_copy() {
        let src = solid(8, 4, [1, 2, 3]);
        let mut canvas = Canvas::new(8, 4);
        blit_fit(&src, 8, 4, 8 * 4, &mut canvas);
        assert_eq!(canvas.data, src);
    }

    #[test]
    fn scaled_blit_fills_image_area_and_blacks_out_bars() {
        // 2:1 red source into a square canvas → letterbox bars top and bottom.
        let src = solid(20, 10, [0, 0, 200]);
        let mut canvas = Canvas::new(10, 10);
        canvas.data.fill(77); // stale content must be overwritten
        blit_fit(&src, 20, 10, 20 * 4, &mut canvas);
        let px = |x: usize, y: usize| &canvas.data[(y * 10 + x) * 4..(y * 10 + x) * 4 + 3];
        assert_eq!(px(5, 0), [0, 0, 0]); // top bar
        assert_eq!(px(5, 9), [0, 0, 0]); // bottom bar
        assert_eq!(px(5, 5), [0, 0, 200]); // image area keeps the colour
    }

    #[test]
    fn pip_slots_stack_in_bottom_right_without_overlap() {
        let area = RectI::new(0, 0, 1920, 1080);
        let slots = pip_slots(area, &[(1280, 720), (800, 800)]);
        assert_eq!(slots.len(), 2);
        for s in &slots {
            assert!(s.x + s.w <= area.w && s.y >= 0 && s.w <= 576);
        }
        assert!(slots[1].y + slots[1].h <= slots[0].y); // second sits above the first
    }

    #[test]
    fn blit_into_draws_clipped_rounded_layer_with_shadow() {
        let mut canvas = Canvas::new(40, 40);
        canvas.data.fill(200);
        let src = solid(10, 10, [0, 0, 255]);
        let dst = RectI::new(10, 10, 20, 20);
        let clip = RectI::new(0, 0, 25, 40); // right part clipped away
        draw_shadow(&mut canvas, dst, clip, 4.0, 6.0, 0.5);
        blit_into(&src, 10, 10, 40, &mut canvas, dst, clip, 4.0);
        let px = |x: usize, y: usize| canvas.data[(y * 40 + x) * 4..(y * 40 + x) * 4 + 3].to_vec();
        assert_eq!(px(20, 20), vec![0, 0, 255]); // layer centre
        assert_eq!(px(28, 20), vec![200, 200, 200]); // clipped → untouched
        // Rounded corner: the layer (B=0) is not drawn there, only shadow.
        assert!(px(10, 10)[0] > 100);
        assert!(px(15, 32)[0] < 200); // shadow below the layer darkens
    }

    #[test]
    fn copy_and_fill_rect_touch_only_the_rect() {
        let mut dst = solid(4, 4, [1, 1, 1]);
        let src = solid(4, 4, [9, 9, 9]);
        copy_rect(&mut dst, &src, 4, 4, RectI::new(2, 2, 10, 10)); // clipped
        assert_eq!(&dst[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 3], [9, 9, 9]);
        assert_eq!(&dst[0..3], [1, 1, 1]);
        fill_rect(&mut dst, 4, 4, RectI::new(-1, -1, 2, 2), [5, 5, 5]);
        assert_eq!(&dst[0..3], [5, 5, 5]);
        assert_eq!(&dst[4..7], [1, 1, 1]);
    }

    #[test]
    fn draw_patch_copies_or_blends_premultiplied() {
        let mut dst = solid(3, 1, [100, 100, 100]);
        // opaque red, transparent, 50% (premultiplied) blue
        let patch = [0, 0, 255, 255, 0, 0, 0, 0, 128, 0, 0, 128];
        draw_patch(&mut dst, 3, 1, &patch, 0, 0, 3, 1, true);
        assert_eq!(&dst[0..3], [0, 0, 255]);
        assert_eq!(&dst[4..7], [100, 100, 100]);
        assert!(dst[8] > 150 && dst[10] < 60, "{:?}", &dst[8..11]);

        let mut dst = solid(3, 1, [100, 100, 100]);
        draw_patch(&mut dst, 3, 1, &patch, 1, 0, 3, 1, false); // clipped copy
        assert_eq!(&dst[0..3], [100, 100, 100]);
        assert_eq!(&dst[4..7], [0, 0, 255]);
    }

    #[test]
    fn cursor_blend_respects_alpha_and_clips() {
        let mut img = solid(4, 4, [100, 100, 100]);
        // 2x2 cursor: opaque white, fully transparent, 50% black (premultiplied → 0 rgb)
        let cursor = [0xffff_ffff, 0x0000_0000, 0x8000_0000, 0xffff_ffff];
        blend_cursor(&mut img, 4, 4, 16, &cursor, 2, 2, 3, 3); // mostly off-image
        assert_eq!(&img[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 3], [255, 255, 255]);

        let mut img = solid(4, 4, [100, 100, 100]);
        blend_cursor(&mut img, 4, 4, 16, &cursor, 2, 2, 0, 0);
        assert_eq!(&img[0..3], [255, 255, 255]); // opaque
        assert_eq!(&img[4..7], [100, 100, 100]); // transparent → untouched
        let half = &img[16..19]; // 50% black over grey → darker grey
        assert!(half.iter().all(|&v| (45..=55).contains(&v)), "{half:?}");
    }
}
