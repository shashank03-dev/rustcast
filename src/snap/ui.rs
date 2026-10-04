//! The look of every screenshot window, derived from the RustCast theme the
//! same way the launcher derives it (see `crate::styles`): the graphite /
//! near-white window material, label opacities, systemFill levels, the
//! systemBlue accent, 16 px window corners, SF Pro / Inter when installed.
//!
//! GTK widgets get it through one CSS provider ([`install_css`]); cairo-drawn
//! chrome (capture overlay, thumbnail) reads the [`Palette`] directly. The
//! vector [`Icon`]s are shared by the overlay toolbar and the thumbnail.

use std::f64::consts::{PI, TAU};

use gtk::cairo::{self, Context};
use gtk::prelude::*;

use super::beautify::rounded_rect;

pub type Rgba = (f64, f64, f64, f64);

/// Theme colours for screenshot UI.
#[derive(Debug, Clone)]
pub struct Palette {
    pub light: bool,
    /// Window material (opaque).
    pub window: Rgba,
    /// HUD material for floating bars over a screenshot (slightly translucent).
    pub hud: Rgba,
    pub text: (f64, f64, f64),
    pub accent: Rgba,
    pub rim: Rgba,
    pub font: String,
}

impl Palette {
    pub fn load() -> Self {
        Self::from_theme(&super::load_config().theme)
    }

    pub fn from_theme(theme: &crate::config::Theme) -> Self {
        let (text, bg) = match theme.theme_mode {
            crate::config::ThemeMode::System => {
                theme.theme_mode.presets(crate::platform::is_dark_mode())
            }
            _ => (theme.text_color, theme.background_color),
        };
        let f = |c: (f32, f32, f32)| (f64::from(c.0), f64::from(c.1), f64::from(c.2));
        let (text, bg) = (f(text), f(bg));
        let light = 0.2126 * bg.0 + 0.7152 * bg.1 + 0.0722 * bg.2 > 0.5;
        // Same formulas as `styles::window_fill`.
        let base = if light {
            let t = |c: f64| c + (1.0 - c) * 0.55;
            (t(bg.0), t(bg.1), t(bg.2))
        } else {
            let m = |a: f64, b: f64| a + (b - a) * 0.12;
            (m(bg.0, text.0), m(bg.1, text.1), m(bg.2, text.2))
        };
        let font = theme
            .font
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .or_else(|| {
                ["SF Pro Text", "SF Pro", "Inter", "Inter Variable"]
                    .iter()
                    .find(|f| crate::fonts::is_installed(f))
                    .map(|f| f.to_string())
            })
            .unwrap_or_else(|| "Sans".to_string());
        Palette {
            light,
            window: (base.0, base.1, base.2, 1.0),
            hud: (base.0, base.1, base.2, 0.94),
            text,
            accent: if light {
                (0.0, 0.478, 1.0, 1.0)
            } else {
                (0.039, 0.518, 1.0, 1.0)
            },
            rim: if light {
                (0.0, 0.0, 0.0, 0.12)
            } else {
                (1.0, 1.0, 1.0, 0.16)
            },
            font,
        }
    }

    /// labelColor at `opacity` (0.88 primary, 0.55 secondary, 0.30 tertiary).
    pub fn label(&self, opacity: f64) -> Rgba {
        (self.text.0, self.text.1, self.text.2, opacity)
    }

    /// systemFill at `level` (0.18 / 0.12 / 0.08 / 0.05).
    pub fn fill(&self, level: f64) -> Rgba {
        let k = if self.light { 0.55 } else { 1.0 };
        (self.text.0, self.text.1, self.text.2, level * k)
    }

    pub fn font_desc(&self, size: f64, bold: bool) -> gtk::pango::FontDescription {
        let mut d = gtk::pango::FontDescription::from_string(&format!(
            "{}{}",
            self.font,
            if bold { " Semi-Bold" } else { "" }
        ));
        d.set_absolute_size(size * f64::from(gtk::pango::SCALE));
        d
    }
}

pub const PRIMARY: f64 = 0.88;
pub const SECONDARY: f64 = 0.55;
pub const TERTIARY: f64 = 0.30;
pub const PRIMARY_FILL: f64 = 0.18;
pub const SECONDARY_FILL: f64 = 0.12;
pub const TERTIARY_FILL: f64 = 0.08;

pub fn set(cr: &Context, c: Rgba) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

fn css_rgba(c: Rgba) -> String {
    format!(
        "rgba({},{},{},{:.3})",
        (c.0 * 255.0).round(),
        (c.1 * 255.0).round(),
        (c.2 * 255.0).round(),
        c.3
    )
}

/// Stylesheet for windows carrying the `rustcast` style class.
pub fn css(p: &Palette) -> String {
    let win = css_rgba(p.window);
    let label = css_rgba(p.label(PRIMARY));
    let secondary = css_rgba(p.label(SECONDARY));
    let fill1 = css_rgba(p.fill(PRIMARY_FILL));
    let fill2 = css_rgba(p.fill(SECONDARY_FILL));
    let fill3 = css_rgba(p.fill(TERTIARY_FILL));
    let fill4 = css_rgba(p.fill(0.05));
    let accent = css_rgba(p.accent);
    let rim = css_rgba(p.rim);
    let sep = css_rgba(p.label(if p.light { 0.10 } else { 0.09 }));
    let checked = if p.light {
        "rgba(255,255,255,1)".to_string()
    } else {
        css_rgba(p.fill(0.22))
    };
    let font = &p.font;
    format!(
        r#"
window.rustcast, window.rustcast > .background, window.rustcast.background {{
  background-color: {win}; color: {label};
}}
window.rustcast * {{ font-family: "{font}"; }}
window.rustcast decoration {{ border-radius: 16px 16px 0 0; }}
window.rustcast headerbar {{
  background: {win}; background-image: none; border: none; box-shadow: none;
  min-height: 44px; padding: 0 10px;
}}
window.rustcast headerbar .title {{ font-weight: 600; font-size: 13px; color: {label}; }}
window.rustcast headerbar .subtitle {{ font-size: 11px; color: {secondary}; }}
window.rustcast label {{ color: {label}; }}
window.rustcast .dim, window.rustcast label.dim {{ color: {secondary}; font-size: 12px; }}
window.rustcast button {{
  background: {fill2}; background-image: none; color: {label};
  border: none; border-radius: 8px; box-shadow: none; text-shadow: none;
  -gtk-icon-shadow: none; padding: 5px 12px; min-height: 18px; font-size: 13px;
}}
window.rustcast button:hover {{ background: {fill1}; }}
window.rustcast button:active {{ background: {fill1}; }}
window.rustcast button:disabled {{ opacity: 0.4; }}
window.rustcast button.suggested {{ background: {accent}; color: #ffffff; }}
window.rustcast button.suggested label {{ color: #ffffff; }}
window.rustcast button.flat {{ background: transparent; }}
window.rustcast button.flat:hover {{ background: {fill3}; }}
window.rustcast headerbar button.titlebutton {{
  background: transparent; padding: 4px; min-width: 22px; min-height: 22px; border-radius: 99px;
}}
window.rustcast headerbar button.titlebutton:hover {{ background: {fill3}; }}
window.rustcast .segmented {{ background: {fill3}; border-radius: 9px; padding: 2px; }}
window.rustcast .segmented button {{ background: transparent; border-radius: 7px; padding: 3px 14px; }}
window.rustcast .segmented button:checked {{ background: {checked}; }}
window.rustcast .card {{ background: {fill4}; border-radius: 10px; border: 1px solid {sep}; }}
window.rustcast .chip {{ border-radius: 99px; padding: 3px 11px; font-size: 12px; }}
window.rustcast .cell {{ padding: 4px 10px; border-bottom: 1px solid {sep}; border-right: 1px solid {sep}; }}
window.rustcast .cell.head {{ font-weight: 600; background: {fill4}; }}
window.rustcast textview, window.rustcast textview text {{ background: transparent; color: {label}; }}
window.rustcast textview.mono, window.rustcast textview.mono text {{ font-family: monospace; font-size: 12px; }}
window.rustcast textview text selection {{ background: {accent}; color: #ffffff; }}
window.rustcast scrolledwindow {{ background: transparent; border: none; }}
window.rustcast scrollbar {{ background: transparent; border: none; }}
window.rustcast scrollbar slider {{ background: {fill1}; border-radius: 99px; min-width: 6px; min-height: 6px; border: none; }}
window.rustcast entry {{
  background: {fill3}; color: {label}; border: none; border-radius: 8px; box-shadow: none;
  padding: 4px 8px; min-height: 22px;
}}
menu.rustcast-menu, .rustcast-menu {{
  background: {win}; color: {label}; border: 1px solid {rim}; border-radius: 10px; padding: 4px;
}}
.rustcast-menu menuitem {{ padding: 5px 12px; border-radius: 6px; color: {label}; }}
.rustcast-menu menuitem label {{ color: {label}; font-family: "{font}"; font-size: 13px; }}
.rustcast-menu menuitem:hover {{ background: {accent}; }}
.rustcast-menu menuitem:hover label {{ color: #ffffff; }}
.rustcast-menu separator {{ background: {sep}; margin: 4px 6px; min-height: 1px; }}
"#
    )
}

/// Apply the RustCast stylesheet to every window of this process (once).
pub fn install_css() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let provider = gtk::CssProvider::new();
    if let Err(e) = provider.load_from_data(css(&Palette::load()).as_bytes()) {
        log::warn!("screenshot stylesheet: {e}");
        return;
    }
    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// A RustCast-styled window with a slim header bar (title, subtitle, close).
pub fn panel_window(title: &str, subtitle: Option<&str>) -> (gtk::Window, gtk::HeaderBar) {
    install_css();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.style_context().add_class("rustcast");
    let header = gtk::HeaderBar::new();
    header.set_title(Some(title));
    header.set_subtitle(subtitle);
    header.set_show_close_button(true);
    header.set_decoration_layout(Some(":close"));
    window.set_titlebar(Some(&header));
    window.set_title(title);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);
    window.connect_key_press_event(|w, ev| {
        if ev.keyval() == gtk::gdk::keys::constants::Escape {
            w.close();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    (window, header)
}

/// A themed popup menu.
pub fn menu() -> gtk::Menu {
    install_css();
    let menu = gtk::Menu::new();
    menu.style_context().add_class("rustcast-menu");
    menu
}

pub fn button(label: &str, suggested: bool) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    if suggested {
        b.style_context().add_class("suggested");
    }
    b
}

/// A segmented control (macOS NSSegmentedControl look). `on_change` gets the
/// index of the newly selected segment.
pub fn segmented(
    labels: &[&str],
    selected: usize,
    on_change: impl Fn(usize) + 'static,
) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bx.style_context().add_class("segmented");
    let buttons: Vec<gtk::ToggleButton> = labels
        .iter()
        .map(|l| {
            let b = gtk::ToggleButton::with_label(l);
            bx.pack_start(&b, false, false, 0);
            b
        })
        .collect();
    buttons[selected.min(buttons.len() - 1)].set_active(true);
    let on_change = std::rc::Rc::new(on_change);
    let guard = std::rc::Rc::new(std::cell::Cell::new(false));
    for (i, b) in buttons.iter().enumerate() {
        let all = buttons.clone();
        let (on_change, guard) = (on_change.clone(), guard.clone());
        b.connect_toggled(move |b| {
            if guard.get() {
                return;
            }
            guard.set(true);
            if b.is_active() {
                for (j, other) in all.iter().enumerate() {
                    if j != i {
                        other.set_active(false);
                    }
                }
                on_change(i);
            } else {
                // Keep exactly one segment selected.
                b.set_active(true);
            }
            guard.set(false);
        });
    }
    (bx, buttons)
}

/// Draw a label with the palette font, top-left at `(x, y)`; returns its size.
pub fn text(
    cr: &Context,
    p: &Palette,
    s: &str,
    x: f64,
    y: f64,
    size: f64,
    bold: bool,
) -> (f64, f64) {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&p.font_desc(size, bold)));
    layout.set_text(s);
    let (w, h) = layout.pixel_size();
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
    (f64::from(w), f64::from(h))
}

pub fn text_size(cr: &Context, p: &Palette, s: &str, size: f64, bold: bool) -> (f64, f64) {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&p.font_desc(size, bold)));
    layout.set_text(s);
    let (w, h) = layout.pixel_size();
    (f64::from(w), f64::from(h))
}

/// A rounded label pill centred on `(cx, cy)` on the HUD material.
pub fn pill(cr: &Context, p: &Palette, s: &str, cx: f64, cy: f64, size: f64, strong: bool) {
    let (w, h) = text_size(cr, p, s, size, false);
    let (px, py) = if strong { (14.0, 8.0) } else { (8.0, 4.0) };
    let (bw, bh) = (w + 2.0 * px, h + 2.0 * py);
    rounded_rect(cr, cx - bw / 2.0, cy - bh / 2.0, bw, bh, bh / 2.0);
    set(cr, p.hud);
    let _ = cr.fill_preserve();
    set(cr, p.rim);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    set(cr, p.label(PRIMARY));
    text(cr, p, s, cx - w / 2.0, cy - h / 2.0, size, false);
}

/// Icons shared by the capture toolbar and the thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Copy,
    Save,
    Pin,
    Edit,
    Text,
    Palette,
    Beautify,
    Undo,
    Redo,
    Close,
    More,
    Check,
}

/// Draw `icon` centred on `(cx, cy)` (about 18 × 18 px) with the current
/// source colour, line width ~1.6.
pub fn icon(cr: &Context, icon: Icon, cx: f64, cy: f64) {
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    let stroke = || {
        let _ = cr.stroke();
    };
    match icon {
        Icon::Copy => {
            rounded_rect(cr, cx - 7.0, cy - 4.0, 11.0, 12.0, 2.0);
            stroke();
            cr.move_to(cx - 3.0, cy - 6.5);
            cr.line_to(cx - 3.0, cy - 8.0);
            cr.line_to(cx + 8.0, cy - 8.0);
            cr.line_to(cx + 8.0, cy + 4.0);
            cr.line_to(cx + 6.5, cy + 4.0);
            stroke();
        }
        Icon::Save => {
            cr.move_to(cx, cy - 8.0);
            cr.line_to(cx, cy + 3.0);
            cr.move_to(cx - 4.5, cy - 1.5);
            cr.line_to(cx, cy + 3.0);
            cr.line_to(cx + 4.5, cy - 1.5);
            cr.move_to(cx - 8.0, cy + 3.0);
            cr.line_to(cx - 8.0, cy + 8.0);
            cr.line_to(cx + 8.0, cy + 8.0);
            cr.line_to(cx + 8.0, cy + 3.0);
            stroke();
        }
        Icon::Pin => {
            cr.arc(cx, cy - 3.5, 4.5, 0.0, TAU);
            stroke();
            cr.move_to(cx, cy + 1.0);
            cr.line_to(cx, cy + 9.0);
            stroke();
        }
        Icon::Edit => {
            cr.move_to(cx - 7.0, cy + 7.0);
            cr.line_to(cx - 6.0, cy + 3.0);
            cr.line_to(cx + 4.0, cy - 7.0);
            cr.line_to(cx + 7.0, cy - 4.0);
            cr.line_to(cx - 3.0, cy + 6.0);
            cr.close_path();
            stroke();
        }
        Icon::Text => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                cr.move_to(cx + sx * 8.0, cy + sy * 4.0);
                cr.line_to(cx + sx * 8.0, cy + sy * 8.0);
                cr.line_to(cx + sx * 4.0, cy + sy * 8.0);
            }
            stroke();
            cr.move_to(cx - 3.5, cy - 3.5);
            cr.line_to(cx + 3.5, cy - 3.5);
            cr.move_to(cx, cy - 3.5);
            cr.line_to(cx, cy + 4.0);
            stroke();
        }
        Icon::Palette => {
            for (i, (dx, dy)) in [(-4.0, -3.0), (4.0, -3.0), (0.0, 4.0)].iter().enumerate() {
                cr.arc(cx + dx, cy + dy, 3.6, 0.0, TAU);
                if i == 0 {
                    let _ = cr.fill();
                } else {
                    stroke();
                }
            }
        }
        Icon::Beautify => {
            for i in 0..8 {
                let a = f64::from(i) * PI / 4.0;
                let r = if i % 2 == 0 { 8.5 } else { 3.0 };
                let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.close_path();
            let _ = cr.fill();
        }
        Icon::Undo | Icon::Redo => {
            let dir = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let _ = cr.save();
            cr.translate(cx, cy);
            cr.scale(dir, 1.0);
            cr.arc(1.0, 1.0, 6.0, PI, PI * 2.6);
            stroke();
            cr.move_to(-8.5, -2.0);
            cr.line_to(-5.0, 2.5);
            cr.line_to(-1.5, -2.0);
            stroke();
            let _ = cr.restore();
        }
        Icon::Close => {
            cr.move_to(cx - 5.5, cy - 5.5);
            cr.line_to(cx + 5.5, cy + 5.5);
            cr.move_to(cx + 5.5, cy - 5.5);
            cr.line_to(cx - 5.5, cy + 5.5);
            stroke();
        }
        Icon::More => {
            for dx in [-6.0, 0.0, 6.0] {
                cr.arc(cx + dx, cy, 1.8, 0.0, TAU);
                let _ = cr.fill();
            }
        }
        Icon::Check => {
            cr.move_to(cx - 6.5, cy + 0.5);
            cr.line_to(cx - 2.0, cy + 5.0);
            cr.line_to(cx + 7.0, cy - 5.0);
            stroke();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_follows_theme_brightness() {
        let mut theme = crate::config::Theme::default();
        let dark = Palette::from_theme(&theme);
        assert!(!dark.light);
        // Graphite, not black (matches styles::window_fill).
        assert!(dark.window.0 > 0.08 && dark.window.0 < 0.2);

        theme.theme_mode = crate::config::ThemeMode::Light;
        let (t, b) = crate::config::ThemeMode::Light.presets(false);
        theme.text_color = t;
        theme.background_color = b;
        let light = Palette::from_theme(&theme);
        assert!(light.light);
        assert!(light.window.0 > 0.9);
        assert!(css(&light).contains("window.rustcast"));
    }
}
