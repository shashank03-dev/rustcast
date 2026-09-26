//! The floating "● REC 00:12" pill shown while recording.
//!
//! Like the screenshot thumbnail it runs as a short-lived GTK subprocess
//! (`rustcast --rec-indicator <label> <bg> <fg>`) on the X11 backend so it can
//! position itself. It is its own window, so it never appears in a locked-window
//! recording. Clicking "Stop" (or pressing the recorder hotkey again) ends the
//! recording; the parent notices the subprocess exiting.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use crate::config::Theme;

fn hex((r, g, b): (f32, f32, f32)) -> String {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// Spawn the indicator for a recording of `label`, styled like RustCast.
/// `can_add` shows the "＋ Add" button (locked-window recordings only).
pub fn spawn(label: &str, theme: &Theme, can_add: bool) -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    spawn_with(&exe, label, theme, can_add)
}

fn spawn_with(exe: &Path, label: &str, theme: &Theme, can_add: bool) -> Option<Child> {
    Command::new(exe)
        .arg("--rec-indicator")
        .arg(label)
        .arg(hex(theme.background_color))
        .arg(hex(theme.text_color))
        .arg(if can_add { "add" } else { "" })
        .env("GDK_BACKEND", "x11")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| log::warn!("failed to spawn recording indicator: {e}"))
        .ok()
}

fn format_elapsed(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Entry point of the `--rec-indicator` subprocess.
pub fn run(args: &[String]) {
    let label = args.first().cloned().unwrap_or_default();
    let bg = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "#000000".to_string());
    let fg = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "#f2f2f5".to_string());
    let can_add = args.get(3).is_some_and(|a| a == "add");

    if gtk::init().is_err() {
        eprintln!("rustcast indicator: GTK init failed");
        return;
    }

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("RustCast Recording");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_keep_above(true);
    window.stick();
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_accept_focus(false);
    window.set_type_hint(gdk::WindowTypeHint::Utility);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }

    // Same glass look as the launcher: theme background, hairline border in the
    // text colour, rounded corners.
    let css = format!(
        "window {{ background-color: transparent; }}
         .pill {{ background-color: alpha({bg}, 0.92); border: 1px solid alpha({fg}, 0.25);
                  border-radius: 14px; padding: 6px 8px 6px 12px; }}
         .dot {{ color: #f24336; font-size: 13px; }}
         .rec {{ color: {fg}; font-weight: bold; font-size: 12px; }}
         .time {{ color: {fg}; font-family: monospace; font-size: 12px; }}
         .label {{ color: alpha({fg}, 0.6); font-size: 11px; }}
         button {{ background: alpha({fg}, 0.08); color: {fg}; border: 1px solid alpha({fg}, 0.22);
                   border-radius: 9px; padding: 2px 10px; min-height: 0; box-shadow: none;
                   background-image: none; text-shadow: none; font-size: 11px; }}
         button:hover {{ background: alpha({fg}, 0.18); }}
         button.stop:hover {{ background: alpha(#f24336, 0.85); color: #ffffff; }}"
    );
    let provider = gtk::CssProvider::new();
    if provider.load_from_data(css.as_bytes()).is_ok()
        && let Some(screen) = gdk::Screen::default()
    {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.style_context().add_class("pill");
    let dot = gtk::Label::new(Some("●"));
    dot.style_context().add_class("dot");
    let rec = gtk::Label::new(Some("REC"));
    rec.style_context().add_class("rec");
    let time = gtk::Label::new(Some("00:00"));
    time.style_context().add_class("time");
    let mut short: String = label.chars().take(28).collect();
    if label.chars().count() > 28 {
        short.push('…');
    }
    let name = gtk::Label::new(Some(&short));
    name.style_context().add_class("label");
    let stop = gtk::Button::with_label("■ Stop");
    stop.set_can_focus(false);
    stop.style_context().add_class("stop");
    // "＋ Add" opens RustCast's recorder page to bring more windows in.
    let add = gtk::Button::with_label("＋ Add window");
    add.set_can_focus(false);
    add.set_tooltip_text(Some("Bring another window into this recording"));
    add.connect_clicked(|_| {
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe)
                .arg("rustcast://recorder-add")
                .env_remove("GDK_BACKEND")
                .spawn();
        }
    });

    row.pack_start(&dot, false, false, 0);
    row.pack_start(&rec, false, false, 0);
    row.pack_start(&time, false, false, 0);
    row.pack_start(&name, false, false, 0);
    if can_add {
        row.pack_start(&add, false, false, 0);
    }
    row.pack_start(&stop, false, false, 0);

    // The pill itself can be dragged anywhere.
    let evbox = gtk::EventBox::new();
    evbox.add(&row);
    {
        let window = window.clone();
        evbox.connect_button_press_event(move |_, ev| {
            if ev.button() == 1 {
                let (x, y) = ev.root();
                window.begin_move_drag(1, x as i32, y as i32, ev.time());
            }
            glib::Propagation::Proceed
        });
    }
    window.add(&evbox);

    stop.connect_clicked(|_| std::process::exit(0));

    // Blink the dot and tick the timer.
    let started = Instant::now();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let secs = started.elapsed().as_secs();
        time.set_text(&format_elapsed(secs));
        dot.set_opacity(if (started.elapsed().as_millis() / 500).is_multiple_of(2) {
            1.0
        } else {
            0.35
        });
        glib::ControlFlow::Continue
    });

    // Top-centre of the primary monitor.
    window.show_all();
    if let Some(monitor) = gdk::Display::default()
        .and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)))
        .map(|m| m.geometry())
    {
        let (w, _) = window.size();
        window.move_(monitor.x() + (monitor.width() - w) / 2, monitor.y() + 14);
    }

    window.connect_destroy(|_| gtk::main_quit());
    gtk::main();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_render_as_hex() {
        assert_eq!(hex((0.0, 0.0, 0.0)), "#000000");
        assert_eq!(hex((1.0, 0.5, 0.0)), "#ff8000");
    }

    #[test]
    fn elapsed_formats_minutes_and_hours() {
        assert_eq!(format_elapsed(5), "00:05");
        assert_eq!(format_elapsed(125), "02:05");
        assert_eq!(format_elapsed(3725), "1:02:05");
    }
}
