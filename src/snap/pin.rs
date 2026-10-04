//! Pin an image to the screen: a borderless, always-on-top floating window.
//!
//! - drag to move · scroll to zoom · Ctrl+scroll for opacity
//! - double-click or Esc to close · right-click for Copy / Save / Edit / Close

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk;
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::glib;
use gtk::prelude::*;

pub fn run(path: &Path) {
    let Ok(original) = Pixbuf::from_file(path) else {
        eprintln!("rustcast pin: cannot load {}", path.display());
        return;
    };
    let path = path.to_path_buf();
    let (iw, ih) = (f64::from(original.width()), f64::from(original.height()));

    // Start at a comfortable size: at most half the monitor.
    let (mw, mh) = gdk::Display::default()
        .and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)))
        .map(|m| {
            (
                f64::from(m.geometry().width()),
                f64::from(m.geometry().height()),
            )
        })
        .unwrap_or((1920.0, 1080.0));
    let zoom = Rc::new(Cell::new((mw * 0.5 / iw).min(mh * 0.5 / ih).min(1.0)));
    let opacity = Rc::new(Cell::new(1.0f64));

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Pinned — RustCast");
    window.set_decorated(false);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_type_hint(gdk::WindowTypeHint::Utility);
    window.stick();

    let image = gtk::Image::new();
    let evbox = gtk::EventBox::new();
    evbox.add(&image);
    window.add(&evbox);

    let apply_zoom = {
        let (image, window, original, zoom) = (
            image.clone(),
            window.clone(),
            original.clone(),
            zoom.clone(),
        );
        move || {
            let z = zoom.get();
            let (w, h) = (((iw * z) as i32).max(24), ((ih * z) as i32).max(24));
            if let Some(scaled) = original.scale_simple(w, h, InterpType::Bilinear) {
                image.set_from_pixbuf(Some(&scaled));
            }
            window.resize(w, h);
        }
    };
    apply_zoom();

    evbox.add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
    {
        let (zoom, opacity, window) = (zoom.clone(), opacity.clone(), window.clone());
        let apply_zoom = apply_zoom.clone();
        evbox.connect_scroll_event(move |_, ev| {
            let up = match ev.direction() {
                gdk::ScrollDirection::Up => true,
                gdk::ScrollDirection::Down => false,
                _ => ev.delta().1 < 0.0,
            };
            if ev.state().contains(gdk::ModifierType::CONTROL_MASK) {
                let o = (opacity.get() + if up { 0.1 } else { -0.1 }).clamp(0.2, 1.0);
                opacity.set(o);
                window.set_opacity(o);
            } else {
                let z = (zoom.get() * if up { 1.1 } else { 1.0 / 1.1 }).clamp(0.05, 8.0);
                zoom.set(z);
                apply_zoom();
            }
            glib::Propagation::Stop
        });
    }

    {
        let window = window.clone();
        let path = path.clone();
        evbox.connect_button_press_event(move |_, ev| {
            match (ev.button(), ev.event_type()) {
                (1, gdk::EventType::DoubleButtonPress) => window.close(),
                (1, _) => {
                    let (x, y) = ev.root();
                    window.begin_move_drag(1, x as i32, y as i32, ev.time());
                }
                (3, _) => context_menu(&window, &path, ev),
                _ => {}
            }
            glib::Propagation::Stop
        });
    }

    window.connect_key_press_event(|w, ev| {
        if ev.keyval() == gdk::keys::constants::Escape {
            w.close();
        }
        glib::Propagation::Proceed
    });
    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    window.present();
    gtk::main();
}

fn context_menu(window: &gtk::Window, path: &Path, ev: &gdk::EventButton) {
    let menu = gtk::Menu::new();
    let add = |label: &str, f: Box<dyn Fn()>| {
        let item = gtk::MenuItem::with_label(label);
        item.connect_activate(move |_| f());
        menu.append(&item);
    };
    let p: PathBuf = path.to_path_buf();
    add("Copy", {
        let p = p.clone();
        Box::new(move || super::copy_image(&p))
    });
    add("Annotate…", {
        let p = p.clone();
        Box::new(move || super::spawn(&["edit", &p.to_string_lossy()]))
    });
    add("Copy Text (OCR)", {
        let p = p.clone();
        Box::new(move || super::spawn(&["ocr-file", &p.to_string_lossy()]))
    });
    add("Show in Folder", {
        let p = p.clone();
        Box::new(move || {
            if let Some(dir) = p.parent() {
                super::open_url(&dir.to_string_lossy());
            }
        })
    });
    menu.append(&gtk::SeparatorMenuItem::new());
    add("Close", {
        let window = window.clone();
        Box::new(move || window.close())
    });
    menu.show_all();
    menu.popup_at_pointer(Some(ev));
}
