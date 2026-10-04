//! The OCR flow around [`super::ocr`]: recognise off the GTK thread behind a
//! small "Reading text…" pill, copy the result, then show it in a RustCast
//! panel with a Text / Code / Table switch, smart actions (links, e-mail,
//! phone numbers, colours, sums), QR codes, Translate and Search.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use image::RgbaImage;

use super::ocr::{self, Layout, Recognition, Smart};
use super::ui;
use crate::config::ScreenshotConfig;

/// `ocr-file` mode: read the text of an image on disk.
pub fn run_file(path: &Path, cfg: ScreenshotConfig, layout: Option<Layout>) {
    match image::open(path) {
        Ok(img) => run_on_image(img.into_rgba8(), cfg, layout),
        Err(e) => super::error_dialog(&format!("Cannot open {}: {e}", path.display())),
    }
}

/// Recognise `img` off the GTK thread, copy the result and show it.
/// `layout` forces Text / Code / Table; `None` picks the best fit.
/// Blocks (runs a GTK main loop) until the result window is closed.
pub fn run_on_image(img: RgbaImage, cfg: ScreenshotConfig, layout: Option<Layout>) {
    let (tx, rx) = mpsc::channel();
    let langs = cfg.ocr_languages.clone();
    std::thread::spawn(move || {
        let codes = super::qr::decode(&img);
        let rec = ocr::recognize(&img, &langs, layout);
        drop(img);
        let _ = tx.send((rec, codes));
    });

    let busy = busy_window("Reading text…");
    let result = wait_for(rx);
    busy.close();
    super::flush_gtk();

    let Some((rec, codes)) = result else {
        return;
    };
    show_result(rec, codes, cfg, layout);
}

/// Run a GTK loop until `rx` delivers (or its sender is dropped).
pub fn wait_for<T: 'static>(rx: mpsc::Receiver<T>) -> Option<T> {
    let main_loop = glib::MainLoop::new(None, false);
    let out = Rc::new(RefCell::new(None));
    {
        let (main_loop, out) = (main_loop.clone(), out.clone());
        glib::timeout_add_local(Duration::from_millis(30), move || match rx.try_recv() {
            Ok(v) => {
                *out.borrow_mut() = Some(v);
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
    out.borrow_mut().take()
}

/// A small "working…" pill in the middle of the screen, in RustCast's material.
pub fn busy_window(label: &str) -> gtk::Window {
    let p = ui::Palette::load();
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.set_default_size(220, 56);
    window.set_position(gtk::WindowPosition::Center);
    let label = label.to_string();
    window.connect_draw(move |w, cr| {
        cr.set_operator(gtk::cairo::Operator::Source);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = cr.paint();
        cr.set_operator(gtk::cairo::Operator::Over);
        let (aw, ah) = (
            f64::from(w.allocated_width()),
            f64::from(w.allocated_height()),
        );
        ui::pill(cr, &p, &label, aw / 2.0, ah / 2.0, 14.0, true);
        glib::Propagation::Stop
    });
    window.show_all();
    super::flush_gtk();
    window
}

struct State {
    rec: Recognition,
    layout: Layout,
    table: Vec<Vec<String>>,
    tabular: bool,
}

fn show_result(
    rec: Result<Recognition, String>,
    codes: Vec<String>,
    cfg: ScreenshotConfig,
    requested: Option<Layout>,
) {
    let (rec, error) = match rec {
        Ok(r) => (r, None),
        Err(e) => (Recognition::default(), Some(e)),
    };
    let empty = rec.is_empty();
    let layout = requested.unwrap_or_else(|| rec.guess_layout());
    let table = rec.table();
    let initial = rec.render(layout);

    let (window, header) = ui::panel_window("Text from Screen", None);
    window.set_default_size(620, 440);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(4);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    window.add(&root);

    // Content: text view (Text / Code) or a grid (Table).
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(10);
    view.set_right_margin(10);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    let text_scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    text_scroll.add(&view);
    let grid = gtk::Grid::new();
    let grid_scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    grid_scroll.add(&grid);
    let message = gtk::Label::new(None);
    message.style_context().add_class("dim");
    message.set_line_wrap(true);
    message.set_justify(gtk::Justification::Center);
    let stack = gtk::Stack::new();
    stack.add_named(&text_scroll, "text");
    stack.add_named(&grid_scroll, "table");
    stack.add_named(&message, "message");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.style_context().add_class("card");
    card.pack_start(&stack, true, true, 0);

    let tabular = rec.looks_like_table();
    let state = Rc::new(RefCell::new(State {
        rec,
        layout,
        table,
        tabular,
    }));

    // Top row: layout switch + hint.
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let hint = gtk::Label::new(None);
    hint.style_context().add_class("dim");
    hint.set_xalign(1.0);
    let save_csv = ui::button("Save CSV…", false);

    let show = {
        let (state, view, grid, stack, hint, save_csv) = (
            state.clone(),
            view.clone(),
            grid.clone(),
            stack.clone(),
            hint.clone(),
            save_csv.clone(),
        );
        move || {
            let st = state.borrow();
            save_csv.set_visible(st.layout == Layout::Table);
            match st.layout {
                Layout::Table => {
                    fill_grid(&grid, &st.table);
                    let cols = st.table.first().map(Vec::len).unwrap_or(0);
                    hint.set_text(&if cols < 2 {
                        "No columns found — try Text".to_string()
                    } else {
                        format!(
                            "{} rows × {cols} columns · pastes into spreadsheets",
                            st.table.len()
                        )
                    });
                    stack.set_visible_child_name("table");
                }
                layout => {
                    let ctx = view.style_context();
                    if layout == Layout::Code {
                        ctx.add_class("mono");
                        view.set_wrap_mode(gtk::WrapMode::None);
                        hint.set_text("Indentation kept");
                    } else {
                        ctx.remove_class("mono");
                        view.set_wrap_mode(gtk::WrapMode::WordChar);
                        hint.set_text(if st.tabular {
                            "Looks like a table — try Table"
                        } else {
                            ""
                        });
                    }
                    if let Some(b) = view.buffer() {
                        b.set_text(&st.rec.render(layout));
                    }
                    stack.set_visible_child_name("text");
                }
            }
        }
    };

    // What Copy puts on the clipboard for the current layout.
    let current = {
        let (state, view) = (state.clone(), view.clone());
        move || -> String {
            let st = state.borrow();
            if st.layout == Layout::Table {
                return ocr::to_tsv(&st.table);
            }
            view.buffer()
                .and_then(|b| b.text(&b.start_iter(), &b.end_iter(), false))
                .map(|s| s.to_string())
                .unwrap_or_default()
        }
    };

    let switch_ready = Rc::new(Cell::new(false));
    let (segments, _) = {
        let (state, show, current, header, switch_ready) = (
            state.clone(),
            show.clone(),
            current.clone(),
            header.clone(),
            switch_ready.clone(),
        );
        ui::segmented(
            &["Text", "Code", "Table"],
            match layout {
                Layout::Text => 0,
                Layout::Code => 1,
                Layout::Table => 2,
            },
            move |i| {
                if !switch_ready.get() {
                    return;
                }
                let l = [Layout::Text, Layout::Code, Layout::Table][i];
                state.borrow_mut().layout = l;
                show();
                let text = current();
                if !text.trim().is_empty() {
                    super::copy_text(&text);
                    header.set_subtitle(Some(match l {
                        Layout::Text => "Copied as text",
                        Layout::Code => "Copied as code",
                        Layout::Table => "Copied as table",
                    }));
                }
            },
        )
    };
    switch_ready.set(true);
    top.pack_start(&segments, false, false, 0);
    top.pack_end(&hint, true, true, 0);
    root.pack_start(&top, false, false, 0);
    root.pack_start(&card, true, true, 0);

    // Smart actions and QR codes.
    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_column_spacing(6);
    chips.set_row_spacing(6);
    chips.set_max_children_per_line(6);
    let mut chip_count = 0;
    let smart = if error.is_none() {
        ocr::smart_actions(&state.borrow().rec.text())
    } else {
        Vec::new()
    };
    for s in smart {
        chips.add(&smart_chip(&s, &header));
        chip_count += 1;
    }
    for code in &codes {
        chips.add(&qr_chip(code, &header));
        chip_count += 1;
    }
    if chip_count > 0 {
        root.pack_start(&chips, false, false, 0);
    }

    // Bottom buttons.
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let target = cfg.translate_target();
    let translate = ui::button(&format!("Translate → {target}"), false);
    let search = ui::button("Search", false);
    let copy = ui::button("Copy", true);
    bottom.pack_start(&translate, false, false, 0);
    bottom.pack_start(&search, false, false, 0);
    bottom.pack_end(&copy, false, false, 0);
    bottom.pack_end(&save_csv, false, false, 0);
    root.pack_start(&bottom, false, false, 0);

    {
        let (current, header) = (current.clone(), header.clone());
        copy.connect_clicked(move |_| {
            let t = current();
            if !t.trim().is_empty() {
                super::copy_text(&t);
                header.set_subtitle(Some("Copied to the clipboard"));
            }
        });
    }
    {
        let (state, header, window) = (state.clone(), header.clone(), window.clone());
        let dir = cfg.save_dir();
        save_csv.connect_clicked(move |_| {
            let csv = ocr::to_csv(&state.borrow().table);
            let dialog = gtk::FileChooserNative::new(
                Some("Save Table"),
                Some(&window),
                gtk::FileChooserAction::Save,
                Some("Save"),
                Some("Cancel"),
            );
            dialog.set_do_overwrite_confirmation(true);
            let _ = std::fs::create_dir_all(&dir);
            dialog.set_current_folder(&dir);
            dialog.set_current_name("table.csv");
            if dialog.run() == gtk::ResponseType::Accept
                && let Some(path) = dialog.filename()
            {
                let path = if path.extension().is_none() {
                    path.with_extension("csv")
                } else {
                    path
                };
                match std::fs::write(&path, csv) {
                    Ok(()) => header.set_subtitle(Some(&format!(
                        "Saved {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ))),
                    Err(e) => header.set_subtitle(Some(&format!("Could not save: {e}"))),
                }
            }
        });
    }
    {
        let (current, header, view, stack) =
            (current.clone(), header.clone(), view.clone(), stack.clone());
        translate.connect_clicked(move |btn| {
            let text = current();
            if text.trim().is_empty() {
                return;
            }
            // translate-shell translates in place; otherwise use the browser.
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
            header.set_subtitle(Some("Translating…"));
            let (tx, rx) = mpsc::channel();
            let target = target.clone();
            std::thread::spawn(move || {
                let _ = tx.send(translate_shell(&text, &target));
            });
            let (header, view, btn, stack) =
                (header.clone(), view.clone(), btn.clone(), stack.clone());
            glib::timeout_add_local(Duration::from_millis(60), move || match rx.try_recv() {
                Ok(result) => {
                    btn.set_sensitive(true);
                    match result {
                        Ok(t) => {
                            // Translations are prose: show them in the text view.
                            if let Some(b) = view.buffer() {
                                b.set_text(&t);
                            }
                            stack.set_visible_child_name("text");
                            header.set_subtitle(Some("Translated and copied"));
                            super::copy_text(&t);
                        }
                        Err(e) => header.set_subtitle(Some(&format!("Translation failed: {e}"))),
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    btn.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            });
        });
    }
    {
        let current = current.clone();
        let search_url = super::load_config().search_url;
        search.connect_clicked(move |_| {
            let q: String = current().split_whitespace().collect::<Vec<_>>().join(" ");
            let q: String = q.chars().take(300).collect();
            if q.is_empty() {
                return;
            }
            let q = url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>();
            super::open_url(&search_url.replace("%s", &q));
        });
    }

    window.show_all();
    show();

    // Initial state: what was copied, or why nothing was.
    if let Some(err) = &error {
        header.set_title(Some("Couldn't Read Text"));
        message.set_text(err);
        message.set_selectable(true);
        message.set_justify(gtk::Justification::Left);
        message.set_xalign(0.0);
        message.set_margin_start(24);
        stack.set_visible_child_name("message");
        segments.set_visible(false);
        hint.set_visible(false);
        translate.set_visible(false);
        search.set_visible(false);
        save_csv.set_visible(false);
        copy.set_label("Copy Install Command");
        copy.connect_clicked(|_| super::copy_text("sudo apt install tesseract-ocr"));
    } else if empty {
        segments.set_visible(false);
        hint.set_visible(false);
        translate.set_sensitive(false);
        search.set_sensitive(false);
        copy.set_sensitive(false);
        save_csv.set_visible(false);
        if let Some(first) = codes.first() {
            super::copy_text(first);
            header.set_subtitle(Some("QR code copied"));
            message.set_text("No text in the selection — the QR code was copied instead.");
        } else {
            header.set_subtitle(Some("Nothing copied"));
            message.set_text(
                "No text found in the selection.\nTry a larger area, or zoom in on small text first.",
            );
        }
        stack.set_visible_child_name("message");
    } else {
        super::copy_text(&initial);
        let chars = initial.chars().filter(|c| !c.is_whitespace()).count();
        header.set_subtitle(Some(&match layout {
            Layout::Text => format!("Copied {chars} characters"),
            Layout::Code => "Copied as code".to_string(),
            Layout::Table => "Copied as table".to_string(),
        }));
    }

    window.connect_destroy(|_| gtk::main_quit());
    window.present();
    gtk::main();
    if !super::main_instance_running() {
        super::linger_for_clipboard();
    }
}

fn fill_grid(grid: &gtk::Grid, rows: &[Vec<String>]) {
    for child in grid.children() {
        grid.remove(&child);
    }
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let l = gtk::Label::new(Some(cell));
            l.set_xalign(0.0);
            l.set_selectable(true);
            let ctx = l.style_context();
            ctx.add_class("cell");
            if r == 0 {
                ctx.add_class("head");
            }
            grid.attach(&l, c as i32, r as i32, 1, 1);
        }
    }
    grid.show_all();
}

fn chip(label: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.style_context().add_class("chip");
    b.set_tooltip_text(Some(tooltip));
    if let Some(l) = b.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        l.set_max_width_chars(32);
    }
    b
}

fn smart_chip(s: &Smart, header: &gtk::HeaderBar) -> gtk::Button {
    let header = header.clone();
    let copied = move |what: &str, value: &str| {
        super::copy_text(value);
        header.set_subtitle(Some(&format!("Copied {what}")));
    };
    match s.clone() {
        Smart::Link(url) => {
            let shown = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_string();
            let b = chip(&format!("↗  {shown}"), "Open link");
            b.connect_clicked(move |_| super::open_url(&url));
            b
        }
        Smart::Email(mail) => {
            let b = chip(&format!("✉  {mail}"), "Write an e-mail");
            b.connect_clicked(move |_| super::open_url(&format!("mailto:{mail}")));
            b
        }
        Smart::Phone(phone) => {
            let b = chip(&format!("☎  {phone}"), "Copy phone number");
            b.connect_clicked(move |_| copied("phone number", &phone));
            b
        }
        Smart::Color(hex) => {
            let b = chip(&format!("●  {hex}"), "Copy colour");
            if let Some(l) = b.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                l.set_markup(&format!(
                    "<span foreground=\"{hex}\">●</span>  {}",
                    glib::markup_escape_text(&hex)
                ));
            }
            b.connect_clicked(move |_| copied("colour", &hex));
            b
        }
        Smart::Math(expr, value) => {
            let b = chip(
                &format!("=  {value}"),
                &format!("{expr} = {value} — copy result"),
            );
            b.connect_clicked(move |_| copied("result", &value));
            b
        }
    }
}

fn qr_chip(code: &str, header: &gtk::HeaderBar) -> gtk::Button {
    let is_url = code.starts_with("http://") || code.starts_with("https://");
    let b = chip(
        &format!("QR  {code}"),
        if is_url {
            "Open QR link"
        } else {
            "Copy QR content"
        },
    );
    let (code, header) = (code.to_string(), header.clone());
    b.connect_clicked(move |_| {
        if is_url {
            super::open_url(&code);
        } else {
            super::copy_text(&code);
            header.set_subtitle(Some("Copied QR code"));
        }
    });
    b
}

fn has_translate_shell() -> bool {
    std::process::Command::new("trans")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn translate_shell(text: &str, target: &str) -> Result<String, String> {
    let out = std::process::Command::new("trans")
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
