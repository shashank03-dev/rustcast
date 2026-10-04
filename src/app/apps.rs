//! This modules handles the logic for each "app" that rustcast can load
//!
//! An "app" is effectively, one of the results that rustcast returns when you search for something

use std::io::Cursor;

use iced::{
    Alignment,
    Length::Fill,
    font::Weight,
    widget::{
        Button, Row, Space, Text, container,
        image::{FilterMethod, Handle},
        text::Wrapping,
    },
};

use crate::{
    app::{Message, Page, RUSTCAST_DESC_NAME},
    clipboard::ClipBoardContentType,
    commands::Function,
    styles::{
        PRIMARY, SECONDARY, TERTIARY, favourite_button_style, label, result_button_style,
        result_row_container_style,
    },
    utils::icns_data_to_handle,
};

/// The rustcast icon bytes (PNG on Linux)
pub const ICNS_ICON: &[u8] = include_bytes!("../../docs/icon.png");

/// Size of the leading icon in a result row.
const ROW_ICON: f32 = 24.0;

/// The RustCast glyph (the mark without its tile) drawn inside the launcher:
/// a light arc for dark themes, a dark arc for light ones.
pub fn brand_glyph(theme: &crate::config::Theme) -> Handle {
    static DARK: std::sync::LazyLock<Handle> = std::sync::LazyLock::new(|| {
        Handle::from_bytes(include_bytes!("../../assets/icons/rustcast-glyph-dark.png").as_slice())
    });
    static LIGHT: std::sync::LazyLock<Handle> = std::sync::LazyLock::new(|| {
        Handle::from_bytes(include_bytes!("../../assets/icons/rustcast-glyph-light.png").as_slice())
    });
    if theme.is_light() {
        LIGHT.clone()
    } else {
        DARK.clone()
    }
}

/// macOS-style icons shown before file-search results.
pub const FOLDER_ICON_PNG: &[u8] = include_bytes!("../../assets/icons/folder.png");
pub const FILE_ICON_PNG: &[u8] = include_bytes!("../../assets/icons/file.png");

/// Decoded once and shared. Every directory result clones `FOLDER_ICON`, so
/// [`App::is_folder`] can recognise folders by handle identity (clones of one
/// `Handle::from_rgba` compare equal) and sort them ahead of files without
/// re-statting the filesystem.
pub static FOLDER_ICON: std::sync::LazyLock<Option<Handle>> =
    std::sync::LazyLock::new(|| icns_data_to_handle(FOLDER_ICON_PNG.to_vec()));
pub static FILE_ICON: std::sync::LazyLock<Option<Handle>> =
    std::sync::LazyLock::new(|| icns_data_to_handle(FILE_ICON_PNG.to_vec()));

/// The shared icon handle for a file-search result: the folder icon for
/// directories, the document icon for everything else.
pub fn file_result_icon(is_dir: bool) -> Option<Handle> {
    if is_dir {
        (*FOLDER_ICON).clone()
    } else {
        (*FILE_ICON).clone()
    }
}

/// The groups of the root search, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResultGroup {
    Commands,
    Emoji,
    Files,
}

impl ResultGroup {
    pub fn title(self) -> &'static str {
        match self {
            ResultGroup::Commands => "Apps & Commands",
            ResultGroup::Emoji => "Emoji",
            ResultGroup::Files => "Files",
        }
    }
}

/// Emoji listed in the root search before "Show all emoji".
pub const MAIN_SEARCH_EMOJI: usize = 3;
/// Files listed in the root search before "Search files".
pub const MAIN_SEARCH_FILES: usize = 4;

/// Each result with the section label drawn above it, if any. Labels mark
/// where a group starts, and only appear once the list mixes in emoji or
/// files — a plain app search stays a bare list.
pub fn row_layout(results: &[App]) -> Vec<(Option<ResultGroup>, &App)> {
    let labelled = results.iter().any(|a| a.group() != ResultGroup::Commands);
    let mut prev = None;
    results
        .iter()
        .map(|app| {
            let group = app.group();
            let header = (labelled && prev != Some(group)).then_some(group);
            prev = Some(group);
            (header, app)
        })
        .collect()
}

/// Scroll offset of row `index` in the root search, its label included.
pub fn row_offset(results: &[App], index: usize) -> f32 {
    row_layout(results)
        .iter()
        .take(index)
        .map(|(header, _)| {
            crate::app::RESULT_ROW_HEIGHT
                + if header.is_some() {
                    crate::app::SECTION_HEADER_HEIGHT
                } else {
                    0.
                }
        })
        .sum()
}

/// This tells each "App" what to do when it is clicked, whether it is a function, a message, or a display
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AppCommand {
    Function(Function),
    Message(Message),
    Display,
}

/// The main app struct, that represents an "App"
///
/// This struct represents a command that rustcast can perform, providing the rustcast
/// the data needed to search for the app, to display the app in search results, and to actually
/// "run" the app.
#[derive(Debug, Clone)]
pub struct App {
    pub ranking: i32,
    pub open_command: AppCommand,
    pub desc: String,
    pub icons: Option<iced::widget::image::Handle>,
    pub display_name: String,
    pub search_name: String,
}

impl PartialEq for App {
    fn eq(&self, other: &Self) -> bool {
        self.search_name == other.search_name
            && self.icons == other.icons
            && self.desc == other.desc
            && self.display_name == other.display_name
    }
}

impl App {
    /// True for file-search directory results, recognised by the shared folder
    /// icon handle (see [`file_result_icon`]). Used to sort folders ahead of
    /// files on the File-search page.
    pub fn is_folder(&self) -> bool {
        self.icons.is_some() && self.icons == *FOLDER_ICON
    }

    /// True for emoji results (see [`App::emoji_apps`]): no icon, and the
    /// command copies the displayed emoji itself.
    pub fn is_emoji(&self) -> bool {
        self.icons.is_none()
            && matches!(
                &self.open_command,
                AppCommand::Function(Function::CopyToClipboard(ClipBoardContentType::Text(t)))
                    if *t == self.display_name
            )
    }

    /// True for file-search results (a file or folder).
    pub fn is_file(&self) -> bool {
        self.icons.is_some() && (self.icons == *FOLDER_ICON || self.icons == *FILE_ICON)
    }

    /// The page a "Show all" row opens, if this is one.
    pub fn show_all_page(&self) -> Option<&Page> {
        match &self.open_command {
            AppCommand::Message(Message::SearchInPage(page, _)) => Some(page),
            _ => None,
        }
    }

    /// The root-search group this result belongs to.
    pub fn group(&self) -> ResultGroup {
        match self.show_all_page() {
            Some(Page::EmojiSearch) => ResultGroup::Emoji,
            Some(Page::FileSearch) => ResultGroup::Files,
            _ if self.is_emoji() => ResultGroup::Emoji,
            _ if self.is_file() => ResultGroup::Files,
            _ => ResultGroup::Commands,
        }
    }

    /// The closing row of a capped group: opens the full page for `query`.
    pub fn show_all(page: Page, query: &str) -> App {
        let name = match page {
            Page::EmojiSearch => "Show all emoji".to_string(),
            _ => format!("Search files for \u{201c}{query}\u{201d}"),
        };
        App {
            ranking: 0,
            open_command: AppCommand::Message(Message::SearchInPage(page, query.to_string())),
            desc: String::new(),
            icons: None,
            display_name: name,
            search_name: String::new(),
        }
    }

    pub fn new(name: String, icon: Option<Handle>, desc: String, command: AppCommand) -> Self {
        Self {
            ranking: 0,
            open_command: command,
            icons: icon,
            search_name: name.to_lowercase(),
            display_name: name,
            desc,
        }
    }
    /// A vec of all the emojis as App structs
    pub fn emoji_apps() -> Vec<App> {
        emojis::iter()
            .filter(|x| x.unicode_version() < emojis::UnicodeVersion::new(17, 13))
            .map(|x| App {
                ranking: 0,
                icons: None,
                display_name: x.to_string(),
                search_name: x.name().to_string(),
                open_command: AppCommand::Function(Function::CopyToClipboard(
                    ClipBoardContentType::Text(x.to_string()),
                )),
                desc: x.name().to_string(),
            })
            .collect()
    }
    /// This returns the basic apps that rustcast has, such as quiting rustcast and opening preferences
    pub fn basic_apps() -> Vec<App> {
        let app_version = option_env!("APP_VERSION").unwrap_or("Unknown Version");

        let icons = icns_data_to_handle(ICNS_ICON.to_vec());

        let ferris_handle =
            image::ImageReader::new(Cursor::new(include_bytes!("../../docs/ferris_rs.png")))
                .with_guessed_format()
                .unwrap()
                .decode()
                .ok()
                .map(|img| Handle::from_rgba(img.width(), img.height(), img.into_bytes()));

        vec![
            App {
                ranking: 0,
                open_command: AppCommand::Function(Function::OpenWebsite(
                    "https://ferris.rs".to_string(),
                )),
                icons: ferris_handle,
                desc: "Easter Egg".to_string(),
                display_name: "Ferris Plushies".to_string(),
                search_name: "ferris.rs".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Function(Function::Quit),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Quit RustCast".to_string(),
                search_name: "quit".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Function(Function::QuitAllApps),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Quit All Apps".to_string(),
                search_name: "quit all apps".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Message(Message::SwitchToPage(Page::Settings)),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Open RustCast Preferences".to_string(),
                search_name: "settings".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Message(Message::SwitchToPage(Page::EmojiSearch)),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Search for an Emoji".to_string(),
                search_name: "emoji".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Message(Message::SwitchToPage(Page::ClipboardHistory)),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Clipboard History".to_string(),
                search_name: "clipboard".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Message(Message::SwitchToPage(Page::FileSearch)),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Search for a file".to_string(),
                search_name: "file search".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Message(Message::ReloadConfig),
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: "Reload RustCast".to_string(),
                search_name: "refresh".to_string(),
            },
            App {
                ranking: 0,
                open_command: AppCommand::Display,
                desc: RUSTCAST_DESC_NAME.to_string(),
                icons: icons.clone(),
                display_name: format!("Current RustCast Version: {app_version}"),
                search_name: "version".to_string(),
            },
        ]
    }

    /// Screenshot, annotation and OCR commands.
    pub fn capture_apps() -> Vec<App> {
        let icons = icns_data_to_handle(ICNS_ICON.to_vec());
        let actions: &[(&str, &str, &str, u64)] = &[
            ("Capture Area", "screenshot capture area region", "area", 0),
            ("Capture Window", "screenshot capture window", "window", 0),
            (
                "Capture Full Screen",
                "screenshot capture full screen",
                "fullscreen",
                0,
            ),
            (
                "Quick Capture (copy instantly)",
                "screenshot quick capture copy",
                "quick",
                0,
            ),
            (
                "Copy Text from Screen (OCR)",
                "ocr copy text from screen extract read scan",
                "ocr",
                0,
            ),
            (
                "Copy Code from Screen",
                "ocr copy code from screen extract indentation",
                "ocr-code",
                0,
            ),
            (
                "Copy Table from Screen",
                "ocr copy table from screen extract spreadsheet csv",
                "ocr-table",
                0,
            ),
            (
                "Pick Colours from Screen",
                "colour color palette picker extract colors from screen hex",
                "palette",
                0,
            ),
            (
                "Compare Last Two Screenshots",
                "compare screenshots diff before after difference",
                "compare",
                0,
            ),
            (
                "Capture Area in 3 Seconds",
                "screenshot timer delay 3 seconds",
                "area",
                3,
            ),
            (
                "Capture Area in 5 Seconds",
                "screenshot timer delay 5 seconds",
                "area",
                5,
            ),
            (
                "Capture Area in 10 Seconds",
                "screenshot timer delay 10 seconds",
                "area",
                10,
            ),
        ];
        let mut apps: Vec<App> = actions
            .iter()
            .map(|(name, search, mode, delay)| App {
                ranking: 0,
                open_command: AppCommand::Function(Function::Capture {
                    mode: mode.to_string(),
                    delay: *delay,
                }),
                desc: "Screenshot".to_string(),
                icons: icons.clone(),
                display_name: name.to_string(),
                search_name: search.to_string(),
            })
            .collect();
        apps.push(App {
            ranking: 0,
            open_command: AppCommand::Function(Function::OpenRawUrl(
                crate::snap::load_config()
                    .screenshot
                    .save_dir()
                    .to_string_lossy()
                    .to_string(),
            )),
            desc: "Screenshot".to_string(),
            icons,
            display_name: "Open Screenshots Folder".to_string(),
            search_name: "screenshots folder open".to_string(),
        });
        apps
    }

    /// Window tiling actions (12 positions)
    pub fn window_apps() -> Vec<App> {
        use crate::platform::window::TilePosition;

        let icons = icns_data_to_handle(ICNS_ICON.to_vec());

        let actions: &[(&str, TilePosition)] = &[
            ("Left Half", TilePosition::LeftHalf),
            ("Right Half", TilePosition::RightHalf),
            ("Top Half", TilePosition::TopHalf),
            ("Bottom Half", TilePosition::BottomHalf),
            ("Top Left Quarter", TilePosition::TopLeft),
            ("Top Right Quarter", TilePosition::TopRight),
            ("Bottom Left Quarter", TilePosition::BottomLeft),
            ("Bottom Right Quarter", TilePosition::BottomRight),
            ("Left Third", TilePosition::LeftThird),
            ("Center Third", TilePosition::CenterThird),
            ("Right Third", TilePosition::RightThird),
            ("Maximize", TilePosition::Maximize),
        ];

        actions
            .iter()
            .map(|(name, pos)| App {
                ranking: 0,
                open_command: AppCommand::Function(Function::TileWindow(pos.clone())),
                desc: "Window Tiling".to_string(),
                icons: icons.clone(),
                display_name: name.to_string(),
                search_name: name.to_lowercase(),
            })
            .collect()
    }

    /// This renders the app into an iced element, allowing it to be displayed in the search results
    pub fn render(
        self,
        theme: crate::config::Theme,
        id_num: u32,
        focussed_id: u32,
        on_press: Option<Message>,
    ) -> iced::Element<'static, Message> {
        let focused = focussed_id == id_num;

        // One line, Raycast style: icon, title, and the kind of result as a
        // quiet trailing accessory ("Application", "Utility", a path…).
        let mut row = Row::new()
            .align_y(Alignment::Center)
            .width(Fill)
            .spacing(12)
            .padding([0, 10])
            .height(Fill);

        // Emoji rows: the emoji in the icon slot, its name as the title.
        let is_emoji = self.is_emoji();
        let (title, accessory) = if is_emoji {
            let mut name = self.desc.clone();
            if let Some(first) = name.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            // The "Emoji" section label already says what it is.
            (name, String::new())
        } else {
            (self.display_name.clone(), self.desc.clone())
        };

        let is_show_all = self.show_all_page().is_some();
        if is_show_all {
            row = row.push(
                container(Text::new("→").size(14).color(label(&theme, TERTIARY)))
                    .center_x(ROW_ICON)
                    .center_y(ROW_ICON),
            );
        } else if is_emoji {
            row = row.push(
                container(
                    Text::new(self.display_name.clone())
                        .font(iced::Font {
                            family: crate::fonts::emoji_family(),
                            ..iced::Font::DEFAULT
                        })
                        .size(if crate::fonts::has_apple_emoji() {
                            24
                        } else {
                            19
                        }),
                )
                .center_x(ROW_ICON)
                .center_y(ROW_ICON),
            );
        } else if theme.show_icons {
            let icon: iced::Element<'static, Message> = match &self.icons {
                Some(icon) => iced::widget::image(icon.clone())
                    .width(ROW_ICON)
                    .height(ROW_ICON)
                    .filter_method(FilterMethod::Linear)
                    .into(),
                None => Space::new().width(ROW_ICON).height(ROW_ICON).into(),
            };
            row = row.push(icon);
        }

        row = row.push(
            container(
                Text::new(title)
                    .font(crate::app::pages::ui::font(&theme, Weight::Medium))
                    .size(14)
                    .wrapping(Wrapping::None)
                    .color(label(&theme, if is_show_all { SECONDARY } else { PRIMARY })),
            )
            .width(Fill)
            .clip(true),
        );
        row = row.push(
            container(
                Text::new(accessory)
                    .font(theme.font())
                    .size(12)
                    .wrapping(Wrapping::None)
                    .color(label(&theme, TERTIARY)),
            )
            .max_width(220)
            .clip(true),
        );

        let name = self.search_name.clone();
        let theme_clone = theme.clone();
        let is_favourite = self.ranking == -1;
        // Only apps and commands can be favourited.
        if self.group() == ResultGroup::Commands {
            row = row.push(
                Button::new(Text::new("♥").size(12))
                    .on_press_with(move || Message::ToggleFavouriteApp(name.clone()))
                    .padding([4, 2])
                    .style(move |_, status| {
                        favourite_button_style(&theme_clone, status, is_favourite, focused)
                    }),
            );
        }

        let msg = on_press.or(match self.open_command.clone() {
            AppCommand::Function(func) => Some(Message::RunFunction(func)),
            AppCommand::Message(msg) => Some(msg),
            AppCommand::Display => None,
        });

        let theme_clone = theme.clone();

        let content = Button::new(row)
            .on_press_maybe(msg)
            .style(move |_, _| result_button_style(&theme_clone))
            .width(Fill)
            .height(Fill)
            .padding(0);

        // The selection pill is inset from the window edge; the outer
        // container gives the row its fixed height.
        container(
            container(content)
                .style(move |_| result_row_container_style(&theme, focused))
                .width(Fill)
                .height(Fill),
        )
        .id(format!("result-{}", id_num))
        .padding([1, 8])
        .width(Fill)
        .height(crate::app::RESULT_ROW_HEIGHT)
        .into()
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn command(name: &str) -> App {
        App::new(
            name.to_string(),
            None,
            "Utility".to_string(),
            AppCommand::Display,
        )
    }

    #[test]
    fn plain_command_search_has_no_section_labels() {
        let results = vec![command("a"), command("b")];
        assert!(row_layout(&results).iter().all(|(h, _)| h.is_none()));
    }

    #[test]
    fn mixed_search_labels_the_start_of_each_group() {
        let emoji = App::emoji_apps().into_iter().next().unwrap();
        assert!(emoji.is_emoji());
        let results = vec![
            command("a"),
            command("b"),
            emoji,
            App::show_all(Page::EmojiSearch, "sm"),
            App::show_all(Page::FileSearch, "sm"),
        ];
        let headers: Vec<_> = row_layout(&results).into_iter().map(|(h, _)| h).collect();
        assert_eq!(
            headers,
            vec![
                Some(ResultGroup::Commands),
                None,
                Some(ResultGroup::Emoji),
                None,
                Some(ResultGroup::Files),
            ]
        );
        let row = crate::app::RESULT_ROW_HEIGHT;
        let label = crate::app::SECTION_HEADER_HEIGHT;
        assert_eq!(row_offset(&results, 2), label + 2. * row);
        assert_eq!(row_offset(&results, 4), 2. * label + 4. * row);
    }
}
