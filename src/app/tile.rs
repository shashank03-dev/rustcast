//! This module handles the logic for the tile, AKA rustcast's main window
pub mod elm;
pub mod update;

use crate::app::apps::App;
use crate::app::{ArrowKey, Message, Move, Page};
use crate::clipboard::ClipBoardContentType;
use crate::config::{Config, Shelly};
use crate::debounce::Debouncer;
use crate::platform::default_app_paths;
use crate::platform::events::Event;
use crate::platform::launching::{EventTapHandle, Shortcut};

use arboard::Clipboard;

use iced::futures::SinkExt;
use iced::futures::channel::mpsc::{Sender, channel};
use iced::keyboard::Modifiers;
use iced::{
    Subscription, Theme, futures,
    keyboard::{self, key::Named},
    stream,
};
use iced::{event, window};

use log::{info, warn};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;

use crate::app::tray::TrayHandle;

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// This is a wrapper around the sender to disable dropping
#[derive(Clone, Debug)]
pub struct ExtSender(pub Sender<Message>);

/// Disable dropping the sender
impl Drop for ExtSender {
    fn drop(&mut self) {}
}

/// All the indexed apps that rustcast can search for
#[derive(Clone, Debug)]
struct AppIndex {
    by_name: HashMap<String, App>,
}

impl AppIndex {
    /// Search for an element in the index that starts with the provided prefix
    fn search_prefix<'a>(&'a self, prefix: &'a str) -> impl ParallelIterator<Item = &'a App> + 'a {
        self.by_name.par_iter().filter_map(move |(name, app)| {
            if name.starts_with(prefix)
                || name.contains(format!(" {prefix}").as_str())
                || name.contains(format!("-{prefix}").as_str())
            {
                Some(app)
            } else {
                None
            }
        })
    }

    fn update_ranking(&mut self, name: &str) {
        let app = match self.by_name.get_mut(name) {
            Some(a) => a,
            None => return,
        };

        app.ranking += 1;
    }

    fn set_ranking(&mut self, name: &str, rank: i32) {
        let app = match self.by_name.get_mut(name) {
            Some(a) => a,
            None => return,
        };

        app.ranking = rank;
    }

    fn get_rankings(&self) -> HashMap<String, i32> {
        HashMap::from_iter(self.by_name.iter().filter_map(|(name, app)| {
            if app.ranking > 0 {
                Some((name.to_owned(), app.ranking.to_owned()))
            } else {
                None
            }
        }))
    }

    fn top_ranked(&self, limit: usize) -> Vec<App> {
        let mut ranked: Vec<App> = self
            .by_name
            .values()
            .filter(|app| app.ranking > 0)
            .cloned()
            .collect();

        ranked.par_sort_by(|left, right| {
            right
                .ranking
                .cmp(&left.ranking)
                .then_with(|| left.display_name.cmp(&right.display_name))
        });
        ranked.truncate(limit);
        ranked
    }

    fn get_favourites(&self) -> Vec<App> {
        let mut favs: Vec<App> = self
            .by_name
            .values()
            .filter(|x| x.ranking == -1)
            .cloned()
            .collect();
        favs.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        favs
    }

    fn empty() -> AppIndex {
        AppIndex {
            by_name: HashMap::new(),
        }
    }

    /// Factory function for creating
    pub fn from_apps(options: Vec<App>) -> Self {
        let mut hmap = HashMap::new();
        for app in options {
            hmap.insert(app.search_name.clone(), app);
        }

        AppIndex { by_name: hmap }
    }
}

/// How often the background file index is rebuilt so newly-created files become
/// searchable. Searching itself is instant (in-memory); this only bounds how
/// stale the index can get.
const INDEX_REFRESH: Duration = Duration::from_secs(90);

/// Directory basenames pruned from the file index: caches and build artefacts
/// that dominate the filesystem walk and are virtually never search targets.
/// Hidden config dirs (e.g. `~/.config`) are deliberately *not* listed, so their
/// non-dotfile contents stay searchable — matching the previous `find` behaviour.
const PRUNED_DIRS: &[&str] = &[
    ".git",
    ".cache",
    ".cargo",
    ".rustup",
    ".npm",
    ".gradle",
    ".m2",
    ".nuget",
    ".nvm",
    ".pyenv",
    ".rbenv",
    ".mozilla",
    ".steam",
    ".var",
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    ".venv",
    ".Trash",
    "snap",
];

/// One entry in the in-memory file index.
#[derive(Clone, Debug, PartialEq)]
struct IndexEntry {
    is_dir: bool,
    /// Absolute path on disk.
    path: String,
}

/// Build the `find` argument list that walks `dirs` once (pruning [`PRUNED_DIRS`])
/// and prints every remaining entry as `"<type>\t<path>"`, e.g. `"d\t/home/u/Music"`.
/// Used to (re)build the in-memory index; query matching is then done in memory.
/// `~` is expanded to the home directory.
fn build_index_args(dirs: &[String], home_dir: &str) -> Vec<String> {
    let roots: Vec<String> = if dirs.is_empty() {
        vec![home_dir.to_string()]
    } else {
        dirs.iter().map(|dir| dir.replace("~", home_dir)).collect()
    };

    // ( -type d ( -name .git -o -name node_modules ... ) -prune ) -o -printf ...
    let mut args = roots;
    args.push("(".to_string());
    args.push("-type".to_string());
    args.push("d".to_string());
    args.push("(".to_string());
    for (i, name) in PRUNED_DIRS.iter().enumerate() {
        if i > 0 {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*name).to_string());
    }
    args.push(")".to_string());
    args.push("-prune".to_string());
    args.push(")".to_string());
    args.push("-o".to_string());
    args.push("-printf".to_string());
    args.push("%y\t%p\n".to_string());
    args
}

/// Parse one `find -printf '%y\t%p\n'` line into an [`IndexEntry`], skipping
/// dotfiles (basename starting with `.`) to mirror the search's own filtering.
fn parse_index_line(line: &str) -> Option<IndexEntry> {
    let record = line.trim_end_matches('\n');
    let (kind, path) = record.split_once('\t')?;
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    if name.starts_with('.') {
        return None;
    }
    Some(IndexEntry {
        is_dir: kind == "d",
        path: path.to_string(),
    })
}

/// Case-insensitive substring test where `needle` is already lowercase. Avoids
/// per-call allocation on the common ASCII path (millions of these run per query).
fn contains_ci(haystack: &str, needle_lc: &str) -> bool {
    if needle_lc.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle_lc.is_ascii() {
        let (hb, nb) = (haystack.as_bytes(), needle_lc.as_bytes());
        if hb.len() < nb.len() {
            return false;
        }
        'outer: for start in 0..=hb.len() - nb.len() {
            for (j, &n) in nb.iter().enumerate() {
                if hb[start + j].to_ascii_lowercase() != n {
                    continue 'outer;
                }
            }
            return true;
        }
        false
    } else {
        haystack.to_lowercase().contains(needle_lc)
    }
}

/// Filter the index for entries whose basename contains `query_lc` (already
/// lowercased, mirroring `find -iname`), ordered folders-first then by path, and
/// capped at [`crate::app::FILE_SEARCH_MAX_RESULTS`]. The scan and sort run in
/// parallel, so even a 500k-entry index resolves in a few milliseconds.
fn filter_index(index: &[IndexEntry], query_lc: &str, home_dir: &str) -> Vec<App> {
    use crate::app::FILE_SEARCH_MAX_RESULTS;

    let mut hits: Vec<&IndexEntry> = index
        .par_iter()
        .filter(|e| {
            let name = e.path.rsplit('/').next().unwrap_or(&e.path);
            contains_ci(name, query_lc)
        })
        .collect();

    // Folders before files; then alphabetical by path for stable, predictable order.
    hits.par_sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.path.cmp(&b.path)));
    hits.truncate(FILE_SEARCH_MAX_RESULTS as usize);

    hits.into_iter()
        .filter_map(|e| crate::commands::path_to_app(&e.path, home_dir, e.is_dir))
        .collect()
}

/// This is the base window, and its a "Tile"
/// Its fields are:
/// - Theme ([`iced::Theme`])
/// - Focus "ID" (which element in the choices is currently selected)
/// - Query (String)
/// - Query Lowercase (String, but lowercase)
/// - Previous Query Lowercase (String)
/// - Results (Vec<[`App`]>) the results of the search
/// - Options ([`AppIndex`]) the options to search through (is a HashMap wrapper)
/// - Emoji Apps ([`AppIndex`]) emojis that are considered as "apps"
/// - Visible (bool) whether the window is visible or not
/// - Focused (bool) whether the window is focused or not
/// - Frontmost ([`Option<Retained<NSRunningApplication>>`]) the frontmost application before the window was opened
/// - Config ([`Config`]) the app's config
/// - Hotkeys, storing the hotkey used for directly opening to the clipboard history page, and
///   opening the app
/// - Sender (The [`ExtSender`] that sends messages, used by the tray icon currently)
/// - Clipboard Content (`Vec<`[`ClipBoardContentType`]`>`) all of the cliboard contents
/// - Page ([`Page`]) the current page of the window (main or clipboard history)
/// - RustCast's height: to figure out which height to resize to
#[derive(Clone)]
pub struct Tile {
    pub theme: iced::Theme,
    pub focus_id: u32,
    pub query: String,
    pub current_mode: String,
    pub ranking: HashMap<String, i32>,
    query_lc: String,
    results: Vec<App>,
    options: AppIndex,
    emoji_apps: AppIndex,
    visible: bool,
    focused: bool,
    /// When the launcher was last opened by a user hotkey. Used to ignore the
    /// spurious focus-out that XWayland delivers right after the window maps, so
    /// a user-triggered window doesn't flash-and-hide before it can be used.
    last_open: Option<std::time::Instant>,
    pub events: Vec<Event>,
    /// X11 window id of the window that was focused before RustCast opened, so
    /// focus (and paste) can be restored to it.
    frontmost: Option<u32>,
    pub config: Config,
    hotkeys: Hotkeys,
    clipboard_content: Vec<ClipBoardContentType>,
    tray: Option<TrayHandle>,
    sender: Option<ExtSender>,
    page: Page,
    pub height: f32,
    pub file_search_sender: Option<tokio::sync::watch::Sender<(String, Vec<String>)>>,
    pub file_dialog_open: bool,
    pub settings_tab: crate::app::SettingsTab,
    debouncer: Debouncer,
    pub clip_filter: crate::app::ClipFilter,
    pub motion: Motion,
}

/// Timestamps driving the page animations (see `pages::ui`).
#[derive(Clone, Debug)]
pub struct Motion {
    /// When the current page appeared (staggered entrance).
    pub page_since: std::time::Instant,
    /// When the selection last moved, and where from.
    pub focus_since: std::time::Instant,
    pub prev_focus: u32,
    /// The last flipped recorder switch.
    pub toggled: Option<(crate::app::RecorderOption, std::time::Instant)>,
}

impl Default for Motion {
    fn default() -> Self {
        let long_ago = std::time::Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(std::time::Instant::now);
        Motion {
            page_since: long_ago,
            focus_since: long_ago,
            prev_focus: 0,
            toggled: None,
        }
    }
}

impl Motion {
    /// Whether anything is still moving (so frames should be requested).
    pub fn active(&self, now: std::time::Instant) -> bool {
        now.duration_since(self.page_since) < Duration::from_millis(700)
            || now.duration_since(self.focus_since) < Duration::from_millis(260)
            || self
                .toggled
                .is_some_and(|(_, t)| now.duration_since(t) < Duration::from_millis(420))
    }

    /// Selection highlight (0‥1) for item `i` given the current focus.
    pub fn focus_amount(&self, i: u32, focus: u32, now: std::time::Instant) -> f32 {
        let t = crate::app::pages::ui::progress(
            self.focus_since,
            now,
            0,
            180,
            iced::animation::Easing::EaseOutCubic,
        );
        if i == focus {
            t
        } else if i == self.prev_focus && self.prev_focus != focus {
            1.0 - t
        } else {
            0.0
        }
    }
}

/// A struct to store all the hotkeys
///
/// Stores the toggle [`HotKey`] and the Clipboard [`HotKey`]
#[derive(Clone, Debug)]
pub struct Hotkeys {
    pub handle: Option<EventTapHandle>,
    pub toggle: Shortcut,
    pub clipboard_hotkey: Shortcut,
    pub screenshot_hotkey: Shortcut,
    pub recorder_hotkey: Shortcut,
    pub ocr_hotkey: Shortcut,
    pub shells: HashMap<Shortcut, Shelly>,
}

impl Hotkeys {
    pub fn all_hotkeys(&self) -> Vec<Shortcut> {
        let mut a = vec![
            self.toggle.clone(),
            self.clipboard_hotkey.clone(),
            self.screenshot_hotkey.clone(),
            self.recorder_hotkey,
            self.ocr_hotkey,
        ];
        a.extend(self.shell_hotkeys());
        a
    }

    /// Only the user-defined shell-command hotkeys. On GNOME the core
    /// hotkeys are handled by the gsettings backend, so the in-process X11
    /// grab registers just these to avoid double-firing.
    pub fn shell_hotkeys(&self) -> Vec<Shortcut> {
        self.shells.keys().map(|x| x.to_owned()).collect()
    }
}

impl Tile {
    /// This returns the theme of the window
    pub fn theme(&self, _: window::Id) -> Option<Theme> {
        Some(self.theme.clone())
    }

    /// The window's clear color: fully transparent, so the rounded material
    /// drawn by the view is all that shows. The compositor reads the surface
    /// as premultiplied alpha, so the theme's background at alpha 0 would
    /// still add its color (a light theme would clear to solid white).
    pub fn style(&self, theme: &Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: iced::Color::TRANSPARENT,
            text_color: theme.palette().text,
        }
    }

    /// This handles the subscriptions of the window
    ///
    /// The subscriptions are:
    /// - Hotkeys
    /// - Hot reloading
    /// - Clipboard history
    /// - Window close events
    /// - Keypresses (escape to close the window)
    /// - Window focus changes
    pub fn subscription(&self) -> Subscription<Message> {
        let keyboard = event::listen_with(|event, _, id| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => Some(Message::EscKeyPressed(id)),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(cha),
                modifiers: Modifiers::LOGO,
                ..
            }) => {
                if cha.to_string() == "," {
                    return Some(Message::SwitchToPage(Page::Settings));
                }
                None
            }
            _ => None,
        });
        // Tick the recorder page's "● REC 00:12" row once a second.
        let recorder_clock = if self.visible && self.page == Page::Recorder {
            iced::time::every(Duration::from_secs(1)).map(|_| Message::RecorderChanged)
        } else {
            Subscription::none()
        };
        // Per-frame redraws while something animates (or the REC dot pulses).
        let now = std::time::Instant::now();
        let animating = self.visible
            && (self.motion.active(now)
                || (self.page == Page::Recorder && crate::recorder::is_recording()));
        let frames = if animating {
            window::frames().map(|_| Message::AnimationFrame)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            frames,
            recorder_clock,
            Subscription::run(handle_hot_reloading),
            keyboard,
            Subscription::run(crate::platform::urlscheme::url_stream),
            Subscription::run(handle_recipient),
            Subscription::run(reload_events),
            Subscription::run(handle_rankings),
            Subscription::run(handle_theme_mode),
            Subscription::run(handle_clipboard_history),
            Subscription::run(crate::app::screenshot::watch_subscription),
            Subscription::run(handle_file_search),
            window::close_events().map(Message::HideWindow),
            keyboard::listen().filter_map(|event| {
                if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
                    match key {
                        keyboard::Key::Named(Named::ArrowUp) => {
                            Some(Message::ChangeFocus(ArrowKey::Up, 1))
                        }
                        keyboard::Key::Named(Named::ArrowLeft) => {
                            Some(Message::ChangeFocus(ArrowKey::Left, 1))
                        }
                        keyboard::Key::Named(Named::ArrowRight) => {
                            Some(Message::ChangeFocus(ArrowKey::Right, 1))
                        }
                        keyboard::Key::Named(Named::ArrowDown) => {
                            Some(Message::ChangeFocus(ArrowKey::Down, 1))
                        }
                        keyboard::Key::Character(chr) => {
                            let s = chr.to_string();
                            if modifiers.command() && s == "r" {
                                Some(Message::ReloadConfig)
                            } else if modifiers.command() {
                                s.parse::<usize>()
                                    .ok()
                                    .filter(|&n| n >= 1 && n <= 9)
                                    .map(|n| Message::OpenResult((n - 1) as u32))
                            } else if s == "p" && modifiers.control() {
                                Some(Message::ChangeFocus(ArrowKey::Up, 1))
                            } else if s == "n" && modifiers.control() {
                                Some(Message::ChangeFocus(ArrowKey::Down, 1))
                            } else {
                                Some(Message::FocusTextInput(Move::Forwards(s)))
                            }
                        }
                        keyboard::Key::Named(Named::Enter) => Some(Message::OpenFocused),
                        keyboard::Key::Named(Named::Backspace) => {
                            Some(Message::FocusTextInput(Move::Back))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }),
            window::events()
                .with(self.focused)
                .filter_map(|(focused, (wid, event))| match event {
                    window::Event::Unfocused => {
                        if focused {
                            Some(Message::WindowFocusChanged(wid, false))
                        } else {
                            None
                        }
                    }
                    window::Event::Focused => Some(Message::WindowFocusChanged(wid, true)),
                    _ => None,
                }),
        ])
    }

    /// Handles the search query changed event.
    ///
    /// This is separate from the `update` function because it has a decent amount of logic, and
    /// should be separated out to make it easier to test. This function is called by the `update`
    /// function to handle the search query changed event.
    pub fn handle_search_query_changed(&mut self) {
        let query = self.query_lc.clone();
        let options = if self.page == Page::Main {
            &self.options
        } else if self.page == Page::EmojiSearch {
            &self.emoji_apps
        } else {
            &AppIndex::empty()
        };
        let mut results: Vec<App> = options
            .search_prefix(&query)
            .map(|x| x.to_owned())
            .collect();

        // Raycast-style: matching emoji show up in the root search too, after
        // apps and commands (files stream in after them).
        if self.page == Page::Main && query.chars().count() >= 2 {
            let mut emoji: Vec<App> = self
                .emoji_apps
                .search_prefix(&query)
                .map(|x| x.to_owned())
                .collect();
            // Names that start with the query first, then shorter names.
            emoji.sort_by_key(|e| (!e.search_name.starts_with(&query), e.search_name.len()));
            let more = emoji.len() > crate::app::apps::MAIN_SEARCH_EMOJI;
            emoji.truncate(crate::app::apps::MAIN_SEARCH_EMOJI);
            results.extend(emoji);
            if more {
                results.push(App::show_all(Page::EmojiSearch, self.query.trim()));
            }
        }

        self.results = results;
    }

    /// Clipboard entries matching the current filter and query, with their
    /// index into the full history.
    pub fn clipboard_visible(&self) -> Vec<(usize, &ClipBoardContentType)> {
        self.clipboard_content
            .iter()
            .enumerate()
            .filter(|(_, c)| self.clip_filter.matches(c, &self.query_lc))
            .collect()
    }

    pub fn frequent_results(&self) -> Vec<App> {
        self.options.top_ranked(5)
    }

    /// Captures the currently-focused window so focus can be restored later.
    pub fn capture_frontmost(&mut self) {
        self.frontmost = crate::platform::linux::x11::active_window();
    }

    /// Restores focus to the previously-focused window.
    pub fn restore_frontmost(&mut self) {
        if let Some(win) = self.frontmost {
            crate::platform::linux::x11::focus_window(win);
        }
    }

    /// The X11 window id of the previously-focused window, if any.
    pub fn frontmost_window(&self) -> Option<u32> {
        self.frontmost
    }
}

/// This is the subscription function that handles the change in clipboard history
fn handle_clipboard_history() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let mut clipboard = Clipboard::new().unwrap();
        let mut prev_byte_rep: Option<ClipBoardContentType> = None;

        loop {
            let byte_rep = if let Ok(a) = clipboard.get_image() {
                Some(ClipBoardContentType::Image(a))
            } else if let Ok(a) = clipboard.get_text()
                && !a.trim().is_empty()
            {
                Some(ClipBoardContentType::Text(a))
            } else {
                None
            };

            if byte_rep != prev_byte_rep
                && let Some(content) = &byte_rep
            {
                info!("Adding item to cbhist");
                output
                    .send(Message::EditClipboardHistory(crate::app::Editable::Create(
                        content.to_owned(),
                    )))
                    .await
                    .ok();
                prev_byte_rep = byte_rep;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

/// Run one pruned `find` pass over `dirs` and collect the results into an index.
/// Parsing (and lowercase-skipping of dotfiles) is done in parallel.
async fn build_index(dirs: &[String], home_dir: &str) -> std::io::Result<Vec<IndexEntry>> {
    let args = build_index_args(dirs, home_dir);
    let output = tokio::process::Command::new("find")
        .args(&args)
        .stderr(std::process::Stdio::null())
        .output()
        .await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text.lines().collect();
    Ok(lines
        .par_iter()
        .filter_map(|l| parse_index_line(l))
        .collect())
}

/// Spawn a background task that keeps the shared file index fresh: it builds the
/// index immediately, rebuilds whenever the search dirs change, and otherwise
/// refreshes every [`INDEX_REFRESH`] so newly-created files become searchable.
fn spawn_index_builder(
    index: Arc<RwLock<Arc<Vec<IndexEntry>>>>,
    mut dirs_rx: tokio::sync::watch::Receiver<Vec<String>>,
    home_dir: String,
) {
    tokio::spawn(async move {
        loop {
            let dirs = dirs_rx.borrow_and_update().clone();
            match build_index(&dirs, &home_dir).await {
                Ok(entries) => {
                    info!("File index ready: {} entries", entries.len());
                    *index.write().expect("index lock poisoned") = Arc::new(entries);
                }
                Err(error) => warn!("File index build failed: {error}"),
            }
            // Rebuild on a dirs change or after the refresh interval, whichever first.
            tokio::select! {
                _ = dirs_rx.changed() => {}
                _ = tokio::time::sleep(INDEX_REFRESH) => {}
            }
        }
    });
}

/// File/folder search subscription.
///
/// Instead of spawning `find` on every keystroke (which re-walked the whole tree
/// each time — multiple seconds for sparse queries), a background task keeps an
/// in-memory index of the search dirs and each query filters that snapshot in
/// parallel. Results are effectively instant and consistent regardless of how
/// rare the query is. Output batching matches what the UI expects.
fn handle_file_search() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let (sender, mut receiver) =
            tokio::sync::watch::channel((String::new(), Vec::<String>::new()));
        output
            .send(Message::SetFileSearchSender(sender))
            .await
            .expect("Failed to send file search sender.");

        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        assert!(!home_dir.is_empty(), "HOME must not be empty.");

        // Shared, atomically-swappable index, kept warm by a background builder.
        let index: Arc<RwLock<Arc<Vec<IndexEntry>>>> = Arc::new(RwLock::new(Arc::new(Vec::new())));
        let default_dirs = vec!["~".to_string()];
        let (dirs_tx, dirs_rx) = tokio::sync::watch::channel(default_dirs.clone());
        spawn_index_builder(index.clone(), dirs_rx, home_dir.clone());

        let mut last_dirs = default_dirs;

        loop {
            if receiver.changed().await.is_err() {
                break;
            }
            let (query, dirs) = receiver.borrow_and_update().clone();

            // Point the background builder at new search dirs when they change.
            if dirs != last_dirs {
                last_dirs = dirs.clone();
                dirs_tx.send(dirs).ok();
            }

            if query.chars().count() < 2 {
                output.send(Message::FileSearchClear).await.ok();
                continue;
            }

            // Filter the current snapshot off the async runtime (parallel scan).
            let snapshot = Arc::clone(&index.read().expect("index lock poisoned"));
            let (home, q) = (home_dir.clone(), query.clone());
            let apps = tokio::task::spawn_blocking(move || filter_index(&snapshot, &q, &home))
                .await
                .unwrap_or_default();

            // Stream results in UI-sized batches, bailing early if a newer query arrived.
            for chunk in apps.chunks(crate::app::FILE_SEARCH_BATCH_SIZE as usize) {
                if receiver.has_changed().unwrap_or(false) {
                    break;
                }
                if output
                    .send(Message::FileSearchResult(chunk.to_vec()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    })
}

fn handle_hot_reloading() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let paths = default_app_paths();
        let mut total_files: usize = paths
            .par_iter()
            .map(|dir| count_entries_in_dir(std::path::Path::new(dir)))
            .sum();

        loop {
            let current_total_files: usize = paths
                .par_iter()
                .map(|dir| count_entries_in_dir(std::path::Path::new(dir)))
                .sum();

            if total_files != current_total_files {
                total_files = current_total_files;
                info!("App count was changed");
                let _ = output.send(Message::UpdateApps).await;
            }

            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    })
}

/// Count the entries (e.g. `.desktop` files) inside an application directory
fn count_entries_in_dir(dir: impl AsRef<std::path::Path>) -> usize {
    // Read the directory; if it fails, treat as empty
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    entries.filter_map(|entry| entry.ok()).count()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {

    /// File-search benchmark on a real directory tree (not run by default).
    ///
    /// ```sh
    /// RUSTCAST_BENCH_DIR=~ cargo test --release file_search_benchmark -- --ignored --nocapture
    /// ```
    ///
    /// Times what RustCast actually does: one `find` pass that builds the
    /// in-memory index, then per-keystroke queries answered from that index.
    /// For comparison it also times walking the disk on every keystroke.
    #[test]
    #[ignore]
    fn file_search_benchmark() {
        use std::time::Instant;
        let dir = std::env::var("RUSTCAST_BENCH_DIR").expect("set RUSTCAST_BENCH_DIR");
        let dir = dir.replace('~', &std::env::var("HOME").unwrap_or_default());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };

        // Index build: one warm-up so every run reads from the page cache.
        let dirs = vec![dir.clone()];
        let _ = rt.block_on(build_index(&dirs, &dir)).unwrap();
        let mut builds = Vec::new();
        let mut entries = 0;
        for _ in 0..5 {
            let t = Instant::now();
            let index = rt.block_on(build_index(&dirs, &dir)).unwrap();
            builds.push(ms(t.elapsed()));
            entries = index.len();
        }
        let index = rt.block_on(build_index(&dirs, &dir)).unwrap();
        println!("entries indexed:          {entries}");
        println!("index build (median of 5): {:.0} ms", median(builds));

        // Typing "invoice" one key at a time, plus a rare and a missing query.
        let mut queries: Vec<String> = (2..="invoice".len())
            .map(|n| "invoice"[..n].to_string())
            .collect();
        queries.push("acme-invoice-0042".into());
        queries.push("zzqx-no-such-file".into());
        println!("\n{:<20} {:>9} {:>12}", "query", "results", "median ms");
        let mut all = Vec::new();
        for q in &queries {
            let mut runs = Vec::new();
            let mut n = 0;
            for _ in 0..30 {
                let t = Instant::now();
                n = filter_index(&index, q, &dir).len();
                runs.push(ms(t.elapsed()));
            }
            let m = median(runs);
            all.push(m);
            println!("{q:<20} {n:>9} {m:>12.2}");
        }
        all.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("per-keystroke worst:      {:.2} ms", all.last().unwrap());

        // Baseline: walking the disk on every keystroke (find -iname).
        let mut walks = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let out = std::process::Command::new("find")
                .args([dir.as_str(), "-iname", "*invoice*"])
                .output()
                .unwrap();
            walks.push(ms(t.elapsed()));
            assert!(out.status.success());
        }
        println!(
            "disk walk per keystroke:  {:.0} ms (find -iname, warm cache)",
            median(walks)
        );
    }
    use super::*;
    use crate::app::apps::{App, AppCommand};
    use crate::commands::Function;

    fn test_app(name: &str, ranking: i32) -> App {
        App {
            ranking,
            open_command: AppCommand::Function(Function::OpenApp(format!("{name}.desktop"))),
            desc: "Application".to_string(),
            icons: None,
            display_name: name.to_string(),
            search_name: name.to_lowercase(),
        }
    }

    #[test]
    fn app_index_search_prefix_matches_prefix_and_word_boundaries() {
        let index = AppIndex::from_apps(vec![
            test_app("Safari", 0),
            App {
                search_name: "visual studio code".to_string(),
                display_name: "Visual Studio Code".to_string(),
                ..test_app("Visual Studio Code", 0)
            },
            App {
                search_name: "signal-desktop".to_string(),
                display_name: "Signal Desktop".to_string(),
                ..test_app("Signal Desktop", 0)
            },
        ]);

        let prefix_results: Vec<_> = index
            .search_prefix("sa")
            .map(|app| app.display_name.clone())
            .collect();
        let spaced_results: Vec<_> = index
            .search_prefix("studio")
            .map(|app| app.display_name.clone())
            .collect();
        let hyphen_results: Vec<_> = index
            .search_prefix("desktop")
            .map(|app| app.display_name.clone())
            .collect();

        assert_eq!(prefix_results, vec!["Safari".to_string()]);
        assert_eq!(spaced_results, vec!["Visual Studio Code".to_string()]);
        assert_eq!(hyphen_results, vec!["Signal Desktop".to_string()]);
    }

    #[test]
    fn app_index_ranking_helpers_work() {
        let mut index = AppIndex::from_apps(vec![
            test_app("Safari", 1),
            test_app("Notes", -1),
            test_app("Arc", 3),
            test_app("Alfred", 3),
        ]);

        index.update_ranking("safari");
        index.set_ranking("notes", -1);

        assert_eq!(index.get_rankings().get("safari"), Some(&2));
        assert_eq!(index.get_rankings().get("notes"), None);

        let top_ranked = index.top_ranked(2);
        assert_eq!(top_ranked.len(), 2);
        assert_eq!(top_ranked[0].display_name, "Alfred");
        assert_eq!(top_ranked[1].display_name, "Arc");

        let favourites = index.get_favourites();
        assert_eq!(favourites.len(), 1);
        assert_eq!(favourites[0].display_name, "Notes");
    }

    #[test]
    fn build_index_args_defaults_to_home_and_prunes() {
        let args = build_index_args(&[], "/home/test");
        assert_eq!(args[0], "/home/test");
        assert!(args.contains(&"-prune".to_string()));
        assert_eq!(args.last().unwrap(), "%y\t%p\n");
        // Heavy directories are in the prune group.
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-name" && w[1] == "node_modules")
        );
    }

    #[test]
    fn build_index_args_expands_tilde_dirs() {
        let args = build_index_args(
            &[String::from("~/Documents"), String::from("/tmp")],
            "/home/test",
        );
        assert_eq!(args[0], "/home/test/Documents");
        assert_eq!(args[1], "/tmp");
    }

    #[test]
    fn parse_index_line_tags_dirs_and_skips_dotfiles() {
        assert_eq!(
            parse_index_line("d\t/home/test/Music"),
            Some(IndexEntry {
                is_dir: true,
                path: "/home/test/Music".to_string(),
            })
        );
        assert_eq!(
            parse_index_line("f\t/home/test/report.pdf"),
            Some(IndexEntry {
                is_dir: false,
                path: "/home/test/report.pdf".to_string(),
            })
        );
        assert!(parse_index_line("f\t/home/test/.env").is_none());
        assert!(parse_index_line("garbage-without-a-tab").is_none());
    }

    #[test]
    fn contains_ci_matches_case_insensitively() {
        assert!(contains_ci("ReportFinal.PDF", "final"));
        assert!(contains_ci("ReportFinal.PDF", "pdf"));
        assert!(!contains_ci("Report", "xyz"));
        assert!(contains_ci("anything", ""));
        assert!(contains_ci("Café-Menu", "café")); // non-ascii fallback path
    }

    #[test]
    fn filter_index_orders_folders_first_caps_and_excludes_non_matches() {
        let index = vec![
            IndexEntry {
                is_dir: false,
                path: "/home/test/alpha_src.txt".to_string(),
            },
            IndexEntry {
                is_dir: true,
                path: "/home/test/src_dir".to_string(),
            },
            IndexEntry {
                is_dir: false,
                path: "/home/test/zeta.txt".to_string(), // no "src" → excluded
            },
            IndexEntry {
                is_dir: true,
                path: "/home/test/another_src".to_string(),
            },
        ];

        let apps = filter_index(&index, "src", "/home/test");

        assert_eq!(apps.len(), 3);
        // Folders first (alphabetical by path), then the matching file.
        assert!(apps[0].is_folder());
        assert!(apps[1].is_folder());
        assert!(!apps[2].is_folder());
        assert_eq!(apps[0].display_name, "another_src");
        assert_eq!(apps[1].display_name, "src_dir");
        assert_eq!(apps[2].display_name, "alpha_src.txt");
    }
}

/// Handles the rx / receiver for sending and receiving messages
fn handle_recipient() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let (sender, mut recipient) = channel(100);
        output
            .send(Message::SetSender(ExtSender(sender)))
            .await
            .expect("Sender not sent");
        loop {
            let abcd = recipient
                .try_recv()
                .map(async |msg| {
                    output.send(msg).await.unwrap();
                })
                .ok();

            if let Some(abcd) = abcd {
                abcd.await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

fn reload_events() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        loop {
            output.send(Message::UpdateEvents).await.ok();
            tokio::time::sleep(Duration::from_mins(2)).await;
        }
    })
}

/// Poll the system dark mode every 2 seconds and send a message when it changes.
fn handle_theme_mode() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let mut prev_dark = crate::platform::is_dark_mode();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let current = crate::platform::is_dark_mode();
            if current != prev_dark {
                prev_dark = current;
                let _ = output.send(Message::ThemeModeChanged(current)).await;
            }
        }
    })
}

fn handle_rankings() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        loop {
            output.send(Message::SaveRanking).await.ok();
            info!("Sent save ranking");
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    })
}
