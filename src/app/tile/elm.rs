//! This module handles the logic for the new and view functions according to the elm
//! architecture. If the subscription function becomes too large, it should be moved to this file

use std::collections::HashMap;
use std::fs;

use iced::widget::scrollable::{Anchor, Direction, Scrollbar};
use iced::widget::text::LineHeight;
use iced::widget::{Column, Row, Scrollable, Text, container, space, stack};
use iced::{Alignment, window};
use iced::{Element, Task};
use iced::{Length::Fill, widget::text_input};

use log::info;
use rayon::iter::ParallelIterator;
use rayon::slice::ParallelSliceMut;

use crate::app::pages::emoji::emoji_page;
use crate::app::pages::settings::settings_page;
use crate::app::tile::{AppIndex, Hotkeys};
use crate::app::{DEFAULT_WINDOW_HEIGHT, SettingsTab, ToApp, ToApps};
use crate::config::Theme;
use crate::debounce::Debouncer;
use crate::platform::events::Event;
use crate::styles::{
    PRIMARY, SECONDARY, WINDOW_RADIUS, contents_style, fill, label, results_scrollbar_style,
    rustcast_text_input_style, separator, sheen,
};
use crate::{app::pages::clipboard::clipboard_page, platform::get_installed_apps};
use crate::{
    app::{Message, Page, apps::App, tile::Tile},
    config::Config,
};

/// Initialise the app. RustCast starts in the background (tray only) — no
/// launcher window is shown until the user triggers the toggle hotkey, which
/// opens, configures, and focuses a window via `open_window`.
pub fn new(hotkeys: Hotkeys, config: &Config) -> (Tile, Task<Message>) {
    info!("Starting in background (tray only)");

    let events = Event::get_events(config.event_duration);

    let store_icons = config.theme.show_icons;

    let mut options = get_installed_apps(store_icons);

    options.extend(config.shells.iter().map(|x| x.to_app()));
    info!("Loaded shell commands");

    options.extend(config.modes.to_apps());
    info!("Loaded modes");

    options.extend(App::basic_apps());
    info!("Loaded basic apps / default apps");
    options.extend(App::window_apps());
    info!("Loaded window tiling apps");
    options.par_sort_by_key(|x| x.display_name.len());
    let options = AppIndex::from_apps(options);

    let home = std::env::var("HOME").unwrap_or("/".to_string());

    let ranking = toml::from_str(
        &fs::read_to_string(home + "/.config/rustcast/ranking.toml").unwrap_or("".to_string()),
    )
    .unwrap_or(HashMap::new());

    crate::platform::urlscheme::install();

    (
        Tile {
            current_mode: "Default".to_string(),
            query: String::new(),
            query_lc: String::new(),
            focus_id: 0,
            results: vec![],
            options,
            hotkeys,
            events,
            emoji_apps: AppIndex::from_apps(App::emoji_apps()),
            visible: false,
            frontmost: None,
            focused: false,
            last_open: None,
            config: config.clone(),
            ranking,
            theme: config.theme.to_owned().clone().into(),
            clipboard_content: crate::persist::load_history(),
            tray: None,
            sender: None,
            page: Page::Main,
            height: DEFAULT_WINDOW_HEIGHT,
            file_search_sender: None,
            file_dialog_open: false,
            settings_tab: SettingsTab::General,
            debouncer: Debouncer::new(config.debounce_delay),
            clip_filter: crate::app::ClipFilter::All,
            motion: crate::app::tile::Motion::default(),
        },
        Task::none(),
    )
}

/// The elm View function that renders the entire rustcast window
pub fn view(tile: &Tile, wid: window::Id) -> Element<'_, Message> {
    if tile.visible {
        let title_input = text_input(tile.config.placeholder.as_str(), &tile.query)
            .on_input(move |a| Message::SearchQueryChanged(a, wid))
            .on_paste(move |a| Message::SearchQueryChanged(a, wid))
            // Spotlight's field: 22pt SF Pro Display on the bare material.
            // 20 + 28 + 19 plus the 1px separator keeps the 68px header.
            .font(crate::app::pages::ui::display_font(
                &tile.config.theme,
                iced::font::Weight::Normal,
            ))
            .size(22)
            .on_submit(Message::OpenFocused)
            .id("query")
            .width(Fill)
            .line_height(LineHeight::Absolute(28.into()))
            .style(move |_, _| rustcast_text_input_style(&tile.config.theme))
            .padding(iced::Padding {
                top: 20.,
                bottom: 19.,
                left: 12.,
                right: 20.,
            });
        // The RustCast mark leads the field, where Spotlight has its glass.
        let title_input = Row::new()
            .push(
                iced::widget::image(crate::app::apps::brand_glyph(&tile.config.theme))
                    .width(20)
                    .height(20),
            )
            .push(title_input)
            .padding(iced::Padding {
                left: 20.,
                ..iced::Padding::ZERO
            })
            .align_y(Alignment::Center);

        let scrollbar_direction =
            if !tile.config.theme.show_scroll_bar || tile.page == Page::Settings {
                Direction::Vertical(Scrollbar::hidden())
            } else {
                Direction::Vertical(
                    Scrollbar::new()
                        .width(1)
                        .scroller_width(1.1)
                        .anchor(Anchor::Start),
                )
            };

        let results = match tile.page {
            // Purpose-built pages render their own body below.
            Page::ClipboardHistory | Page::Recorder => space().into(),
            Page::EmojiSearch => emoji_page(
                tile.config.theme.clone(),
                tile.emoji_apps
                    .search_prefix(&tile.query_lc)
                    .map(|x| x.to_owned())
                    .collect(),
                tile.focus_id,
            ),
            Page::Settings => settings_page(tile.config.clone(), tile.settings_tab),
            Page::FileSearch | Page::Main => container(Column::from_iter(
                crate::app::apps::row_layout(&tile.results)
                    .into_iter()
                    .enumerate()
                    .flat_map(|(i, (header, app))| {
                        // Section labels only group the root search.
                        let header = header
                            .filter(|_| tile.page == Page::Main)
                            .map(|group| section_header(group.title(), &tile.config.theme));
                        header.into_iter().chain(std::iter::once(app.clone().render(
                            tile.config.theme.clone(),
                            i as u32,
                            tile.focus_id,
                            Some(Message::OpenResult(i as u32)),
                        )))
                    }),
            ))
            .padding([crate::app::RESULTS_LIST_PADDING, 0.])
            .into(),
        };

        let results_count = match &tile.page {
            Page::Main | Page::EmojiSearch | Page::FileSearch | Page::Recorder => {
                tile.results.len()
            }
            Page::ClipboardHistory => tile.clipboard_visible().len(),
            Page::Settings => 0,
        };

        let theme = tile.config.theme.clone();
        let body: Element<'_, Message> = match tile.page {
            Page::ClipboardHistory => page_surface(
                clipboard_page(crate::app::pages::clipboard::ClipboardPage {
                    visible: tile.clipboard_visible(),
                    all: &tile.clipboard_content,
                    focus: tile.focus_id,
                    filter: tile.clip_filter,
                    theme: tile.config.theme.clone(),
                    motion: &tile.motion,
                }),
                &tile.config.theme,
            ),
            Page::Recorder => page_surface(
                crate::app::pages::recorder::recorder_page(
                    &tile.results,
                    &tile.config.recorder,
                    tile.focus_id,
                    &tile.config.theme,
                    &tile.motion,
                ),
                &tile.config.theme,
            ),
            // The window is sized to the rows it shows, so the list takes
            // whatever is left between the field and the footer.
            _ => Scrollable::with_direction(results, scrollbar_direction)
                .style(move |_, _| results_scrollbar_style(&theme))
                .id("results")
                .height(Fill)
                .into(),
        };

        let status = if tile.query_lc.is_empty() {
            match &tile.page {
                // The footer already names RustCast on the left.
                Page::Main => String::new(),
                page => page.to_string(),
            }
        } else {
            match results_count {
                1 => "1 result".to_string(),
                0 => "No results".to_string(),
                count => format!("{count} results"),
            }
        };
        let focused_is_emoji = tile
            .results
            .get(tile.focus_id as usize)
            .is_some_and(|app| app.is_emoji());
        let action = match tile.page {
            Page::Main if focused_is_emoji => Some("Copy"),
            Page::Main | Page::FileSearch if results_count > 0 => Some("Open"),
            Page::EmojiSearch if results_count > 0 => Some("Copy"),
            _ => None,
        };

        let has_body = !matches!(tile.page, Page::Main | Page::FileSearch) || results_count > 0;
        let theme = &tile.config.theme;
        let contents = Column::new()
            .push(title_input)
            // The field's separator only shows when something sits under it;
            // otherwise the footer's own separator is enough.
            .push(if has_body {
                hairline(theme)
            } else {
                space().height(1).into()
            })
            .push(body)
            .push(footer(theme.clone(), &tile.current_mode, status, action))
            .spacing(0);

        // The top-edge highlight floats over the content so it takes no
        // layout space.
        let sheen_theme = theme.clone();
        let sheen = container(
            container(space().width(Fill).height(1))
                .style(move |_| container::Style {
                    background: Some(sheen(&sheen_theme)),
                    ..Default::default()
                })
                .width(Fill),
        )
        .padding(iced::Padding {
            top: 1.,
            left: WINDOW_RADIUS,
            right: WINDOW_RADIUS,
            bottom: 0.,
        })
        .width(Fill);

        container(stack![contents, sheen])
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(|_| contents_style(&tile.config.theme))
            .into()
    } else {
        space().into()
    }
}

/// A group label in the root search ("Apps & Commands", "Emoji", "Files"):
/// small, semibold, tertiary, aligned with the row titles' inset.
fn section_header<'a>(title: &'static str, theme: &Theme) -> Element<'a, Message> {
    container(
        Text::new(title)
            .size(11)
            .font(crate::app::pages::ui::font(
                theme,
                iced::font::Weight::Semibold,
            ))
            .color(label(theme, crate::styles::TERTIARY)),
    )
    .padding(iced::Padding {
        top: 10.,
        bottom: 4.,
        left: 18.,
        right: 18.,
    })
    .height(crate::app::SECTION_HEADER_HEIGHT)
    .width(Fill)
    .align_y(Alignment::End)
    .into()
}

/// A 1px separatorColor line across the window.
fn hairline<'a>(theme: &Theme) -> Element<'a, Message> {
    let theme = theme.clone();
    container(space().width(Fill).height(1))
        .width(Fill)
        .height(1)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(separator(&theme))),
            ..Default::default()
        })
        .into()
}

/// A purpose-built page body, filling the space between the search field
/// and the footer. It draws on the window material.
fn page_surface<'a>(content: Element<'a, Message>, _theme: &Theme) -> Element<'a, Message> {
    container(content).width(Fill).height(Fill).into()
}

/// The action bar along the bottom (Raycast style): the RustCast mark and the
/// current mode on the left; the result status, or the primary action with
/// its key, on the right.
fn footer(
    theme: Theme,
    current_mode: &str,
    status: String,
    action: Option<&'static str>,
) -> Element<'static, Message> {
    use crate::app::pages::ui;
    let mode = if current_mode.eq_ignore_ascii_case("default") {
        "RustCast".to_string()
    } else {
        let (first, rest) = current_mode.split_at(1);
        format!("{}{} Mode", first.to_uppercase(), rest)
    };
    let caption = |s: String, opacity: f32| {
        Text::new(s)
            .size(12)
            .color(label(&theme, opacity))
            .font(ui::font(&theme, iced::font::Weight::Medium))
    };
    let right: Element<'static, Message> = match action {
        Some(action) => Row::new()
            .push(caption(action.to_string(), PRIMARY))
            .push(ui::kbd("↵", &theme, 1.0))
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        None => caption(status, SECONDARY).into(),
    };
    let bar_theme = theme.clone();
    Column::new()
        .push(hairline(&theme))
        .push(
            container(
                Row::new()
                    .push(
                        iced::widget::image(crate::app::apps::brand_glyph(&theme))
                            .width(14)
                            .height(14),
                    )
                    .push(caption(mode, SECONDARY))
                    .push(space().width(Fill))
                    .push(right)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(Fill),
            )
            .padding([0, 14])
            .center_y(35)
            .width(Fill)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(fill(&bar_theme, 0.025))),
                ..Default::default()
            }),
        )
        .height(36)
        .into()
}
