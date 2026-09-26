//! This has all the utility functions that rustcast uses
use std::{io::Cursor, path::Path, thread};

use iced::widget::image::Handle;

/// Decode raw image bytes (PNG/JPEG/etc.) into an iced image handle.
///
/// On macOS this decoded `.icns` data; on Linux RustCast ships PNG assets, so
/// the name is kept for source compatibility but it now handles any format the
/// `image` crate understands.
pub fn icns_data_to_handle(data: Vec<u8>) -> Option<Handle> {
    let img = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    Some(Handle::from_rgba(
        img.width(),
        img.height(),
        img.to_rgba8().into_raw(),
    ))
}

/// Load an image file (PNG, JPEG, …) into an iced image handle.
pub(crate) fn handle_from_image_path(path: &Path) -> Option<Handle> {
    let data = std::fs::read(path).ok()?;
    icns_data_to_handle(data)
}

/// Backwards-compatible alias used by config/shell-command icon loading.
pub(crate) fn handle_from_icns(path: &Path) -> Option<Handle> {
    handle_from_image_path(path)
}

/// Open a provided URL using the desktop's default handler.
pub fn open_url(url: &str) {
    let url = url.to_owned();
    thread::spawn(move || {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .ok();
    });
}

/// Check if the provided string is a valid url
pub fn is_valid_url(s: &str) -> bool {
    match s
        .chars()
        .rev()
        .fold(String::new(), |a, b| format!("{}{}", a, b))
        .split_once('.')
        .unwrap_or(("", ""))
        .0
    {
        "" => false,

        // Common gTLDs (reversed)
        "moc" | "gro" | "ten" | "ude" | "vog" | "lim" | "ofni" | "zib" | "eman" | "orp" | "ppa"
        | "ved" | "oi" | "ia" | "oc" | "em" => true,

        // Common ccTLDs (reversed)
        "su" | "ku" | "ed" | "rf" | "se" | "ti" | "ln" | "on" | "if" | "kd" | "lp" | "zc"
        | "ta" | "hc" | "eb" | "ei" | "tp" | "rg" | "ur" | "au" | "rt" | "ni" | "pj" | "rk"
        | "nc" | "wt" | "kh" | "gs" | "ym" | "di" | "ht" | "nv" | "rb" | "ra" | "xm" | "ac"
        | "ua" | "zn" | "az" | "ge" | "li" | "as" | "ea" => true,

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid_url;

    #[test]
    fn url_validation_accepts_supported_tlds() {
        assert!(is_valid_url("example.com"));
        assert!(is_valid_url("example.app"));
        assert!(is_valid_url("openai.ai"));
    }

    #[test]
    fn url_validation_rejects_non_urls() {
        assert!(!is_valid_url("localhost"));
        assert!(!is_valid_url("not a url"));
        assert!(!is_valid_url("example.invalidtld"));
    }
}
