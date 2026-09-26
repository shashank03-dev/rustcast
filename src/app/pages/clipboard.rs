//! The clipboard history page — a full, purpose-built view (Super+Shift+C).
//!
//! ```text
//! ┌ search ────────────────────────────────────────────────────────────┐
//! │ Clipboard history  12            [ All 12 ][ Text 9 ][ Images 3 ] ←→│
//! ├──────────────────────────┬─────────────────────────────────────────┤
//! │ ▍TEXT          Ctrl 1    │ TEXT  142 characters · 23 words · 3 lines│
//! │  first line of the clip  │                                         │
//! │ ┌IMAGE         Ctrl 2    │   full preview                          │
//! │ …                        │                                         │
//! │                          │ [ Copy ↵ ]              [Delete] [Clear]│
//! └──────────────────────────┴─────────────────────────────────────────┘
//! ```
//!
//! Cards slide in with a stagger, the selection glides between cards, and
//! ←/→ switch the filter. Typing filters the history.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Instant;

use iced::widget::image::{Handle, Image};
use iced::widget::text::Wrapping;
use iced::widget::{Scrollable, Space, column, row, scrollable, text};
use iced::{ContentFit, Length};
use once_cell::sync::Lazy;

use crate::app::pages::prelude::*;
use crate::app::pages::ui::{self, Tone};
use crate::app::tile::Motion;
use crate::app::{ClipFilter, Editable};
use crate::clipboard::ClipBoardContentType;
use crate::commands::Function;

/// Height of one history card.
pub const CARD_HEIGHT: f32 = 66.0;
/// Card height plus spacing — used to keep the selection scrolled into view.
pub const CARD_PITCH: f32 = CARD_HEIGHT + 8.0;
const LIST_WIDTH: f32 = 320.0;
/// Longest text shown in the preview (the full text is still copied).
const PREVIEW_LIMIT: usize = 20_000;

/// Image textures are cached per clipboard image: building a new handle every
/// frame would re-upload the texture on each animation tick.
static IMAGE_CACHE: Lazy<Mutex<HashMap<u64, Handle>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn image_key(img: &arboard::ImageData) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.width, img.height, img.bytes.len()).hash(&mut h);
    let step = (img.bytes.len() / 4096).max(1);
    for b in img.bytes.iter().step_by(step) {
        b.hash(&mut h);
    }
    h.finish()
}

fn image_handle(img: &arboard::ImageData) -> Handle {
    let key = image_key(img);
    let mut cache = IMAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() > 128 {
        cache.clear();
    }
    cache
        .entry(key)
        .or_insert_with(|| {
            Handle::from_rgba(img.width as u32, img.height as u32, img.bytes.to_vec())
        })
        .clone()
}

/// First non-empty line of a text clip, shortened for a card.
fn first_line(t: &str) -> String {
    let line = t
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        out.push('…');
    }
    out
}

fn text_stats(t: &str) -> String {
    let chars = t.chars().count();
    let words = t.split_whitespace().count();
    let lines = t.lines().count().max(1);
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    format!(
        "{} · {} · {}",
        plural(chars, "character"),
        plural(words, "word"),
        plural(lines, "line")
    )
}

/// Everything the page needs to render.
pub struct ClipboardPage<'a> {
    pub visible: Vec<(usize, &'a ClipBoardContentType)>,
    pub all: &'a [ClipBoardContentType],
    pub focus: u32,
    pub filter: ClipFilter,
    pub theme: Theme,
    pub motion: &'a Motion,
}

fn filter_count(all: &[ClipBoardContentType], f: ClipFilter) -> usize {
    all.iter().filter(|c| f.matches(c, "")).count()
}

fn filter_tabs(page: &ClipboardPage, fade: f32) -> Element<'static, Message> {
    let theme = page.theme.clone();
    let tabs = ClipFilter::ALL.iter().map(|&f| {
        let active = f == page.filter;
        let label = row![
            text(f.label()).size(12).font(theme.font()),
            text(filter_count(page.all, f).to_string())
                .size(11)
                .font(theme.font())
                .color(if active {
                    iced::Color::from_rgba(1.0, 1.0, 1.0, 0.75 * fade)
                } else {
                    theme.text_color(0.45 * fade)
                }),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let tone = if active { Tone::Primary } else { Tone::Quiet };
        ui::pill_button(label, tone, &theme, Some(Message::SetClipFilter(f)))
            .padding([5, 12])
            .into()
    });
    row(tabs).spacing(6).align_y(Alignment::Center).into()
}

fn header(page: &ClipboardPage, now: Instant) -> Element<'static, Message> {
    let t = ui::stagger(page.motion.page_since, now, 0);
    let theme = &page.theme;
    let title = row![
        text(crate::app::Page::ClipboardHistory.to_string())
            .size(18)
            .font(theme.font())
            .color(theme.text_color(t)),
        ui::badge(page.all.len(), ui::accent(1.0), theme, t),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    ui::enter(
        row![
            title,
            ui::spacer(),
            filter_tabs(page, t),
            ui::kbd("← →", theme, t)
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        t,
    )
}

fn card(
    page: &ClipboardPage,
    pos: usize,
    item: &ClipBoardContentType,
    now: Instant,
) -> Element<'static, Message> {
    let theme = &page.theme;
    let fade = ui::stagger(page.motion.page_since, now, pos + 1);
    let focus = page.motion.focus_amount(pos as u32, page.focus, now);

    let (kind, color) = match item {
        ClipBoardContentType::Text(_) => ("Text", ui::accent(1.0)),
        ClipBoardContentType::Image(_) => ("Image", ui::green(1.0)),
    };
    let mut top = row![ui::badge(kind, color, theme, fade), ui::spacer()]
        .align_y(Alignment::Center)
        .spacing(6);
    if pos < 9 {
        top = top.push(ui::kbd(format!("Ctrl {}", pos + 1), theme, fade));
    }

    let body: Element<'static, Message> = match item {
        ClipBoardContentType::Text(t) => column![
            text(first_line(t))
                .size(14)
                .font(theme.font())
                .wrapping(Wrapping::None)
                .color(theme.text_color(0.95 * fade)),
            text(text_stats(t))
                .size(11)
                .font(theme.font())
                .wrapping(Wrapping::None)
                .color(theme.text_color(0.45 * fade)),
        ]
        .spacing(1)
        .into(),
        ClipBoardContentType::Image(img) => row![
            Image::new(image_handle(img))
                .width(30)
                .height(30)
                .content_fit(ContentFit::Cover)
                .opacity(fade),
            text(format!("{} × {}", img.width, img.height))
                .size(13)
                .font(theme.font())
                .color(theme.text_color(0.85 * fade)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
    };

    let content = row![
        ui::focus_bar(focus, CARD_HEIGHT - 20.0),
        container(column![top, body].spacing(4))
            .width(Length::Fill)
            .clip(true),
    ]
    .spacing(10)
    .padding([8, 12])
    .height(CARD_HEIGHT);

    let card = ui::card_button(
        content,
        theme,
        focus,
        fade,
        Some(Message::OpenResult(pos as u32)),
    )
    .width(Length::Fill);
    ui::enter(card, fade)
}

fn preview(
    page: &ClipboardPage,
    item: &ClipBoardContentType,
    now: Instant,
) -> Element<'static, Message> {
    let theme = page.theme.clone();
    let fade = ui::stagger(page.motion.page_since, now, 2);
    // Cross-fade on selection change.
    let swap = ui::progress(
        page.motion.focus_since,
        now,
        0,
        200,
        iced::animation::Easing::EaseOutCubic,
    );
    let fade_in = fade * (0.35 + 0.65 * swap);

    let (kind, color, meta) = match item {
        ClipBoardContentType::Text(t) => ("Text", ui::accent(1.0), text_stats(t)),
        ClipBoardContentType::Image(img) => (
            "Image",
            ui::green(1.0),
            format!("{} × {} px", img.width, img.height),
        ),
    };
    let head = row![
        ui::badge(kind, color, &theme, fade),
        text(meta)
            .size(12)
            .font(theme.font())
            .color(theme.text_color(0.55 * fade)),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let body: Element<'static, Message> = match item {
        ClipBoardContentType::Text(t) => {
            let shown: String = t.chars().take(PREVIEW_LIMIT).collect();
            Scrollable::with_direction(
                container(
                    text(shown)
                        .size(15)
                        .font(theme.font())
                        .color(theme.text_color(0.95 * fade_in)),
                )
                .padding([4, 2])
                .width(Length::Fill),
                scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(3).scroller_width(3),
                ),
            )
            .height(Length::Fill)
            .into()
        }
        ClipBoardContentType::Image(img) => container(
            Image::new(image_handle(img))
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .opacity(fade_in),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into(),
    };

    let copy_label = row![
        text("Copy").size(13).font(theme.font()),
        ui::kbd("↵", &theme, 1.0)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let actions = row![
        ui::pill_button(
            copy_label,
            Tone::Primary,
            &theme,
            Some(Message::RunFunction(Function::CopyToClipboard(
                item.clone()
            ))),
        ),
        ui::spacer(),
        ui::pill_button(
            text("Delete").size(13).font(theme.font()),
            Tone::Danger,
            &theme,
            Some(Message::EditClipboardHistory(Editable::Delete(
                item.clone()
            ))),
        ),
        ui::pill_button(
            text("Clear").size(13).font(theme.font()),
            Tone::Quiet,
            &theme,
            Some(Message::ClearClipboardHistory),
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let t2 = theme.clone();
    ui::enter(
        container(
            column![head, body, actions]
                .spacing(12)
                .height(Length::Fill),
        )
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| ui::panel_style(&t2, None, fade)),
        fade,
    )
}

fn empty_state(theme: &Theme, message: String, hint: &str, fade: f32) -> Element<'static, Message> {
    let t2 = theme.clone();
    container(
        column![
            ui::glyph("⧉", ui::accent(1.0), 64.0, fade),
            text(message)
                .size(20)
                .font(theme.font())
                .center()
                .wrapping(Wrapping::WordOrGlyph)
                .color(theme.text_color(0.9 * fade)),
            text(hint.to_string())
                .size(12)
                .font(theme.font())
                .color(theme.text_color(0.45 * fade)),
        ]
        .spacing(14)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(move |_| ui::panel_style(&t2, None, fade))
    .into()
}

/// The whole clipboard page body (between the search field and the footer).
pub fn clipboard_page(page: ClipboardPage) -> Element<'static, Message> {
    let now = Instant::now();
    let theme = page.theme.clone();
    let fade = ui::stagger(page.motion.page_since, now, 1);

    let body: Element<'static, Message> = if page.all.is_empty() {
        empty_state(
            &theme,
            "Copy something to use the clipboard history".to_string(),
            "Text and images you copy appear here",
            fade,
        )
    } else if page.visible.is_empty() {
        empty_state(
            &theme,
            "No results found".to_string(),
            "← → change the filter",
            fade,
        )
    } else {
        let list = Scrollable::with_direction(
            column(
                page.visible
                    .iter()
                    .enumerate()
                    .map(|(pos, (_, item))| card(&page, pos, item, now)),
            )
            .spacing(CARD_PITCH - CARD_HEIGHT)
            .padding(iced::Padding {
                right: 6.0,
                ..iced::Padding::ZERO
            }),
            scrollable::Direction::Vertical(scrollable::Scrollbar::hidden()),
        )
        .id("results")
        .width(LIST_WIDTH)
        .height(Length::Fill);

        let selected = page
            .visible
            .get(page.focus as usize)
            .or(page.visible.first())
            .map(|(_, c)| *c);
        let right = match selected {
            Some(item) => preview(&page, item, now),
            None => Space::new().into(),
        };
        row![list, right].spacing(12).height(Length::Fill).into()
    };

    container(column![header(&page, now), body].spacing(12))
        .padding([14, 16])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_skips_blank_lines_and_shortens() {
        assert_eq!(first_line("\n\n  hello \nworld"), "hello");
        let long = "x".repeat(80);
        assert_eq!(first_line(&long).chars().count(), 61);
    }

    #[test]
    fn text_stats_pluralise() {
        assert_eq!(text_stats("hi"), "2 characters · 1 word · 1 line");
        assert_eq!(text_stats("a b\nc"), "5 characters · 3 words · 2 lines");
    }
}
