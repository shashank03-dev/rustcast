//! This is the config file type definitions for rustcast
use std::{collections::HashMap, path::Path, sync::Arc};

use iced::{Font, font::Family, theme::Custom, widget::image::Handle};
use serde::{Deserialize, Serialize};

use crate::{
    app::{
        ToApp,
        apps::{App, AppCommand},
    },
    commands::Function,
    utils::handle_from_icns,
};

/// The main config struct (effectively the config file's "schema")
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub toggle_hotkey: String,
    pub clipboard_hotkey: String,
    pub screenshot_hotkey: String,
    pub recorder_hotkey: String,
    pub recorder: RecorderConfig,
    pub buffer_rules: Buffer,
    pub event_duration: u32,
    pub main_page: MainPage,
    pub start_at_login: bool,
    pub theme: Theme,
    pub placeholder: String,
    pub search_url: String,
    pub cbhist: bool,
    pub cbhist_paste_on_select: bool,
    pub show_trayicon: bool,
    pub shells: Vec<Shelly>,
    pub modes: HashMap<String, String>,
    pub aliases: HashMap<String, String>,
    pub search_dirs: Vec<String>,
    pub log_path: String,
    pub debounce_delay: u64,
}

impl Default for Config {
    /// The default config
    fn default() -> Self {
        Self {
            toggle_hotkey: "ALT+SPACE".to_string(),
            clipboard_hotkey: "SUPER+SHIFT+C".to_string(),
            screenshot_hotkey: "SUPER+SHIFT+S".to_string(),
            recorder_hotkey: "SUPER+SHIFT+R".to_string(),
            recorder: RecorderConfig::default(),
            buffer_rules: Buffer::default(),
            theme: Theme::default(),
            start_at_login: false,
            event_duration: 60,
            placeholder: String::from("Time to be productive!"),
            search_url: "https://duckduckgo.com/search?q=%s".to_string(),
            cbhist: true,
            cbhist_paste_on_select: false,
            show_trayicon: true,
            main_page: MainPage::default(),
            search_dirs: vec!["~".to_string()],
            log_path: "/tmp/rustcast.log".to_string(),
            modes: HashMap::new(),
            aliases: HashMap::new(),
            shells: vec![],
            debounce_delay: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default, Eq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum MainPage {
    Favourites,
    FrequentlyUsed,
    Events,
    #[default]
    Blank,
}

impl std::fmt::Display for MainPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MainPage::Blank => "Rustcast",
            MainPage::Favourites => "Favourites",
            MainPage::FrequentlyUsed => "Frequently Used",
            MainPage::Events => "Events",
        })
    }
}

/// The mode for the theme (dark, light, or follow system)
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
}

impl Default for ThemeMode {
    fn default() -> Self {
        ThemeMode::Dark
    }
}

impl ThemeMode {
    /// Return preset text and background colors for this mode.
    pub fn presets(&self, is_system_dark: bool) -> ((f32, f32, f32), (f32, f32, f32)) {
        match self {
            ThemeMode::Dark => (
                (0.95, 0.95, 0.96), // light text
                (0.0, 0.0, 0.0),    // dark background
            ),
            ThemeMode::Light => (
                (0.05, 0.05, 0.05), // dark text
                (0.95, 0.95, 0.96), // light background
            ),
            ThemeMode::System => {
                if is_system_dark {
                    ((0.95, 0.95, 0.96), (0.0, 0.0, 0.0))
                } else {
                    ((0.05, 0.05, 0.05), (0.95, 0.95, 0.96))
                }
            }
        }
    }
}

/// The settings you can set for the theme
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Theme {
    pub text_color: (f32, f32, f32),
    pub background_color: (f32, f32, f32),
    pub blur: bool,
    pub show_icons: bool,
    pub show_scroll_bar: bool,
    pub font: Option<String>,
    pub theme_mode: ThemeMode,
}

impl Default for Theme {
    fn default() -> Self {
        let (text, bg) = ThemeMode::Dark.presets(true);
        Self {
            text_color: text,
            background_color: bg,
            blur: false,
            show_icons: true,
            show_scroll_bar: false,
            font: None,
            theme_mode: ThemeMode::Dark,
        }
    }
}

impl From<Theme> for iced::Theme {
    fn from(value: Theme) -> Self {
        let palette = iced::theme::Palette {
            background: value.bg_color(),
            text: value.text_color(1.),
            primary: iced::Color {
                r: 0.22,
                g: 0.55,
                b: 0.96,
                a: 1.0,
            },
            danger: iced::Color {
                r: 0.95,
                g: 0.26,
                b: 0.21,
                a: 1.0,
            },
            warning: iced::Color {
                r: 1.0,
                g: 0.76,
                b: 0.03,
                a: 1.0,
            },
            success: iced::Color {
                r: 0.30,
                g: 0.69,
                b: 0.31,
                a: 1.0,
            },
        };
        iced::Theme::Custom(Arc::new(Custom::new("RustCast Theme".to_string(), palette)))
    }
}

impl Theme {
    /// Return the text color in the theme config of type [`iced::Color`]
    pub fn text_color(&self, opacity: f32) -> iced::Color {
        let theme = self.to_owned();
        iced::Color {
            r: theme.text_color.0,
            g: theme.text_color.1,
            b: theme.text_color.2,
            a: opacity,
        }
    }

    /// Return the background color in the theme config of type [`iced::Color`]
    pub fn bg_color(&self) -> iced::Color {
        iced::Color {
            r: self.background_color.0,
            g: self.background_color.1,
            b: self.background_color.2,
            a: 0.,
        }
    }

    /// Return the font in the theme config of type [`iced::Font`]
    pub fn font(&self) -> Font {
        let opt_font_name = self.font.clone();
        match opt_font_name {
            Some(font_name) => Font {
                family: Family::Name(font_name.leak()),
                ..Default::default()
            },
            None => Font {
                family: Family::SansSerif,
                ..Default::default()
            },
        }
    }
}

/// Settings for the built-in screen recorder.
///
/// - `aspect_lock`: every frame is fitted (letterboxed) into a fixed
///   `output_width`×`output_height` canvas, so resizing or re-shaping the locked
///   window never changes the video's dimensions. When off, the video keeps the
///   size the window had when recording started.
/// - Other windows can be *added* to a running locked recording; they are drawn
///   on top of the locked window where they really are (`picture_in_picture`
///   off) or as corner tiles (on).
/// - `keep_recording_when_minimized`: minimizing a locked window "ghosts" it
///   instead (invisible, click-through, behind other windows) so its content keeps
///   rendering and stays in the recording; activating it again brings it back.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct RecorderConfig {
    pub fps: u32,
    pub aspect_lock: bool,
    pub output_width: u32,
    pub output_height: u32,
    pub keep_recording_when_minimized: bool,
    pub show_cursor: bool,
    pub record_audio: bool,
    pub show_indicator: bool,
    /// Windows added to a locked recording are shown as tidy corner tiles
    /// instead of at their real position over the locked window.
    pub picture_in_picture: bool,
    pub output_dir: String,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            fps: 30,
            aspect_lock: true,
            output_width: 1920,
            output_height: 1080,
            keep_recording_when_minimized: true,
            show_cursor: true,
            record_audio: false,
            show_indicator: true,
            picture_in_picture: false,
            output_dir: "~/Videos/RustCast".to_string(),
        }
    }
}

impl RecorderConfig {
    /// Frames per second clamped to a sane range.
    pub fn fps(&self) -> u32 {
        self.fps.clamp(1, 120)
    }

    /// The fixed output size used when `aspect_lock` is on, rounded down to even
    /// numbers (H.264 / yuv420p requires even dimensions).
    pub fn output_size(&self) -> (u32, u32) {
        (
            (self.output_width.clamp(16, 7680)) & !1,
            (self.output_height.clamp(16, 4320)) & !1,
        )
    }

    /// The output directory with `~` expanded.
    pub fn output_dir(&self) -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        std::path::PathBuf::from(self.output_dir.replacen('~', &home, 1))
    }
}

/// The rules for the buffer AKA search results
///
/// - clear_on_hide is whether the buffer should be cleared when the window is hidden
/// - clear_on_enter is whether the buffer should be cleared when the user presses enter after
///   searching
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Buffer {
    pub clear_on_hide: bool,
    pub clear_on_enter: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer {
            clear_on_hide: true,
            clear_on_enter: true,
        }
    }
}

/// Command is the command it will run when the button is clicked
/// Icon_path is the path to an icon, but this is optional
/// Alias is the text that is used to call this command / search for it
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct Shelly {
    pub command: String,
    pub icon_path: Option<String>,
    pub alias: String,
    pub alias_lc: String,
    pub hotkey: Option<String>,
}

impl ToApp for Shelly {
    fn to_app(&self) -> App {
        let self_clone = self.clone();
        let icon = self_clone.icon_path.and_then(|x| {
            let x = x.replace("~", &std::env::var("HOME").unwrap());
            if x.ends_with(".icns") {
                handle_from_icns(Path::new(&x))
            } else {
                Some(Handle::from_path(Path::new(&x)))
            }
        });
        App {
            ranking: 0,
            open_command: AppCommand::Function(Function::RunShellCommand(self_clone.command)),
            desc: "Shell Command".to_string(),
            icons: icon,
            display_name: self_clone.alias,
            search_name: self_clone.alias_lc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_values_match_expected_defaults() {
        let config = Config::default();

        assert_eq!(config.toggle_hotkey, "ALT+SPACE");
        assert_eq!(config.clipboard_hotkey, "SUPER+SHIFT+C");
        assert_eq!(config.search_url, "https://duckduckgo.com/search?q=%s");
        assert_eq!(config.search_dirs, vec!["~".to_string()]);
        assert_eq!(config.debounce_delay, 300);
        assert_eq!(config.main_page, MainPage::Blank);
        assert_eq!(config.recorder_hotkey, "SUPER+SHIFT+R");
        assert!(config.recorder.aspect_lock);
    }

    #[test]
    fn recorder_output_size_is_even_and_clamped() {
        let rec = RecorderConfig {
            output_width: 1281,
            output_height: 3,
            ..RecorderConfig::default()
        };
        assert_eq!(rec.output_size(), (1280, 16));
        assert_eq!(
            RecorderConfig {
                fps: 0,
                ..rec.clone()
            }
            .fps(),
            1
        );
    }

    #[test]
    fn old_configs_without_recorder_fields_still_parse() {
        let cfg: Config = toml::from_str("toggle_hotkey = \"ALT+SPACE\"\n").unwrap();
        assert_eq!(cfg.recorder, RecorderConfig::default());
        assert_eq!(cfg.recorder_hotkey, "SUPER+SHIFT+R");
    }

    #[test]
    fn main_page_display_labels_are_stable() {
        assert_eq!(MainPage::Blank.to_string(), "Rustcast");
        assert_eq!(MainPage::Favourites.to_string(), "Favourites");
        assert_eq!(MainPage::FrequentlyUsed.to_string(), "Frequently Used");
    }
}
