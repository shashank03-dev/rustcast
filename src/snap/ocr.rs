//! OCR: read the text inside a screenshot.
//!
//! Engine: the system `tesseract` binary, run as a short-lived child process.
//! Nothing is loaded into RustCast itself — the engine and its language model
//! only occupy memory for the fraction of a second the recognition takes and
//! are released as soon as it exits. That keeps the launcher's footprint at
//! zero for this feature, which is what low-end machines need.
//!
//! The crop is prepared so Tesseract does well on screen text (which is far
//! smaller and lower-DPI than the scans it is tuned for):
//! - converted to 8-bit grayscale (¼ of the RGBA size) and sent as PGM over a
//!   pipe — no PNG encoding, no temporary files;
//! - light-on-dark UI (dark themes, terminals) is inverted to dark-on-light;
//! - small crops are upscaled 2–3× so glyphs reach a size the model expects,
//!   with a cap so large crops never balloon in memory;
//! - a white margin is added (Tesseract misses text touching the border);
//! - `OMP_THREAD_LIMIT=1`: on small images threads only add memory and
//!   start-up cost.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use image::{GrayImage, RgbaImage};

use crate::config::ScreenshotConfig;

/// Largest image (in pixels) handed to the engine after upscaling.
const MAX_PIXELS: u32 = 6_000_000;
const MARGIN: u32 = 12;

pub fn tesseract_installed() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Installed Tesseract language packs (e.g. `["eng", "hin", "osd"]`).
pub fn installed_languages() -> Vec<String> {
    Command::new("tesseract")
        .arg("--list-langs")
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && l != "osd")
                .collect()
        })
        .unwrap_or_default()
}

/// Keep only the requested languages that are installed; fall back to English
/// (or whatever is installed) so a typo in the config never breaks OCR.
fn pick_languages(requested: &str, installed: &[String]) -> String {
    let ok: Vec<&str> = requested
        .split('+')
        .map(str::trim)
        .filter(|l| installed.iter().any(|i| i == l))
        .collect();
    if !ok.is_empty() {
        return ok.join("+");
    }
    if installed.iter().any(|l| l == "eng") {
        return "eng".to_string();
    }
    installed
        .first()
        .cloned()
        .unwrap_or_else(|| "eng".to_string())
}

/// Grayscale, auto-invert, upscale and pad an image for recognition.
pub fn prepare(img: &RgbaImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut gray = GrayImage::new(w, h);
    let mut sum: u64 = 0;
    for (src, dst) in img.pixels().zip(gray.pixels_mut()) {
        let [r, g, b, _] = src.0;
        let l = ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8) as u8;
        sum += u64::from(l);
        dst.0[0] = l;
    }
    let mean = sum / u64::from((w * h).max(1));
    if mean < 110 {
        for p in gray.pixels_mut() {
            p.0[0] = 255 - p.0[0];
        }
    }

    // Screen text is ~10–16 px tall; Tesseract is happiest around 30+ px.
    let mut factor = if h <= 120 { 3 } else { 2 };
    while factor > 1 && (w * factor) * (h * factor) > MAX_PIXELS {
        factor -= 1;
    }
    let scaled = if factor > 1 {
        image::imageops::resize(
            &gray,
            w * factor,
            h * factor,
            image::imageops::FilterType::Triangle,
        )
    } else {
        gray
    };

    let (sw, sh) = scaled.dimensions();
    let mut padded = GrayImage::from_pixel(sw + 2 * MARGIN, sh + 2 * MARGIN, image::Luma([255]));
    image::imageops::replace(&mut padded, &scaled, i64::from(MARGIN), i64::from(MARGIN));
    padded
}

fn to_pgm(img: &GrayImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(img.as_raw());
    out
}

/// Recognise the text in `img`. `languages` is a Tesseract language string
/// such as `eng` or `eng+hin`.
pub fn recognize(img: &RgbaImage, languages: &str) -> Result<String, String> {
    if !tesseract_installed() {
        return Err(missing_engine_message());
    }
    let langs = pick_languages(languages, &installed_languages());
    let pgm = to_pgm(&prepare(img));

    let run = |psm: &str| -> Result<String, String> {
        let mut child = Command::new("tesseract")
            .args([
                "stdin", "stdout", "-l", &langs, "--psm", psm, "--dpi", "300",
            ])
            .env("OMP_THREAD_LIMIT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&pgm).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(clean(&String::from_utf8_lossy(&out.stdout)))
    };

    // Automatic page segmentation handles columns and mixed layouts; a single
    // text block is the better guess when that finds nothing (tiny crops).
    let text = run("3")?;
    if text.is_empty() { run("6") } else { Ok(text) }
}

/// Tidy Tesseract output: drop form feeds, trailing spaces and runs of blank lines.
fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut blank = 0;
    for line in raw.replace('\u{c}', "").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

pub fn missing_engine_message() -> String {
    "Text recognition needs Tesseract, which is not installed.\n\n\
     Ubuntu / Debian:  sudo apt install tesseract-ocr\n\
     Fedora:           sudo dnf install tesseract\n\
     Arch:             sudo pacman -S tesseract tesseract-data-eng\n\n\
     Extra languages, e.g. Hindi:  sudo apt install tesseract-ocr-hin\n\
     then set ocr_languages = \"eng+hin\" under [screenshot] in the config."
        .to_string()
}

/// `ocr-file` mode: read the text of an image on disk.
pub fn run_file(path: &Path, cfg: ScreenshotConfig) {
    match image::open(path) {
        Ok(img) => run_on_image(img.into_rgba8(), cfg),
        Err(e) => eprintln!("rustcast ocr: cannot open {}: {e}", path.display()),
    }
}

/// Recognise `img` off the GTK thread, copy the result and show it.
/// Blocks (runs a GTK main loop) until the result window is closed.
pub fn run_on_image(img: RgbaImage, cfg: ScreenshotConfig) {
    let (tx, rx) = mpsc::channel();
    let langs = cfg.ocr_languages.clone();
    std::thread::spawn(move || {
        let codes = super::qr::decode(&img);
        let text = recognize(&img, &langs);
        drop(img);
        let _ = tx.send((text, codes));
    });

    let spinner = busy_window("Reading text…");
    let main_loop = glib::MainLoop::new(None, false);
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    {
        let main_loop = main_loop.clone();
        let result = result.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || match rx.try_recv() {
            Ok(r) => {
                *result.borrow_mut() = Some(r);
                main_loop.quit();
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                main_loop.quit();
                glib::ControlFlow::Break
            }
        });
    }
    main_loop.run();
    spinner.close();

    let Some((text, codes)) = result.borrow_mut().take() else {
        return;
    };
    let (text, error) = match text {
        Ok(t) => (t, None),
        Err(e) => (String::new(), Some(e)),
    };
    if !text.is_empty() {
        super::copy_text(&text);
    } else if let Some(first) = codes.first() {
        super::copy_text(first);
    }
    show_result(text, codes, error, cfg);
}

/// A tiny "working…" pill in the middle of the screen.
pub fn busy_window(label: &str) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);
    let lbl = gtk::Label::new(Some(label));
    lbl.set_margin_top(14);
    lbl.set_margin_bottom(14);
    lbl.set_margin_start(22);
    lbl.set_margin_end(22);
    window.add(&lbl);
    window.show_all();
    super::flush_gtk();
    window
}

/// The result window: editable recognised text, QR codes, Copy / Translate / Search.
fn show_result(text: String, codes: Vec<String>, error: Option<String>, cfg: ScreenshotConfig) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Text from screen — RustCast");
    window.set_default_size(560, 380);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);
    window.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_line_wrap(true);
    let chars = text.chars().count();
    status.set_markup(&match (&error, chars, codes.is_empty()) {
        (Some(_), _, true) => "<b>Couldn't read text</b>".to_string(),
        (_, 0, true) => "<b>No text found</b> in the selection".to_string(),
        (_, 0, false) => "<b>QR code copied</b> to the clipboard".to_string(),
        _ => format!("<b>Copied</b> {chars} characters to the clipboard"),
    });
    root.pack_start(&status, false, false, 0);

    for code in &codes {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::new(Some(&format!("QR: {code}")));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_selectable(true);
        row.pack_start(&label, true, true, 0);
        let copy = gtk::Button::with_label("Copy");
        {
            let code = code.clone();
            copy.connect_clicked(move |_| super::copy_text(&code));
        }
        row.pack_end(&copy, false, false, 0);
        if code.starts_with("http://") || code.starts_with("https://") {
            let open = gtk::Button::with_label("Open");
            let code = code.clone();
            open.connect_clicked(move |_| super::open_url(&code));
            row.pack_end(&open, false, false, 0);
        }
        root.pack_start(&row, false, false, 0);
    }

    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    if let Some(buffer) = view.buffer() {
        buffer.set_text(error.as_deref().unwrap_or(&text));
    }
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_shadow_type(gtk::ShadowType::In);
    scroll.add(&view);
    root.pack_start(&scroll, true, true, 0);

    let current_text = {
        let view = view.clone();
        move || -> String {
            view.buffer()
                .and_then(|b| b.text(&b.start_iter(), &b.end_iter(), false))
                .map(|s| s.to_string())
                .unwrap_or_default()
        }
    };

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let target = cfg.translate_target();
    let copy = gtk::Button::with_label("Copy");
    let translate = gtk::Button::with_label(&format!("Translate → {target}"));
    let search = gtk::Button::with_label("Search");
    let close = gtk::Button::with_label("Close");
    buttons.pack_start(&copy, false, false, 0);
    buttons.pack_start(&translate, false, false, 0);
    buttons.pack_start(&search, false, false, 0);
    buttons.pack_end(&close, false, false, 0);
    root.pack_start(&buttons, false, false, 0);
    window.add(&root);

    {
        let current_text = current_text.clone();
        let status = status.clone();
        copy.connect_clicked(move |_| {
            super::copy_text(&current_text());
            status.set_markup("<b>Copied</b> to the clipboard");
        });
    }
    {
        let current_text = current_text.clone();
        let status = status.clone();
        let view = view.clone();
        translate.connect_clicked(move |btn| {
            let text = current_text();
            if text.trim().is_empty() {
                return;
            }
            // Offline-friendly path: translate-shell prints the translation
            // in-place. Without it, hand off to the browser.
            if !has_translate_shell() {
                let url = format!(
                    "https://translate.google.com/?sl=auto&tl={}&op=translate&text={}",
                    target,
                    url::form_urlencoded::byte_serialize(
                        text.chars().take(5000).collect::<String>().as_bytes()
                    )
                    .collect::<String>()
                );
                super::open_url(&url);
                return;
            }
            btn.set_sensitive(false);
            status.set_markup("<b>Translating…</b>");
            let (tx, rx) = mpsc::channel();
            let target = target.clone();
            std::thread::spawn(move || {
                let _ = tx.send(translate_shell(&text, &target));
            });
            let (status, view, btn) = (status.clone(), view.clone(), btn.clone());
            glib::timeout_add_local(Duration::from_millis(60), move || match rx.try_recv() {
                Ok(result) => {
                    btn.set_sensitive(true);
                    match result {
                        Ok(t) => {
                            if let Some(b) = view.buffer() {
                                b.set_text(&t);
                            }
                            super::copy_text(&t);
                            status.set_markup("<b>Translated</b> and copied to the clipboard");
                        }
                        Err(e) => status.set_text(&format!("Translation failed: {e}")),
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            });
        });
    }
    {
        let current_text = current_text.clone();
        let search_url = super::load_config().search_url;
        search.connect_clicked(move |_| {
            let q: String = current_text().chars().take(300).collect();
            let q = url::form_urlencoded::byte_serialize(q.trim().as_bytes()).collect::<String>();
            super::open_url(&search_url.replace("%s", &q));
        });
    }
    {
        let window = window.clone();
        close.connect_clicked(move |_| window.close());
    }
    window.connect_key_press_event(|w, ev| {
        if ev.keyval() == gtk::gdk::keys::constants::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });

    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    window.present();
    gtk::main();
    if !super::main_instance_running() {
        super::linger_for_clipboard();
    }
}

fn has_translate_shell() -> bool {
    Command::new("trans")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn translate_shell(text: &str, target: &str) -> Result<String, String> {
    let out = Command::new("trans")
        .args([
            "-b",
            "-no-ansi",
            "-no-autocorrect",
            &format!(":{target}"),
            text,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if t.is_empty() {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    } else {
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_selection_falls_back_gracefully() {
        let installed = vec!["eng".to_string(), "hin".to_string()];
        assert_eq!(pick_languages("eng+hin", &installed), "eng+hin");
        assert_eq!(pick_languages("deu+hin", &installed), "hin");
        assert_eq!(pick_languages("xyz", &installed), "eng");
        assert_eq!(pick_languages("xyz", &["fra".to_string()]), "fra");
    }

    #[test]
    fn output_is_tidied() {
        assert_eq!(clean("Hello  \n\n\n\nWorld\n\u{c}"), "Hello\n\nWorld");
        assert_eq!(clean("a\nb\n"), "a\nb");
        assert_eq!(clean("   \n\n"), "");
    }

    #[test]
    fn dark_text_backgrounds_are_inverted_and_small_crops_upscaled() {
        // White text on black → mean is dark → inverted to mostly white.
        let img = RgbaImage::from_pixel(40, 20, image::Rgba([0, 0, 0, 255]));
        let g = prepare(&img);
        assert_eq!(g.dimensions(), (40 * 3 + 2 * MARGIN, 20 * 3 + 2 * MARGIN));
        assert!(g.pixels().all(|p| p.0[0] == 255));
    }

    #[test]
    fn huge_crops_are_not_upscaled() {
        let img = RgbaImage::from_pixel(3000, 2000, image::Rgba([255, 255, 255, 255]));
        let g = prepare(&img);
        assert_eq!(g.dimensions(), (3000 + 2 * MARGIN, 2000 + 2 * MARGIN));
    }
}
