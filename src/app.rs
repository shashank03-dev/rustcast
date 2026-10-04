//! Main logic for the app
use std::collections::HashMap;

use crate::app::apps::{App, AppCommand, ICNS_ICON};
use crate::commands::Function;
use crate::config::{Config, GlassMode, MainPage, Shelly, ThemeMode};
use crate::debounce::DebouncePolicy;
use crate::platform::launching::Shortcut;
use crate::utils::icns_data_to_handle;
use crate::{app::tile::ExtSender, clipboard::ClipBoardContentType};
use iced::time::Duration;

pub mod apps;
pub mod menubar;
pub mod pages;
pub mod screenshot;
pub mod tile;
pub mod tray;

use iced::window::{self, Id, Settings};
/// The default window width
pub const WINDOW_WIDTH: f32 = 640.;

/// The default window height
pub const DEFAULT_WINDOW_HEIGHT: f32 = 106.;

/// Height of one search result row, selection inset included.
pub const RESULT_ROW_HEIGHT: f32 = 44.;

/// Height of a section label ("Emoji", "Files") in the root search.
pub const SECTION_HEADER_HEIGHT: f32 = 28.;

/// Rows of the root search shown before the list scrolls.
pub const MAX_VISIBLE_ROWS: usize = 5;

/// Launcher height for the root search: the first rows with their section
/// labels (see [`apps::row_layout`]).
pub fn main_results_window_height(results: &[apps::App]) -> f32 {
    let list: f32 = apps::row_layout(results)
        .iter()
        .take(MAX_VISIBLE_ROWS)
        .map(|(header, _)| {
            RESULT_ROW_HEIGHT
                + if header.is_some() {
                    SECTION_HEADER_HEIGHT
                } else {
                    0.
                }
        })
        .sum();
    if list == 0. {
        DEFAULT_WINDOW_HEIGHT
    } else {
        DEFAULT_WINDOW_HEIGHT + list + 2. * RESULTS_LIST_PADDING
    }
}

/// Space above and below the results list.
pub const RESULTS_LIST_PADDING: f32 = 6.;

/// Launcher height showing `rows` result rows under the search field.
pub const fn results_window_height(rows: usize) -> f32 {
    if rows == 0 {
        DEFAULT_WINDOW_HEIGHT
    } else {
        DEFAULT_WINDOW_HEIGHT + rows as f32 * RESULT_ROW_HEIGHT + 2. * RESULTS_LIST_PADDING
    }
}

/// The clipboard history page gets a big, purpose-built window.
pub const CLIPBOARD_WIDTH: f32 = 860.;
pub const CLIPBOARD_HEIGHT: f32 = 600.;

/// The screen recorder page's window.
pub const RECORDER_WIDTH: f32 = 720.;
pub const RECORDER_HEIGHT: f32 = 580.;

static LAUNCHER_SIZE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Remember the launcher's current size so the platform code can centre it.
pub fn set_launcher_size(width: f32, height: f32) {
    let packed = ((width as u64) << 32) | (height as u64 & 0xffff_ffff);
    LAUNCHER_SIZE.store(packed, std::sync::atomic::Ordering::Relaxed);
}

/// The launcher's current (width, height), defaulting to the classic size.
pub fn launcher_size() -> (f32, f32) {
    let packed = LAUNCHER_SIZE.load(std::sync::atomic::Ordering::Relaxed);
    if packed == 0 {
        return (WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT);
    }
    ((packed >> 32) as f32, (packed & 0xffff_ffff) as f32)
}

/// Launcher width for a page.
pub fn page_width(page: &Page) -> f32 {
    match page {
        Page::ClipboardHistory => CLIPBOARD_WIDTH,
        Page::Recorder => RECORDER_WIDTH,
        _ => WINDOW_WIDTH,
    }
}

/// Fixed launcher height for pages that don't size to their results.
pub fn page_height(page: &Page) -> Option<f32> {
    match page {
        Page::ClipboardHistory => Some(CLIPBOARD_HEIGHT),
        Page::Recorder => Some(RECORDER_HEIGHT),
        _ => None,
    }
}

/// Which clipboard entries are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipFilter {
    #[default]
    All,
    Text,
    Images,
}

impl ClipFilter {
    pub const ALL: [ClipFilter; 3] = [ClipFilter::All, ClipFilter::Text, ClipFilter::Images];

    pub fn label(self) -> &'static str {
        match self {
            ClipFilter::All => "All",
            ClipFilter::Text => "Text",
            ClipFilter::Images => "Images",
        }
    }

    /// The next (or previous) filter, wrapping around.
    pub fn step(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    pub fn matches(self, item: &crate::clipboard::ClipBoardContentType, query_lc: &str) -> bool {
        use crate::clipboard::ClipBoardContentType as C;
        let kind_ok = matches!(
            (self, item),
            (ClipFilter::All, _)
                | (ClipFilter::Text, C::Text(_))
                | (ClipFilter::Images, C::Image(_))
        );
        kind_ok
            && (query_lc.is_empty()
                || match item {
                    C::Text(t) => t.to_lowercase().contains(query_lc),
                    C::Image(_) => "image".contains(query_lc),
                })
    }
}

/// Maximum file search results returned by a single mdfind invocation.
pub const FILE_SEARCH_MAX_RESULTS: u32 = 400;

/// Number of results to accumulate before flushing a batch to the UI.
pub const FILE_SEARCH_BATCH_SIZE: u32 = 10;

/// The rustcast descriptor name to be put for all rustcast commands
pub const RUSTCAST_DESC_NAME: &str = "Utility";

/// The different pages that rustcast can have / has
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Main,
    FileSearch,
    ClipboardHistory,
    EmojiSearch,
    Settings,
    Recorder,
}

/// The settings panel tabs
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsTab {
    General,
    Appearance,
    Recorder,
    Commands,
}

/// On/off options of the screen recorder, toggled from the recorder page or
/// the settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderOption {
    AspectLock,
    KeepWhenMinimized,
    ShowCursor,
    RecordAudio,
    ShowIndicator,
    PictureInPicture,
}

impl RecorderOption {
    pub fn get(self, cfg: &crate::config::RecorderConfig) -> bool {
        match self {
            RecorderOption::AspectLock => cfg.aspect_lock,
            RecorderOption::KeepWhenMinimized => cfg.keep_recording_when_minimized,
            RecorderOption::ShowCursor => cfg.show_cursor,
            RecorderOption::RecordAudio => cfg.record_audio,
            RecorderOption::ShowIndicator => cfg.show_indicator,
            RecorderOption::PictureInPicture => cfg.picture_in_picture,
        }
    }

    pub fn set(self, cfg: &mut crate::config::RecorderConfig, value: bool) {
        match self {
            RecorderOption::AspectLock => cfg.aspect_lock = value,
            RecorderOption::KeepWhenMinimized => cfg.keep_recording_when_minimized = value,
            RecorderOption::ShowCursor => cfg.show_cursor = value,
            RecorderOption::RecordAudio => cfg.record_audio = value,
            RecorderOption::ShowIndicator => cfg.show_indicator = value,
            RecorderOption::PictureInPicture => cfg.picture_in_picture = value,
        }
    }
}

/// Actions that open a native file dialog
#[derive(Debug, Clone)]
pub enum FileDialogAction {
    PickModeFile(String),
    EditSearchDir(String),
    AddSearchDir,
}

/// Config fields that can be individually reset to default
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResetField {
    ToggleHotkey,
    ClipboardHotkey,
    ScreenshotHotkey,
    OcrHotkey,
    OcrLanguages,
    ThumbnailSeconds,
    Placeholder,
    SearchUrl,
    DebounceDelay,
    StartAtLogin,
    ShowMenubarIcon,
    ClipboardHistory,
    ClipboardPasteOnSelect,
    MainPage,
    ShowScrollbar,
    ClearOnHide,
    ClearOnEnter,
    ShowIcons,
    Font,
    EventDuration,
    TextColor,
    BackgroundColor,
    ThemeMode,
    Glass,
    Aliases,
    Modes,
    SearchDirs,
    ShellCommands,
    RecorderHotkey,
    RecorderFps,
    RecorderOutputSize,
    RecorderOutputDir,
    RecorderOption(RecorderOption),
}

impl std::fmt::Display for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.to_owned() {
            Page::Main => "App search",
            Page::FileSearch => "File search",
            Page::EmojiSearch => "Emoji search",
            Page::ClipboardHistory => "Clipboard history",
            Page::Settings => "Settings",
            Page::Recorder => "Screen recorder",
        })
    }
}

/// The types of arrow keys
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ArrowKey {
    Up,
    Down,
    Left,
    Right,
}

/// The ways the cursor can move when a key is pressed
#[derive(Debug, Clone)]
pub enum Move {
    Back,
    Forwards(String),
}

#[derive(Debug, Clone)]
pub enum Editable<T> {
    Create(T),
    Delete(T),
    Update { old: T, new: T },
}

/// The message type that iced uses for actions that can do something
#[derive(Debug, Clone)]
pub enum Message {
    UriReceived(String),
    WriteConfig(bool),
    SaveRanking,
    ToggleAutoStartup(bool),
    LoadRanking,
    ToggleFavouriteApp(String),
    ResizeWindow(Id, f32),
    OpenWindow,
    OpenResult(u32),
    OpenToSettings,
    SearchQueryChanged(String, Id),
    KeyPressed(Shortcut),
    FocusTextInput(Move),
    HideWindow(Id),
    RunFunction(Function),
    OpenFocused,
    SetConfig(SetConfigFields),
    OpenFileDialog(FileDialogAction),
    FileDialogResult(Option<Box<Message>>),
    ReturnFocus,
    SwitchSettingsTab(SettingsTab),
    ResetField(ResetField),
    EscKeyPressed(Id),
    UpdateEvents,
    ClearSearchResults,
    WindowFocusChanged(Id, bool),
    ClearSearchQuery,
    HideTrayIcon,
    SwitchMode(String),
    ReloadConfig,
    UpdateApps,
    SetSender(ExtSender),
    SwitchToPage(Page),
    /// Open a page with the search field already set to the given query
    /// (the "Show all emoji" / "Search files" rows of the root search).
    SearchInPage(Page, String),
    EditClipboardHistory(Editable<ClipBoardContentType>),
    ClearClipboardHistory,
    ChangeFocus(ArrowKey, u32),
    FileSearchResult(Vec<App>),
    FileSearchClear,
    SetFileSearchSender(tokio::sync::watch::Sender<(String, Vec<String>)>),
    DebouncedSearch(Id),
    ThemeModeChanged(bool),
    SimulatePaste(i32),
    /// The recorder started/stopped/failed — refresh the recorder page.
    RecorderChanged,
    RecorderStart(crate::recorder::RecordTarget),
    RecorderStop,
    /// Bring another window into the running locked recording (or take it out).
    RecorderAddWindow(u32, String),
    RecorderRemoveWindow(u32),
    /// Show the recorder page (opening the launcher if needed).
    OpenRecorderPage,
    RecorderToggle(RecorderOption),
    /// Redraw for an animation frame.
    AnimationFrame,
    SetClipFilter(ClipFilter),
    /// Run several Jev steps in order.
    JevRunAll(Vec<Message>),
    /// Re-check a focus loss after a short delay; hide only if another app
    /// really took focus.
    ConfirmFocusLost(Id),
    /// Select a result without opening it (clicking a clipboard card).
    SelectResult(u32),
    /// The Jev model's reading of a query the parser couldn't place:
    /// (the query it answered, the action it chose).
    JevModelResult(String, Option<crate::jev::Intent>),
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum SetConfigFields {
    ToDefault,
    ToggleHotkey(String),
    ClipboardHotkey(String),
    ScreenshotHotkey(String),
    OcrHotkey(String),
    OcrLanguages(String),
    ThumbnailSeconds(String),
    PlaceHolder(String),
    SearchUrl(String),
    ClipboardHistory(bool),
    ShowMenubarIcon(bool),
    SetPage(MainPage),
    SetEventDuration(String),
    Modes(Editable<(String, String)>),
    Aliases(Editable<(String, String)>),
    SearchDirs(Editable<String>),
    ShellCommands(Editable<Shelly>),
    DebounceDelay(u64),
    SetThemeFields(SetConfigThemeFields),
    SetBufferFields(SetConfigBufferFields),
    ClipboardPasteOnSelect(bool),
    SetRecorderFields(SetConfigRecorderFields),
}

#[derive(Debug, Clone)]
pub enum SetConfigRecorderFields {
    Hotkey(String),
    Fps(u32),
    OutputWidth(u32),
    OutputHeight(u32),
    OutputDir(String),
    Option(RecorderOption, bool),
}

#[derive(Debug, Clone)]
pub enum SetConfigThemeFields {
    ShowScrollBar(bool),
    TextColor(f32, f32, f32),
    BackgroundColor(f32, f32, f32),
    ShowIcons(bool),
    Font(String),
    ThemeMode(ThemeMode),
    Glass(GlassMode),
}

#[derive(Debug, Clone)]
pub enum SetConfigBufferFields {
    ClearOnHide(bool),
    ClearOnEnter(bool),
}

/// The window settings for rustcast
pub fn default_settings() -> Settings {
    Settings {
        resizable: false,
        decorations: false,
        minimizable: false,
        level: window::Level::AlwaysOnTop,
        transparent: true,
        // winit's blur asks for the whole window rectangle, which shows
        // blurred square corners. The rounded region is requested per resize
        // instead, and only when the glass is translucent.
        blur: false,
        // Map centred on the active monitor so the window never flashes at the
        // window manager's default spot; `position_launcher` then nudges it up
        // to the Spotlight/Raycast upper-third anchor once it is mapped.
        position: window::Position::Centered,
        size: iced::Size {
            width: WINDOW_WIDTH,
            height: DEFAULT_WINDOW_HEIGHT,
        },
        // The RustCast mark as the window icon, and an application id that
        // matches rustcast.desktop so docks, the window switcher and GNOME's
        // top bar show "RustCast" with its icon rather than a generic entry.
        icon: window::icon::from_file_data(
            include_bytes!("../assets/icons/rustcast-256.png"),
            None,
        )
        .ok(),
        platform_specific: window::settings::PlatformSpecific {
            application_id: "rustcast".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A Trait to define that a struct can be converted to an app
pub trait ToApp {
    /// Convert self into an app
    fn to_app(&self) -> App;
}

/// A Trait to define that a type (containing multiple elements) can be converted to multiple Apps
///
/// i.e. [`Vec<Box<dyn ToApp>>`] can implement ToApps but it doesn't make sense to do that
pub trait ToApps {
    /// convert self into a Vec of apps
    fn to_apps(&self) -> Vec<App>;
}

/// [`HashMap<String, String>`] is for storing the modes, and is an assumtion that the String
/// values are shell commands
impl ToApps for HashMap<String, String> {
    fn to_apps(&self) -> Vec<App> {
        let icons = icns_data_to_handle(ICNS_ICON.to_vec());

        let mut to_apps: Vec<App> = self
            .keys()
            .map(|key| {
                let display_name = format!(
                    "{}{} Mode",
                    key.split_at(1).0.to_uppercase(),
                    key.split_at(1).1
                );
                App {
                    ranking: 0,
                    open_command: apps::AppCommand::Message(Message::SwitchMode(
                        key.trim().to_owned(),
                    )),
                    search_name: key.to_owned(),
                    desc: "Switch Modes".to_string(),
                    icons: icons.clone(),
                    display_name,
                }
            })
            .collect();

        if self.get("default").is_none() {
            to_apps.push(App {
                ranking: 0,
                open_command: AppCommand::Message(Message::SwitchMode("Default".to_string())),
                desc: "Change mode".to_string(),
                icons: icons.clone(),
                display_name: "Default mode".to_string(),
                search_name: "default".to_string(),
            });
        };

        to_apps
    }
}

impl DebouncePolicy for Page {
    fn debounce_delay(&self, config: &Config) -> Option<Duration> {
        match self {
            Page::Main | Page::ClipboardHistory | Page::Settings | Page::Recorder => None,
            Page::FileSearch | Page::EmojiSearch => {
                Some(Duration::from_millis(config.debounce_delay))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn page_display_labels_are_stable() {
        assert_eq!(Page::Main.to_string(), "App search");
        assert_eq!(Page::FileSearch.to_string(), "File search");
        assert_eq!(Page::ClipboardHistory.to_string(), "Clipboard history");
        assert_eq!(Page::EmojiSearch.to_string(), "Emoji search");
        assert_eq!(Page::Settings.to_string(), "Settings");
        assert_eq!(Page::Recorder.to_string(), "Screen recorder");
    }

    #[test]
    fn clip_filter_steps_and_matches() {
        use crate::clipboard::ClipBoardContentType as C;
        assert_eq!(ClipFilter::All.step(true), ClipFilter::Text);
        assert_eq!(ClipFilter::All.step(false), ClipFilter::Images);
        let text = C::Text("Hello World".to_string());
        assert!(ClipFilter::All.matches(&text, "world"));
        assert!(ClipFilter::Text.matches(&text, ""));
        assert!(!ClipFilter::Images.matches(&text, ""));
        assert!(!ClipFilter::All.matches(&text, "absent"));
    }

    #[test]
    fn recorder_options_round_trip() {
        let mut cfg = crate::config::RecorderConfig::default();
        for opt in [
            RecorderOption::AspectLock,
            RecorderOption::KeepWhenMinimized,
            RecorderOption::ShowCursor,
            RecorderOption::RecordAudio,
            RecorderOption::ShowIndicator,
            RecorderOption::PictureInPicture,
        ] {
            let before = opt.get(&cfg);
            opt.set(&mut cfg, !before);
            assert_eq!(opt.get(&cfg), !before);
        }
    }

    #[test]
    fn page_debounce_policy_matches_expected_pages() {
        let config = Config {
            debounce_delay: 123,
            ..Config::default()
        };

        assert_eq!(Page::Main.debounce_delay(&config), None);
        assert_eq!(Page::ClipboardHistory.debounce_delay(&config), None);
        assert_eq!(Page::Settings.debounce_delay(&config), None);
        assert_eq!(
            Page::FileSearch.debounce_delay(&config),
            Some(Duration::from_millis(123))
        );
        assert_eq!(
            Page::EmojiSearch.debounce_delay(&config),
            Some(Duration::from_millis(123))
        );
    }

    #[test]
    fn mode_to_apps_adds_default_when_missing() {
        let mut modes = HashMap::new();
        modes.insert("work".to_string(), "echo work".to_string());

        let apps = modes.to_apps();

        assert!(apps.iter().any(|app| app.search_name == "work"));
        assert!(apps.iter().any(|app| app.search_name == "default"));
    }

    #[test]
    fn mode_to_apps_does_not_duplicate_default() {
        let mut modes = HashMap::new();
        modes.insert("default".to_string(), "echo default".to_string());

        let apps = modes.to_apps();
        let default_count = apps
            .iter()
            .filter(|app| app.search_name == "default")
            .count();

        assert_eq!(default_count, 1);
    }
}
