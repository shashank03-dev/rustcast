//! Screenshot studio: capture, annotate, OCR, pin.
//!
//! Everything here runs in a short-lived subprocess (`rustcast --snap <mode>`)
//! so the resident launcher never holds a frozen screen, a GTK editor or an OCR
//! engine in memory — when the capture is done the process exits and all of it
//! is returned to the system. That matters on low-RAM machines.
//!
//! Modes:
//! - `area`       freeze the screen, select a region (or click a window), annotate
//! - `window`     same overlay, starting in window-pick mode
//! - `fullscreen` capture the whole monitor under the pointer immediately
//! - `quick`      select a region, copy + save instantly (no annotation)
//! - `ocr`        select a region, copy the text in it
//! - `edit <png>` open an existing image in the annotation editor
//! - `pin <png>`  float an image above all windows
//! - `ocr-file <png>` read the text of an existing image
//!
//! `--delay <secs>` shows a countdown before any capture mode.
//!
//! Results reach the clipboard through the running RustCast instance (see
//! [`copy_text`] / [`copy_image`]) so they outlive this process and show up in
//! the clipboard history.

pub mod beautify;
pub mod editor;
pub mod grab;
pub mod ocr;
pub mod pin;
pub mod qr;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::config::{Config, ScreenshotConfig};

/// Spawn a screenshot subprocess, e.g. `spawn(&["area"])` or
/// `spawn(&["edit", "/path/shot.png"])`.
pub fn spawn(args: &[&str]) {
    let Ok(exe) = std::env::current_exe() else {
        log::warn!("cannot locate own exe to start a capture");
        return;
    };
    let mut cmd = Command::new(exe);
    cmd.arg("--snap").args(args);
    // Same reasoning as the thumbnail overlay: run on X11/XWayland so the
    // overlay can cover a whole monitor at a fixed position. The real Wayland
    // display stays available as RUSTCAST_WAYLAND_DISPLAY (for the portal).
    cmd.env("GDK_BACKEND", "x11");
    cmd.env_remove("WAYLAND_DISPLAY");
    if let Err(e) = cmd.spawn() {
        log::warn!("failed to start capture: {e}");
    }
}

/// Entry point of the `--snap` subprocess. `args` are the arguments after `--snap`.
pub fn run(args: &[String]) {
    force_x11_backend();

    let mut mode = String::from("area");
    let mut path: Option<PathBuf> = None;
    let mut delay = 0u64;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--delay" => delay = it.next().and_then(|d| d.parse().ok()).unwrap_or(0),
            m if !m.starts_with("--") && path.is_none() && mode_takes_path(&mode) => {
                path = Some(PathBuf::from(m));
            }
            m if !m.starts_with("--") => {
                mode = m.to_string();
            }
            _ => {}
        }
    }

    if gtk::init().is_err() {
        eprintln!("rustcast snap: GTK init failed");
        return;
    }
    let cfg = load_config().screenshot;

    match mode.as_str() {
        "edit" | "pin" | "ocr-file" => {
            let Some(path) = path else {
                eprintln!("rustcast snap: {mode} needs an image path");
                return;
            };
            match mode.as_str() {
                "edit" => editor::run_file(&path, cfg),
                "pin" => pin::run(&path),
                _ => ocr::run_file(&path, cfg),
            }
        }
        "area" | "window" | "quick" | "ocr" | "fullscreen" => {
            if delay > 0 {
                countdown(delay);
            }
            let flavor = match mode.as_str() {
                "window" => editor::Flavor::Window,
                "quick" => editor::Flavor::Quick,
                "ocr" => editor::Flavor::Ocr,
                "fullscreen" => editor::Flavor::Fullscreen,
                _ => editor::Flavor::Area,
            };
            editor::run_capture(flavor, cfg);
        }
        other => eprintln!("rustcast snap: unknown mode '{other}'"),
    }
}

fn mode_takes_path(mode: &str) -> bool {
    matches!(mode, "edit" | "pin" | "ocr-file")
}

/// When started straight from a Wayland terminal, move GTK onto XWayland the
/// same way `main` does for the launcher (window positioning needs X11).
fn force_x11_backend() {
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // SAFETY: called first thing in the subprocess, before any threads exist.
        unsafe {
            if let Some(wd) = std::env::var_os("WAYLAND_DISPLAY") {
                std::env::set_var("RUSTCAST_WAYLAND_DISPLAY", wd);
            }
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }
}

pub fn load_config() -> Config {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_to_string(format!("{home}/.config/rustcast/config.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// A small "3… 2… 1…" pill before a delayed capture. Esc cancels (exits).
fn countdown(secs: u64) {
    use gtk::glib;
    use gtk::prelude::*;

    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_keep_above(true);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    let size = 120;
    window.set_default_size(size, size);
    window.set_position(gtk::WindowPosition::Center);

    let remaining = std::rc::Rc::new(std::cell::Cell::new(secs));
    {
        let remaining = remaining.clone();
        window.connect_draw(move |_, cr| {
            cr.set_operator(gtk::cairo::Operator::Source);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = cr.paint();
            cr.set_operator(gtk::cairo::Operator::Over);
            cr.arc(60.0, 60.0, 54.0, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(0.08, 0.08, 0.1, 0.85);
            let _ = cr.fill();
            let layout = pangocairo::functions::create_layout(cr);
            let mut font = gtk::pango::FontDescription::from_string("Sans Bold");
            font.set_absolute_size(52.0 * f64::from(gtk::pango::SCALE));
            layout.set_font_description(Some(&font));
            layout.set_text(&remaining.get().to_string());
            let (w, h) = layout.pixel_size();
            cr.move_to(60.0 - f64::from(w) / 2.0, 60.0 - f64::from(h) / 2.0);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            pangocairo::functions::show_layout(cr, &layout);
            glib::Propagation::Stop
        });
    }
    window.show_all();

    let main_loop = glib::MainLoop::new(None, false);
    {
        let main_loop = main_loop.clone();
        let window = window.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let left = remaining.get().saturating_sub(1);
            remaining.set(left);
            if left == 0 {
                window.close();
                main_loop.quit();
                return glib::ControlFlow::Break;
            }
            window.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    main_loop.run();
    // Let the compositor remove the pill before the screen is grabbed.
    flush_gtk();
    std::thread::sleep(Duration::from_millis(180));
}

/// Run pending GTK events (e.g. so a hidden window is really gone).
pub fn flush_gtk() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

fn timestamp_name(prefix: &str, ext: &str) -> String {
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{ms}.{ext}")
}

/// Encode `img` according to the configured format and write it into `dir`.
pub fn write_image(img: &image::RgbaImage, dir: &Path, cfg: &ScreenshotConfig) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let ext = cfg.extension();
    let path = dir.join(timestamp_name("rustcast", ext));
    write_image_to(img, &path, cfg).then_some(path)
}

/// Encode `img` to `path`, picking the format from the extension.
pub fn write_image_to(img: &image::RgbaImage, path: &Path, cfg: &ScreenshotConfig) -> bool {
    // Write to a dot-file first and rename, so the screenshot watcher never
    // reads a half-written image.
    let Some(name) = path.file_name() else {
        return false;
    };
    let tmp = path.with_file_name(format!(".{}.part", name.to_string_lossy()));
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let result = (|| -> image::ImageResult<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        match ext.as_str() {
            "jpg" | "jpeg" => {
                let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
                let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    file,
                    cfg.jpeg_quality.clamp(10, 100),
                );
                rgb.write_with_encoder(enc)
            }
            "webp" => img.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(file)),
            _ => img.write_with_encoder(image::codecs::png::PngEncoder::new_with_quality(
                file,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            )),
        }
    })();
    match result.and_then(|_| std::fs::rename(&tmp, path).map_err(Into::into)) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("could not write {}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
            false
        }
    }
}

/// Put text on the clipboard. Prefers the running RustCast instance (so the
/// text survives this process and lands in clipboard history); otherwise owns
/// the clipboard here and asks a clipboard manager to keep it.
pub fn copy_text(text: &str) {
    let url = format!(
        "rustcast://snap-copy?text={}",
        url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>()
    );
    if crate::platform::urlscheme::forward_if_running(&url) {
        return;
    }
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(text);
    clipboard.store();
}

/// Put the image stored at `path` on the clipboard (see [`copy_text`]).
pub fn copy_image(path: &Path) {
    let url = format!(
        "rustcast://snap-copy?image={}",
        url::form_urlencoded::byte_serialize(path.to_string_lossy().as_bytes()).collect::<String>()
    );
    if crate::platform::urlscheme::forward_if_running(&url) {
        return;
    }
    if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file(path) {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_image(&pixbuf);
        clipboard.store();
    }
}

/// Main-process side of [`copy_text`] / [`copy_image`]: handle a
/// `rustcast://snap-copy?...` URL.
pub fn handle_copy_url(url: &url::Url) {
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "text" => {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    cb.set_text(value.into_owned()).ok();
                }
            }
            "image" => {
                let path = PathBuf::from(value.as_ref());
                let Ok(img) = image::open(&path) else {
                    continue;
                };
                let rgba = img.to_rgba8();
                let data = arboard::ImageData {
                    width: rgba.width() as usize,
                    height: rgba.height() as usize,
                    bytes: std::borrow::Cow::Owned(rgba.into_raw()),
                };
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    cb.set_image(data).ok();
                }
            }
            _ => {}
        }
    }
}

/// Wait (max ~1.5 s) so a clipboard manager can take over what this process
/// owns, when the main instance was not reachable.
pub fn linger_for_clipboard() {
    let until = std::time::Instant::now() + Duration::from_millis(1500);
    while std::time::Instant::now() < until {
        flush_gtk();
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Is a RustCast instance running to receive clipboard handoffs?
pub fn main_instance_running() -> bool {
    crate::platform::urlscheme::is_running()
}

/// Open a URL with the desktop's default handler.
pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}
