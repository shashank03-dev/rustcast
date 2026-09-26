//! GTK screenshot thumbnail window with a native drag source.
//!
//! Runs in the `--overlay` subprocess (see [`super::overlay`]). Using GTK on the
//! Wayland backend means the drag is a real Wayland drag-and-drop, which the
//! compositor delivers to Wayland *and* XWayland apps alike — so the screenshot
//! can be dropped into terminals, browsers, editors, chat apps, anything.
//!
//! Interaction:
//! - drag the thumbnail → drops `text/uri-list` (a `file://` path) + `image/png`
//! - right-click → copies the file path as text (for terminal programs that
//!   load images by path)
//! - auto-dismisses after a timeout

use std::path::PathBuf;
use std::time::Duration;

use gtk::gdk;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::glib;
use gtk::prelude::*;

const THUMB_MAX: i32 = 200;
const MARGIN: i32 = 28;
/// Diagonal offset per already-open screenshot, so multiple thumbnails cascade
/// up-and-right from the bottom-left corner instead of overlapping exactly.
const STACK_OFFSET: i32 = 26;
const MAX_STACK: i32 = 8;
const AUTO_DISMISS: Duration = Duration::from_secs(8);

/// How many *other* overlay subprocesses are currently alive. New thumbnails
/// offset by this so screenshots taken in quick succession stack visibly.
/// Process-based so it self-cleans when a thumbnail dismisses or crashes.
fn stack_index() -> i32 {
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return 0;
    };
    let mut count = 0;
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == me {
            continue;
        }
        if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
            let s = String::from_utf8_lossy(&cmdline);
            if s.contains("--overlay") && s.to_ascii_lowercase().contains("rustcast") {
                count += 1;
            }
        }
    }
    count.min(MAX_STACK)
}

pub fn run(path: PathBuf) {
    if gtk::init().is_err() {
        eprintln!("rustcast overlay: GTK init failed");
        return;
    }

    let Ok(pixbuf) = Pixbuf::from_file_at_scale(&path, THUMB_MAX, THUMB_MAX, true) else {
        eprintln!("rustcast overlay: cannot load {}", path.display());
        return;
    };
    let (w, h) = (pixbuf.width(), pixbuf.height());

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_type_hint(gdk::WindowTypeHint::Utility);
    window.set_default_size(w, h);
    window.set_size_request(w, h);
    // Bottom-left placement, cascading up-and-right for each screenshot already
    // on screen. Requires X11 (the overlay is forced onto XWayland) — GNOME
    // Wayland would ignore client positioning.
    window.set_gravity(gdk::Gravity::SouthWest);
    if let Some(monitor) = gdk::Display::default()
        .and_then(|d| d.primary_monitor())
        .map(|m| m.geometry())
    {
        let offset = stack_index() * STACK_OFFSET;
        let x = monitor.x() + MARGIN + offset;
        let y = monitor.y() + monitor.height() - h - MARGIN - offset;
        window.move_(x, y);
    }

    let image = gtk::Image::from_pixbuf(Some(&pixbuf));
    let evbox = gtk::EventBox::new();
    evbox.add(&image);
    window.add(&evbox);

    let targets = [
        gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::OTHER_APP, 0),
        gtk::TargetEntry::new("image/png", gtk::TargetFlags::OTHER_APP, 1),
    ];
    evbox.drag_source_set(
        gdk::ModifierType::BUTTON1_MASK,
        &targets,
        gdk::DragAction::COPY,
    );
    evbox.drag_source_set_icon_pixbuf(&pixbuf);

    // Provide the dragged data on request.
    {
        let path = path.clone();
        evbox.connect_drag_data_get(move |_w, _ctx, sel, info, _time| match info {
            0 => {
                let uri = format!("file://{}", path.display());
                let _ = sel.set_uris(&[&uri]);
            }
            1 => {
                if let Ok(bytes) = std::fs::read(&path) {
                    sel.set(&gdk::Atom::intern("image/png"), 8, &bytes);
                }
            }
            _ => {}
        });
    }

    // Close once the drag finishes.
    {
        let window = window.clone();
        evbox.connect_drag_end(move |_w, _ctx| window.close());
    }

    // Right-click → copy the file path as clipboard text, then dismiss shortly
    // after (giving the clipboard manager time to take ownership).
    {
        let path = path.clone();
        let window = window.clone();
        evbox.connect_button_press_event(move |_w, ev| {
            if ev.button() == 3 {
                let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
                clipboard.set_text(&path.display().to_string());
                clipboard.store();
                let window = window.clone();
                glib::timeout_add_local_once(Duration::from_millis(200), move || window.close());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    // Auto-dismiss.
    {
        let window = window.clone();
        glib::timeout_add_local_once(AUTO_DISMISS, move || window.close());
    }

    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    gtk::main();
}
