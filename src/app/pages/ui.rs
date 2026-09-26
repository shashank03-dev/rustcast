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
use iced::widget::{Space, button, container, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Vector};

use crate::app::Message;
use crate::config::Theme;
use crate::styles::{tint, with_alpha};

/// The accent used for selection and primary actions.
pub fn accent(a: f32) -> Color {
    Color::from_rgba(0.22, 0.55, 0.96, a)
}

/// The recording red.
pub fn red(a: f32) -> Color {
    Color::from_rgba(0.95, 0.26, 0.21, a)
}

pub fn green(a: f32) -> Color {
    Color::from_rgba(0.30, 0.78, 0.45, a)
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
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
    let base = theme.bg_color();
    let rest = with_alpha(tint(base, if hovered { 0.10 } else { 0.06 }), 0.62);
    let selected = with_alpha(tint(base, 0.16), 0.92);
    let bg = mix(rest, selected, focus);
    container::Style {
        background: Some(Background::Color(with_alpha(bg, bg.a * fade))),
        border: Border {
            color: with_alpha(
                mix(theme.text_color(0.10), accent(0.85), focus),
                (0.10 + 0.75 * focus) * fade,
            ),
            width: 1.0,
            radius: Radius::new(12.0),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35 * focus * fade),
            offset: Vector::new(0.0, 6.0 * focus),
            blur_radius: 18.0 * focus,
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

/// Thin accent bar on the leading edge of the focused card.
pub fn focus_bar<'a>(focus: f32, height: f32) -> Element<'a, Message> {
    container(Space::new().width(3).height(height * focus.clamp(0.0, 1.0)))
        .height(height)
        .center_y(height)
        .style(move |_| container::Style {
            background: Some(Background::Color(accent(focus.clamp(0.0, 1.0)))),
            border: Border {
                radius: Radius::new(2.0),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// Small uppercase pill, e.g. `TEXT`, `IMAGE`, `MINIMIZED`.
pub fn badge<'a>(
    label: impl ToString,
    color: Color,
    theme: &Theme,
    fade: f32,
) -> Element<'a, Message> {
    let font = theme.font();
    container(
        text(label.to_string().to_uppercase())
            .size(10)
            .font(font)
            .color(with_alpha(color, fade)),
    )
    .padding([2, 7])
    .style(move |_| container::Style {
        background: Some(Background::Color(with_alpha(color, 0.14 * fade))),
        border: Border {
            color: with_alpha(color, 0.35 * fade),
            width: 1.0,
            radius: Radius::new(8.0),
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
            .size(10)
            .font(theme.font())
            .color(theme.text_color(0.65 * fade)),
    )
    .padding([1, 6])
    .style(move |_| container::Style {
        background: Some(Background::Color(with_alpha(
            tint(t.bg_color(), 0.14),
            0.7 * fade,
        ))),
        border: Border {
            color: t.text_color(0.18 * fade),
            width: 1.0,
            radius: Radius::new(5.0),
        },
        ..container::Style::default()
    })
    .into()
}

/// Muted uppercase section heading.
pub fn section_label<'a>(label: impl ToString, theme: &Theme, fade: f32) -> Element<'a, Message> {
    text(label.to_string().to_uppercase())
        .size(11)
        .font(theme.font())
        .color(theme.text_color(0.45 * fade))
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
            let (bg, fg, border) = match tone {
                Tone::Primary => (accent(0.85 + boost * 0.5), Color::WHITE, accent(1.0)),
                Tone::Danger => (red(0.16 + boost), red(1.0), red(0.45)),
                Tone::Quiet => (
                    with_alpha(tint(theme.bg_color(), 0.10 + boost), 0.75),
                    theme.text_color(0.9),
                    theme.text_color(0.16),
                ),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: Radius::new(10.0),
                },
                shadow: Shadow::default(),
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
    let base = with_alpha(tint(theme.bg_color(), 0.05), 0.55);
    let bg = match tone {
        Some(c) => mix(base, with_alpha(c, 0.22), 0.6),
        None => base,
    };
    container::Style {
        background: Some(Background::Color(with_alpha(bg, bg.a * fade))),
        border: Border {
            color: tone
                .map(|c| with_alpha(c, 0.45 * fade))
                .unwrap_or(theme.text_color(0.10 * fade)),
            width: 1.0,
            radius: Radius::new(14.0),
        },
        text_color: Some(theme.text_color(fade)),
        ..container::Style::default()
    }
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
