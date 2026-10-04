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
//! - `window-id <xid>` capture that window (X11), then annotate
//! - `fullscreen` capture the whole monitor under the pointer immediately
//! - `quick`      select a region, copy + save instantly (no annotation)
//! - `ocr`        select a region, copy the text in it (`--as text|code|table`;
//!   `ocr-code` / `ocr-table` are shorthands)
//! - `palette`    select a region, show its colour palette
//! - `edit <img>` open an existing image in the annotation editor
//! - `pin <img>`  float an image above all windows
//! - `ocr-file <img>` / `palette-file <img>` the same for an image on disk
//! - `compare [before] [after]` before/after view (default: last two captures)
//!
//! `--delay <secs>` shows a countdown before any capture mode.
//!
//! Results reach the clipboard through the running RustCast instance (see
//! [`copy_text`] / [`copy_image`]) so they outlive this process and show up in
//! the clipboard history.

pub mod beautify;
pub mod compare;
pub mod editor;
pub mod grab;
pub mod ocr;
pub mod ocr_window;
pub mod palette;
pub mod pin;
pub mod qr;
pub mod ui;

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

    let mut positional: Vec<String> = Vec::new();
    let mut delay = 0u64;
    let mut layout: Option<ocr::Layout> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--delay" => delay = it.next().and_then(|d| d.parse().ok()).unwrap_or(0),
            "--as" => layout = it.next().and_then(|l| ocr::Layout::parse(l)),
            f if f.starts_with("--") => {}
            v => positional.push(v.to_string()),
        }
    }
    let mut mode = positional
        .first()
        .cloned()
        .unwrap_or_else(|| "area".to_string());
    match mode.as_str() {
        "ocr-code" => (mode, layout) = ("ocr".into(), Some(ocr::Layout::Code)),
        "ocr-table" => (mode, layout) = ("ocr".into(), Some(ocr::Layout::Table)),
        _ => {}
    }
    let arg = |i: usize| positional.get(i).map(PathBuf::from);

    if gtk::init().is_err() {
        eprintln!("rustcast snap: GTK init failed");
        return;
    }
    ui::install_css();
    let cfg = load_config().screenshot;

    match mode.as_str() {
        "edit" | "pin" | "ocr-file" | "palette-file" => {
            let Some(path) = arg(1) else {
                eprintln!("rustcast snap: {mode} needs an image path");
                return;
            };
            if !path.exists() {
                error_dialog(&format!("{} no longer exists.", path.display()));
                return;
            }
            match mode.as_str() {
                "edit" => editor::run_file(&path, cfg),
                "pin" => pin::run(&path),
                "palette-file" => palette::run_file(&path),
                _ => ocr_window::run_file(&path, cfg, layout),
            }
        }
        "compare" => compare::run(arg(1), arg(2)),
        "window-id" => {
            let Some(xid) = positional.get(1).and_then(|x| parse_xid(x)) else {
                eprintln!("rustcast snap: window-id needs a window id");
                return;
            };
            if delay > 0 {
                countdown(delay);
            }
            editor::run_capture(editor::Flavor::Area, cfg, None, Some(xid));
        }
        "area" | "window" | "quick" | "ocr" | "fullscreen" | "palette" => {
            if delay > 0 {
                countdown(delay);
            }
            let flavor = match mode.as_str() {
                "window" => editor::Flavor::Window,
                "quick" => editor::Flavor::Quick,
                "ocr" => editor::Flavor::Ocr,
                "palette" => editor::Flavor::Palette,
                "fullscreen" => editor::Flavor::Fullscreen,
                _ => editor::Flavor::Area,
            };
            editor::run_capture(flavor, cfg, layout, None);
        }
        other => eprintln!("rustcast snap: unknown mode '{other}'"),
    }
}

fn parse_xid(s: &str) -> Option<u32> {
    match s.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// While this file exists a capture is in progress; floating thumbnails hide
/// themselves so they never end up in the screenshot.
pub fn capturing_marker() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("rustcast-capturing")
}

/// True while a capture is really running: the marker exists and the
/// process that wrote it is alive (a killed capture must not leave
/// thumbnails hidden forever).
pub fn capture_in_progress() -> bool {
    std::fs::read_to_string(capturing_marker())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

/// Holds the capture marker; removes it when dropped (also on early return).
pub struct CaptureGuard;

impl CaptureGuard {
    /// Create the marker and give visible thumbnails a moment to hide.
    pub fn begin() -> Self {
        let _ = std::fs::write(capturing_marker(), std::process::id().to_string());
        if crate::platform::linux::overlay_gtk::any_visible() {
            std::thread::sleep(Duration::from_millis(220));
        }
        CaptureGuard
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(capturing_marker());
    }
}

/// A RustCast-styled message window. Blocks until closed.
pub fn error_dialog(msg: &str) {
    use gtk::prelude::*;
    let (window, _) = ui::panel_window("RustCast", None);
    window.set_default_size(420, -1);
    window.set_resizable(false);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(6);
    root.set_margin_bottom(14);
    root.set_margin_start(18);
    root.set_margin_end(18);
    let label = gtk::Label::new(Some(msg));
    label.set_line_wrap(true);
    label.set_selectable(true);
    label.set_xalign(0.0);
    label.set_max_width_chars(60);
    root.pack_start(&label, true, true, 0);
    let ok = ui::button("OK", true);
    ok.set_halign(gtk::Align::End);
    {
        let window = window.clone();
        ok.connect_clicked(move |_| window.close());
    }
    root.pack_start(&ok, false, false, 0);
    window.add(&root);
    let main_loop = gtk::glib::MainLoop::new(None, false);
    {
        let main_loop = main_loop.clone();
        window.connect_destroy(move |_| main_loop.quit());
    }
    window.show_all();
    window.present();
    main_loop.run();
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

/// A small "3… 2… 1…" pill before a delayed capture. Clicking it cancels.
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
        let p = ui::Palette::load();
        window.connect_draw(move |_, cr| {
            cr.set_operator(gtk::cairo::Operator::Source);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = cr.paint();
            cr.set_operator(gtk::cairo::Operator::Over);
            cr.arc(60.0, 60.0, 54.0, 0.0, std::f64::consts::TAU);
            ui::set(cr, p.hud);
            let _ = cr.fill_preserve();
            ui::set(cr, p.rim);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let n = remaining.get().to_string();
            let (w, h) = ui::text_size(cr, &p, &n, 48.0, true);
            ui::set(cr, p.label(ui::PRIMARY));
            ui::text(cr, &p, &n, 60.0 - w / 2.0, 54.0 - h / 2.0, 48.0, true);
            let hint = "click to cancel";
            let (w, _) = ui::text_size(cr, &p, hint, 10.0, false);
            ui::set(cr, p.label(ui::SECONDARY));
            ui::text(cr, &p, hint, 60.0 - w / 2.0, 82.0, 10.0, false);
            glib::Propagation::Stop
        });
    }
    window.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);
    window.connect_button_press_event(|_, _| std::process::exit(0));
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

pub fn timestamp_name(prefix: &str, ext: &str) -> String {
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
