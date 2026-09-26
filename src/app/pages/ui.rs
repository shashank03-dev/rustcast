//! Small component kit for the purpose-built pages (clipboard, recorder):
//! cards, badges, switches, key caps and section labels, plus the motion
//! helpers that animate them (staggered entrances, eased selection, springy
//! switches — the Framer-Motion feel, done natively).
//!
//! Every component takes a `fade` factor (0‥1) that is multiplied into its
//! alphas, so whole sections can fade in as one.

use std::time::{Duration, Instant};

use iced::animation::Easing;
use iced::border::Radius;
use iced::font::Weight;
use iced::widget::{Space, button, container, text};
use iced::{Background, Border, Color, Element, Font, Length, Shadow, Vector};

use crate::app::Message;
use crate::config::Theme;
use crate::styles::with_alpha;

pub use crate::styles::{faded, mix};

/// The accent used for selection and primary actions: macOS systemBlue.
pub fn accent(a: f32) -> Color {
    Color::from_rgba(0.039, 0.518, 1.0, a)
}

/// The recording red (systemRed).
pub fn red(a: f32) -> Color {
    Color::from_rgba(1.0, 0.271, 0.227, a)
}

/// systemGreen.
pub fn green(a: f32) -> Color {
    Color::from_rgba(0.188, 0.820, 0.345, a)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ----- tokens ---------------------------------------------------------------
//
// Surfaces are systemFill-style overlays on the window material (see
// `styles`), so they stay translucent over glass and solid over the opaque
// material, in light and dark mode alike. Elevation goes
// window < panel < card < hover < selected.

pub const WINDOW: f32 = 0.035;
pub const PANEL: f32 = 0.05;
pub const CARD: f32 = 0.07;
pub const HOVER: f32 = 0.10;
pub const SELECTED: f32 = 0.14;

/// Surface overlay at elevation `level` (one of the constants above).
pub fn surface(theme: &Theme, level: f32) -> Color {
    crate::styles::fill(theme, level)
}

/// 1px separators and resting borders.
pub fn hairline(theme: &Theme, fade: f32) -> Color {
    faded(crate::styles::separator(theme), fade)
}

/// Text hierarchy: titles and body, supporting detail, hints.
pub fn text_primary(theme: &Theme, fade: f32) -> Color {
    theme.text_color(0.96 * fade)
}
pub fn text_secondary(theme: &Theme, fade: f32) -> Color {
    theme.text_color(0.64 * fade)
}
pub fn text_tertiary(theme: &Theme, fade: f32) -> Color {
    theme.text_color(0.44 * fade)
}

/// The theme's UI font at `weight` (SF Pro Text when configured).
pub fn font(theme: &Theme, weight: Weight) -> Font {
    Font {
        weight,
        ..theme.font()
    }
}

/// Font for large titles (20px and up). SF Pro ships two optical sizes:
/// Text for UI sizes and Display for headings, so swap to Display here when
/// it is installed.
pub fn display_font(theme: &Theme, weight: Weight) -> Font {
    let base = theme.font();
    Font {
        family: crate::fonts::display_variant(base.family),
        weight,
        ..base
    }
}

// ----- motion ---------------------------------------------------------------

/// Eased progress (0‥1, may overshoot for "back" easings) of an animation that
/// started at `start`, waits `delay_ms`, and runs for `dur_ms`.
pub fn progress(start: Instant, now: Instant, delay_ms: u64, dur_ms: u64, easing: Easing) -> f32 {
    let elapsed = now.saturating_duration_since(start);
    let delay = Duration::from_millis(delay_ms);
    if elapsed <= delay {
        return 0.0;
    }
    let x = ((elapsed - delay).as_secs_f32() / (dur_ms as f32 / 1000.0)).clamp(0.0, 1.0);
    easing.value(x)
}

/// Staggered entrance for the `index`-th element of a page.
pub fn stagger(start: Instant, now: Instant, index: usize) -> f32 {
    progress(
        start,
        now,
        (index as u64).min(12) * 28,
        320,
        Easing::EaseOutCubic,
    )
}

/// Vertical slide offset (px) for an entrance progress value.
pub fn slide(t: f32) -> f32 {
    (1.0 - t.clamp(0.0, 1.0)) * 14.0
}

/// Wrap `content` so it slides up into place as `t` goes 0 → 1.
pub fn enter<'a>(content: impl Into<Element<'a, Message>>, t: f32) -> Element<'a, Message> {
    container(content)
        .padding(iced::Padding {
            top: slide(t),
            ..iced::Padding::ZERO
        })
        .into()
}

// ----- components -----------------------------------------------------------

/// Card surface. `focus` (0‥1, animated) blends in the selection look;
/// `hovered` adds a lighter lift.
pub fn card_style(theme: &Theme, focus: f32, hovered: bool, fade: f32) -> container::Style {
    let rest = surface(theme, if hovered { HOVER } else { CARD });
    // Selected cards take an accent tint, like a focused macOS list row.
    let bg = mix(rest, accent(0.22), focus);
    container::Style {
        background: Some(Background::Color(faded(bg, fade))),
        border: Border {
            color: with_alpha(
                mix(theme.text_color(1.0), accent(1.0), focus),
                (0.08 + 0.62 * focus) * fade,
            ),
            width: 1.0,
            radius: Radius::new(10.0),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.18 * focus * fade),
            offset: Vector::new(0.0, 3.0 * focus),
            blur_radius: 10.0 * focus,
        },
        text_color: Some(theme.text_color(fade)),
        snap: false,
    }
}

/// A clickable card: `content` inside a button styled as a card.
pub fn card_button<'a>(
    content: impl Into<Element<'a, Message>>,
    theme: &Theme,
    focus: f32,
    fade: f32,
    on_press: Option<Message>,
) -> button::Button<'a, Message> {
    let theme = theme.clone();
    button(content)
        .padding(0)
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let c = card_style(&theme, focus, hovered, fade);
            button::Style {
                background: c.background,
                border: c.border,
                shadow: c.shadow,
                text_color: theme.text_color(fade),
                snap: false,
            }
        })
}

/// Small status chip, e.g. `Text`, `Image`, `Minimized`: tinted fill, no
/// outline, sentence case.
pub fn badge<'a>(
    label: impl ToString,
    color: Color,
    theme: &Theme,
    fade: f32,
) -> Element<'a, Message> {
    container(
        text(label.to_string())
            .size(11)
            .font(font(theme, Weight::Medium))
            .color(with_alpha(mix(color, theme.text_color(1.0), 0.25), fade)),
    )
    .padding([2, 8])
    .style(move |_| container::Style {
        background: Some(Background::Color(with_alpha(color, 0.16 * fade))),
        border: Border {
            radius: Radius::new(6.0),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// A key-cap hint such as `↵` or `⌘1`.
pub fn kbd<'a>(label: impl ToString, theme: &Theme, fade: f32) -> Element<'a, Message> {
    let t = theme.clone();
    container(
        text(label.to_string())
            .size(11)
            .font(font(theme, Weight::Medium))
            .color(text_secondary(theme, fade)),
    )
    .padding([1, 6])
    .style(move |_| container::Style {
        background: Some(Background::Color(faded(surface(&t, HOVER), fade))),
        border: Border {
            color: t.text_color(0.14 * fade),
            width: 1.0,
            radius: Radius::new(5.0),
        },
        // Key-cap bottom edge.
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35 * fade),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 0.0,
        },
        ..container::Style::default()
    })
    .into()
}

/// Muted uppercase section heading.
pub fn section_label<'a>(label: impl ToString, theme: &Theme, fade: f32) -> Element<'a, Message> {
    text(label.to_string().to_uppercase())
        .size(11)
        .font(font(theme, Weight::Semibold))
        .color(text_tertiary(theme, fade))
        .into()
}

/// A pill switch. `t` is the knob position (0 = off, 1 = on) and may
/// overshoot slightly for a springy feel.
pub fn switch<'a>(t: f32, theme: &Theme, fade: f32) -> Element<'a, Message> {
    let track_on = accent(0.95 * fade);
    let track_off = theme.text_color(0.16 * fade);
    let tc = t.clamp(0.0, 1.0);
    let knob_x = lerp(3.0, 17.0, t).clamp(1.0, 19.0);
    let knob = container(Space::new().width(14).height(14)).style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, fade))),
        border: Border {
            radius: Radius::new(7.0),
            ..Border::default()
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.3 * fade),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 3.0,
        },
        ..container::Style::default()
    });
    container(
        iced::widget::row![Space::new().width(knob_x), knob]
            .height(20)
            .align_y(iced::Alignment::Center),
    )
    .width(34)
    .height(20)
    .style(move |_| container::Style {
        background: Some(Background::Color(mix(track_off, track_on, tc))),
        border: Border {
            radius: Radius::new(10.0),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// Rounded pill button in one of three tones.
#[derive(Clone, Copy)]
pub enum Tone {
    Primary,
    Danger,
    Quiet,
}

pub fn pill_button<'a>(
    content: impl Into<Element<'a, Message>>,
    tone: Tone,
    theme: &Theme,
    on_press: Option<Message>,
) -> button::Button<'a, Message> {
    let theme = theme.clone();
    button(content)
        .padding([7, 14])
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let hover = matches!(status, button::Status::Hovered);
            let pressed = matches!(status, button::Status::Pressed);
            let boost = if pressed {
                0.25
            } else if hover {
                0.12
            } else {
                0.0
            };
            // Solid primary, ghost danger (fills only on hover), quiet
            // secondary on a raised surface.
            let (bg, fg, border) = match tone {
                Tone::Primary => (
                    mix(accent(1.0), Color::WHITE, boost * 0.6),
                    Color::WHITE,
                    with_alpha(Color::WHITE, 0.14),
                ),
                Tone::Danger => (red(boost * 1.2), red(1.0), Color::TRANSPARENT),
                Tone::Quiet => (
                    mix(surface(&theme, HOVER), theme.text_color(1.0), boost * 0.5),
                    text_primary(&theme, 1.0),
                    hairline(&theme, 1.0),
                ),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: Radius::new(8.0),
                },
                shadow: match tone {
                    Tone::Primary => Shadow {
                        color: with_alpha(accent(1.0), 0.35),
                        offset: Vector::new(0.0, 2.0),
                        blur_radius: 10.0,
                    },
                    _ => Shadow::default(),
                },
                snap: false,
            }
        })
}

/// A square glyph tile used as a card's leading icon.
pub fn glyph<'a>(symbol: &str, color: Color, size: f32, fade: f32) -> Element<'a, Message> {
    container(
        text(symbol.to_string())
            .size(size * 0.5)
            .color(with_alpha(color, fade)),
    )
    .width(size)
    .height(size)
    .center_x(size)
    .center_y(size)
    .style(move |_| container::Style {
        background: Some(Background::Color(with_alpha(color, 0.14 * fade))),
        border: Border {
            color: with_alpha(color, 0.30 * fade),
            width: 1.0,
            radius: Radius::new(size * 0.28),
        },
        ..container::Style::default()
    })
    .into()
}

/// A full-width panel surface (preview areas, hero banners).
pub fn panel_style(theme: &Theme, tone: Option<Color>, fade: f32) -> container::Style {
    let base = surface(theme, PANEL);
    let bg = match tone {
        Some(c) => mix(base, c, 0.14),
        None => base,
    };
    container::Style {
        background: Some(Background::Color(faded(bg, fade))),
        border: Border {
            color: tone
                .map(|c| with_alpha(c, 0.40 * fade))
                .unwrap_or(hairline(theme, fade)),
            width: 1.0,
            radius: Radius::new(12.0),
        },
        text_color: Some(theme.text_color(fade)),
        ..container::Style::default()
    }
}

/// Segmented control: a recessed track with the active segment raised.
/// Each item is (label, count, message); `active` is the selected index.
pub fn segmented<'a>(
    items: Vec<(String, usize, Message)>,
    active: usize,
    theme: &Theme,
    fade: f32,
) -> Element<'a, Message> {
    let segs = items
        .into_iter()
        .enumerate()
        .map(|(i, (label, count, msg))| {
            let on = i == active;
            let t = theme.clone();
            let content = iced::widget::row![
                text(label)
                    .size(12)
                    .font(font(
                        theme,
                        if on { Weight::Semibold } else { Weight::Medium }
                    ))
                    .color(if on {
                        text_primary(theme, fade)
                    } else {
                        text_secondary(theme, fade)
                    }),
                text(count.to_string())
                    .size(11)
                    .font(font(theme, Weight::Medium))
                    .color(text_tertiary(theme, fade)),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center);
            button(content)
                .padding([4, 11])
                .on_press(msg)
                .style(move |_, status| {
                    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
                    let bg = if on {
                        Some(Background::Color(faded(surface(&t, SELECTED + 0.03), fade)))
                    } else if hover {
                        Some(Background::Color(faded(surface(&t, HOVER), fade)))
                    } else {
                        None
                    };
                    button::Style {
                        background: bg,
                        text_color: text_primary(&t, fade),
                        border: Border {
                            color: if on {
                                t.text_color(0.10 * fade)
                            } else {
                                Color::TRANSPARENT
                            },
                            width: 1.0,
                            radius: Radius::new(7.0),
                        },
                        shadow: if on {
                            Shadow {
                                color: Color::from_rgba(0.0, 0.0, 0.0, 0.30 * fade),
                                offset: Vector::new(0.0, 1.0),
                                blur_radius: 4.0,
                            }
                        } else {
                            Shadow::default()
                        },
                        snap: false,
                    }
                })
                .into()
        });
    let t = theme.clone();
    container(iced::widget::row(segs).spacing(2))
        .padding(3)
        .style(move |_| container::Style {
            background: Some(Background::Color(faded(surface(&t, WINDOW), fade))),
            border: Border {
                color: hairline(&t, fade),
                width: 1.0,
                radius: Radius::new(10.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// Fill the remaining width.
pub fn spacer<'a>() -> Element<'a, Message> {
    Space::new().width(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_respects_delay_and_completes() {
        let start = Instant::now();
        assert_eq!(progress(start, start, 100, 200, Easing::Linear), 0.0);
        let mid = start + Duration::from_millis(200);
        assert!((progress(start, mid, 100, 200, Easing::Linear) - 0.5).abs() < 0.01);
        let end = start + Duration::from_millis(400);
        assert_eq!(progress(start, end, 100, 200, Easing::Linear), 1.0);
    }

    #[test]
    fn stagger_orders_items() {
        let start = Instant::now();
        let now = start + Duration::from_millis(120);
        assert!(stagger(start, now, 0) > stagger(start, now, 3));
        assert_eq!(slide(1.0), 0.0);
        assert!(slide(0.0) > 0.0);
    }

    #[test]
    fn mix_blends_endpoints() {
        let a = Color::BLACK;
        let b = Color::WHITE;
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }
}
