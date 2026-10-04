//! QR code reading for captures (pure Rust, no extra system libraries).

use image::RgbaImage;

/// Decode every QR code found in `img`. Returns their contents.
pub fn decode(img: &RgbaImage) -> Vec<String> {
    let (w, h) = img.dimensions();
    if w < 21 || h < 21 {
        return Vec::new();
    }
    let mut prepared =
        rqrr::PreparedImage::prepare_from_greyscale(w as usize, h as usize, |x, y| {
            let [r, g, b, _] = img.get_pixel(x as u32, y as u32).0;
            ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8) as u8
        });
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, content)| content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_images_have_no_codes() {
        let img = RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
        assert!(decode(&img).is_empty());
        let tiny = RgbaImage::from_pixel(5, 5, image::Rgba([0, 0, 0, 255]));
        assert!(decode(&tiny).is_empty());
    }
}
