//! This handles most of the styling for the rustcast elements
use crate::config::{GlassMode, Theme as ConfigTheme};
use iced::Radians;
use iced::Shadow;
use iced::border::Radius;
use iced::gradient::Linear;
use iced::widget::{button, checkbox, container, radio, scrollable, slider};
use iced::{Background, Border, Color, widget::text_input};

/// Helper: mix base color with white (simple “tint”)
pub fn tint(mut c: Color, amount: f32) -> Color {
    c.r = c.r + (1.0 - c.r) * amount;
    c.g = c.g + (1.0 - c.g) * amount;
    c.b = c.b + (1.0 - c.b) * amount;
    c
}

/// Helper: apply alpha
pub fn with_alpha(mut c: Color, a: f32) -> Color {
    c.a = a;
    c
}

/// The search field: no box of its own, it sits directly on the window
/// material like Spotlight's.
pub fn rustcast_text_input_style(theme: &ConfigTheme) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: label(theme, 0.),
        placeholder: label(theme, TERTIARY),
        value: label(theme, PRIMARY),
        selection: with_alpha(accent(theme), 0.40),
    }
}

/// The launcher window: one material fill, a light rim and rounded corners.
/// Every section inside draws on top of this and stays transparent, so the
/// material (and the compositor's blur behind it) is painted exactly once.
pub fn contents_style(theme: &ConfigTheme) -> container::Style {
    container::Style {
        background: Some(Background::Color(window_fill(theme))),
        text_color: Some(label(theme, PRIMARY)),
        border: Border {
            color: rim(theme),
            width: 1.0,
            radius: Radius::new(WINDOW_RADIUS),
        },
        ..Default::default()
    }
}

pub fn delete_button_style(theme: &ConfigTheme, status: button::Status) -> button::Style {
    // systemRed; ghost until hovered, like a destructive toolbar button.
    let red = Color::from_rgb(1.0, 0.271, 0.227);
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(with_alpha(red, 0.14))),
        button::Status::Pressed => Some(Background::Color(with_alpha(red, 0.24))),
        _ => Some(Background::Color(fill(theme, QUATERNARY_FILL))),
    };
    button::Style {
        text_color: red,
        background: bg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.,
            radius: Radius::new(7),
        },
        ..Default::default()
    }
}

/// Styling for each of the buttons that are what the "results" of rustcast are
pub fn result_button_style(theme: &ConfigTheme) -> button::Style {
    button::Style {
        text_color: label(theme, PRIMARY),
        background: None,
        ..Default::default()
    }
}

/// The favourite heart: systemPink when set, a faint outline-like glyph
/// otherwise that brightens on hover.
pub fn favourite_button_style(
    theme: &ConfigTheme,
    status: button::Status,
    is_favourite: bool,
    on_selection: bool,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let text_color = if is_favourite {
        Color::from_rgb(1.0, 0.216, 0.373)
    } else if on_selection || hovered {
        label(theme, if hovered { SECONDARY } else { TERTIARY })
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        text_color,
        background: None,
        ..Default::default()
    }
}

/// macOS overlay scroller: a thin rounded thumb, no track.
pub fn results_scrollbar_style(tile: &ConfigTheme) -> scrollable::Style {
    scrollable::Style {
        container: container::Style {
            text_color: None,
            background: None,
            border: Border::default(),
            shadow: Shadow::default(),
            snap: false,
        },
        vertical_rail: scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: Background::Color(label(tile, 0.35)),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.,
                    radius: Radius::new(3),
                },
            },
        },
        horizontal_rail: scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: Background::Color(Color::TRANSPARENT),
                border: Border::default(),
            },
        },
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: Color::TRANSPARENT,
        },
    }
}

pub fn settings_radio_button_style(theme: &ConfigTheme, status: radio::Status) -> radio::Style {
    let selected = matches!(
        status,
        radio::Status::Active { is_selected: true } | radio::Status::Hovered { is_selected: true }
    );
    radio::Style {
        background: Background::Color(if selected {
            accent(theme)
        } else {
            fill(theme, TERTIARY_FILL)
        }),
        dot_color: Color::WHITE,
        border_width: if selected { 0. } else { 1. },
        border_color: label(theme, 0.22),
        text_color: Some(label(theme, PRIMARY)),
    }
}

/// A result row. Selection is a quiet neutral highlight (Raycast/Spotlight
/// list style): a soft fill inset from the window edge, labels unchanged.
pub fn result_row_container_style(tile: &ConfigTheme, focused: bool) -> container::Style {
    container::Style {
        background: focused.then(|| Background::Color(fill(tile, SELECTION_FILL))),
        border: Border {
            radius: Radius::new(ROW_RADIUS),
            ..Border::default()
        },
        text_color: Some(label(tile, PRIMARY)),
        ..Default::default()
    }
}

/// The emoji results container style
///
/// Takes a focused boolean, to know if this specific button is focused or not
pub fn emoji_button_container_style(tile_theme: &ConfigTheme, focused: bool) -> container::Style {
    container::Style {
        background: focused.then(|| Background::Color(fill(tile_theme, SELECTION_FILL))),
        text_color: Some(label(tile_theme, PRIMARY)),
        border: Border {
            color: if focused {
                label(tile_theme, 0.22)
            } else {
                Color::TRANSPARENT
            },
            width: 1.0,
            radius: Radius::new(12.0),
        },
        ..Default::default()
    }
}

/// Emoji buttons styling
pub fn emoji_button_style(tile_theme: &ConfigTheme, status: button::Status) -> button::Style {
    let level = match status {
        button::Status::Hovered | button::Status::Pressed => TERTIARY_FILL,
        _ => 0.0,
    };
    button::Style {
        background: Some(Background::Color(fill(tile_theme, level))),
        text_color: label(tile_theme, PRIMARY),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::new(12.0),
        },
        ..Default::default()
    }
}

/// macOS text field: recessed fill, hairline border, accent focus ring.
pub fn settings_text_input_item_style(
    theme: &ConfigTheme,
    status: text_input::Status,
) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(fill(theme, QUATERNARY_FILL)),
        border: Border {
            color: if focused {
                with_alpha(accent(theme), 0.70)
            } else {
                separator(theme)
            },
            width: if focused { 1.5 } else { 1.0 },
            radius: Radius::new(8.),
        },
        icon: label(theme, SECONDARY),
        placeholder: label(theme, TERTIARY),
        value: label(theme, PRIMARY),
        selection: with_alpha(accent(theme), 0.40),
    }
}

/// Push button on the window material (Copy config, Open file).
pub fn settings_save_button_style(theme: &ConfigTheme, status: button::Status) -> button::Style {
    let level = match status {
        button::Status::Pressed => PRIMARY_FILL,
        button::Status::Hovered => SECONDARY_FILL,
        _ => TERTIARY_FILL,
    };
    button::Style {
        text_color: label(theme, PRIMARY),
        background: Some(Background::Color(fill(theme, level))),
        border: Border {
            color: separator(theme),
            width: 0.5,
            radius: Radius::new(7),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.12),
            offset: iced::Vector::new(0.0, 0.5),
            blur_radius: 1.0,
        },
        ..Default::default()
    }
}

/// The default button (Save): accent fill, white label.
pub fn settings_primary_button_style(theme: &ConfigTheme, status: button::Status) -> button::Style {
    let base = accent(theme);
    let bg = match status {
        button::Status::Pressed => mix(base, Color::BLACK, 0.15),
        button::Status::Hovered => mix(base, Color::WHITE, 0.10),
        _ => base,
    };
    button::Style {
        text_color: Color::WHITE,
        background: Some(Background::Color(bg)),
        border: Border {
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.12),
            width: 0.5,
            radius: Radius::new(7),
        },
        shadow: Shadow {
            color: with_alpha(base, 0.30),
            offset: iced::Vector::new(0.0, 1.0),
            blur_radius: 4.0,
        },
        ..Default::default()
    }
}

pub fn settings_add_button_style(theme: &ConfigTheme, status: button::Status) -> button::Style {
    let level = match status {
        button::Status::Pressed => SECONDARY_FILL,
        button::Status::Hovered => TERTIARY_FILL,
        _ => QUATERNARY_FILL,
    };
    button::Style {
        background: Some(Background::Color(fill(theme, level))),
        text_color: label(theme, PRIMARY),
        border: Border {
            color: separator(theme),
            width: 0.5,
            radius: Radius::new(7),
        },
        ..Default::default()
    }
}

/// Settings tabs as a macOS segmented control: the active segment is a
/// raised pill, the others are plain labels.
pub fn settings_tab_style(
    theme: &ConfigTheme,
    active: bool,
    status: button::Status,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (bg, text, shadow) = if active {
        let raised = if theme.is_light() {
            Color::WHITE
        } else {
            fill(theme, 0.13)
        };
        (
            Some(Background::Color(raised)),
            label(theme, PRIMARY),
            Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.22),
                offset: iced::Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            },
        )
    } else if hovered {
        (
            Some(Background::Color(fill(theme, QUATERNARY_FILL))),
            label(theme, PRIMARY),
            Shadow::default(),
        )
    } else {
        (None, label(theme, SECONDARY), Shadow::default())
    };
    button::Style {
        text_color: text,
        background: bg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.,
            radius: Radius::new(7.),
        },
        shadow,
        ..Default::default()
    }
}

/// The settings panel sits straight on the window material.
pub fn settings_container_style(theme: &ConfigTheme) -> container::Style {
    container::Style {
        background: None,
        text_color: Some(label(theme, PRIMARY)),
        ..Default::default()
    }
}

/// macOS checkbox: rounded square, accent fill with a white check when on.
pub fn settings_checkbox_style(theme: &ConfigTheme, status: checkbox::Status) -> checkbox::Style {
    let checked = matches!(
        status,
        checkbox::Status::Active { is_checked: true }
            | checkbox::Status::Hovered { is_checked: true }
            | checkbox::Status::Disabled { is_checked: true }
    );
    checkbox::Style {
        background: Background::Color(if checked {
            accent(theme)
        } else {
            fill(theme, TERTIARY_FILL)
        }),
        icon_color: Color::WHITE,
        border: iced::Border {
            color: if checked {
                Color::TRANSPARENT
            } else {
                label(theme, 0.22)
            },
            width: 1.,
            radius: Radius::new(4.),
        },
        text_color: None,
    }
}

/// macOS slider: thin track filled with the accent, white knob with a shadow.
pub fn settings_slider_style(theme: &ConfigTheme, _status: slider::Status) -> slider::Style {
    slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(accent(theme)),
                Background::Color(fill(theme, PRIMARY_FILL)),
            ),
            width: 4.,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.,
                radius: Radius::new(2),
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 9. },
            background: Background::Color(Color::WHITE),
            border_width: 0.5,
            border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.18),
        },
    }
}

// ----- macOS material -------------------------------------------------------
//
// Values follow AppKit's semantic colors (labelColor, separatorColor,
// systemFill, controlAccentColor) and the Spotlight/HUD window material. The
// window paints one fill; with a blurring compositor it is translucent so the
// desktop shows through frosted, otherwise it is solid. Blur is done by the
// compositor on the GPU, RustCast itself never blurs anything.

/// Corner radius of the launcher window.
pub const WINDOW_RADIUS: f32 = 16.0;
/// Corner radius of a selected row.
pub const ROW_RADIUS: f32 = 8.0;

/// Fill of the selected list row (and emoji cell).
pub const SELECTION_FILL: f32 = 0.10;

/// labelColor / secondaryLabelColor / tertiaryLabelColor opacities.
pub const PRIMARY: f32 = 0.88;
pub const SECONDARY: f32 = 0.55;
pub const TERTIARY: f32 = 0.30;

/// systemFill levels, from most to least prominent.
pub const PRIMARY_FILL: f32 = 0.18;
pub const SECONDARY_FILL: f32 = 0.12;
pub const TERTIARY_FILL: f32 = 0.08;
pub const QUATERNARY_FILL: f32 = 0.05;

/// Whether the window is translucent glass (the compositor blurs behind it).
pub fn translucent(theme: &ConfigTheme) -> bool {
    static BLURS: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(crate::platform::compositor_blurs);
    match theme.glass {
        GlassMode::On => true,
        GlassMode::Off => false,
        GlassMode::Auto => *BLURS,
    }
}

/// The window material. Dark: #1E1E1E-ish graphite; light: near-white.
/// Derived from the theme so custom background colors still apply.
pub fn window_fill(theme: &ConfigTheme) -> Color {
    let base = with_alpha(theme.bg_color(), 1.0);
    let (tone, alpha) = if theme.is_light() {
        (tint(base, 0.55), 0.72)
    } else {
        (mix(base, theme.text_color(1.0), 0.12), 0.70)
    };
    with_alpha(tone, if translucent(theme) { alpha } else { 1.0 })
}

/// The glass rim: a light edge in dark mode, a soft dark edge in light mode.
pub fn rim(theme: &ConfigTheme) -> Color {
    if theme.is_light() {
        Color::from_rgba(0.0, 0.0, 0.0, 0.12)
    } else {
        Color::from_rgba(1.0, 1.0, 1.0, 0.16)
    }
}

/// The specular highlight along the top edge of the glass.
pub fn sheen(theme: &ConfigTheme) -> Background {
    let peak = if theme.is_light() { 0.9 } else { 0.30 };
    Background::Gradient(
        Linear::new(Radians(std::f32::consts::FRAC_PI_2))
            .add_stop(0.0, Color::from_rgba(1.0, 1.0, 1.0, 0.0))
            .add_stop(0.5, Color::from_rgba(1.0, 1.0, 1.0, peak))
            .add_stop(1.0, Color::from_rgba(1.0, 1.0, 1.0, 0.0))
            .into(),
    )
}

/// separatorColor.
pub fn separator(theme: &ConfigTheme) -> Color {
    label(theme, if theme.is_light() { 0.10 } else { 0.09 })
}

/// A label color at `opacity` (one of PRIMARY / SECONDARY / TERTIARY).
pub fn label(theme: &ConfigTheme, opacity: f32) -> Color {
    // Light-mode labels are black at these opacities; dark-mode labels are
    // the theme's text color.
    theme.text_color(opacity)
}

/// A systemFill overlay at `level`, drawn over the window material.
pub fn fill(theme: &ConfigTheme, level: f32) -> Color {
    let k = if theme.is_light() { 0.55 } else { 1.0 };
    theme.text_color(level * k)
}

/// controlAccentColor: systemBlue (#0A84FF dark, #007AFF light).
pub fn accent(theme: &ConfigTheme) -> Color {
    if theme.is_light() {
        Color::from_rgb(0.0, 0.478, 1.0)
    } else {
        Color::from_rgb(0.039, 0.518, 1.0)
    }
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

/// Multiply a color's alpha by `fade` (for fade-in animations).
pub fn faded(mut c: Color, fade: f32) -> Color {
    c.a *= fade.clamp(0.0, 1.0);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_material_when_glass_is_off() {
        let theme = ConfigTheme {
            glass: GlassMode::Off,
            ..ConfigTheme::default()
        };
        assert_eq!(window_fill(&theme).a, 1.0);
        let glass = ConfigTheme {
            glass: GlassMode::On,
            ..ConfigTheme::default()
        };
        assert!(window_fill(&glass).a < 1.0);
    }

    #[test]
    fn dark_material_is_graphite_not_black() {
        let theme = ConfigTheme {
            glass: GlassMode::Off,
            ..ConfigTheme::default()
        };
        let c = window_fill(&theme);
        assert!(c.r > 0.08 && c.r < 0.16);
    }

    #[test]
    fn faded_multiplies_alpha() {
        let c = faded(Color::from_rgba(1.0, 1.0, 1.0, 0.5), 0.5);
        assert!((c.a - 0.25).abs() < f32::EPSILON);
    }
}
