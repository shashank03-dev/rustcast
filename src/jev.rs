//! Jev — RustCast's command operator.
//!
//! Type `jev` followed by what you want in plain words and Jev turns it into
//! actions you can run with Enter:
//!
//! ```text
//! jev open downloads                 jev go to desktop/projects
//! jev open report.pdf                jev open notes in documents
//! jev find invoice in downloads      jev create folder Ideas on desktop
//! jev launch firefox                 jev switch to terminal
//! jev close spotify                  jev show desktop
//! jev record firefox                 jev record screen      jev stop recording
//! jev tile left                      jev google rust iced tutorial
//! jev open downloads and firefox     (several steps → "Run all")
//! ```
//!
//! [`parse`] turns text into [`Intent`]s (pure, no I/O); [`plan`] resolves them
//! against a [`World`] (apps, windows, folders) into rows for the launcher.

use std::path::{Path, PathBuf};

use crate::app::Message;
use crate::app::apps::{App, AppCommand, ICNS_ICON, file_result_icon};
use crate::commands::Function;
use crate::platform::linux::x11::ClientWindow;
use crate::platform::window::TilePosition;
use crate::recorder::RecordTarget;
use crate::utils::icns_data_to_handle;

/// The prefix that hands the query to Jev.
pub const PREFIX: &str = "jev";

/// Maximum directory depth searched when looking inside a folder.
const SCAN_DEPTH: usize = 4;
/// Upper bound on directory entries visited per scan, so a huge tree can't
/// stall the UI thread.
const SCAN_BUDGET: usize = 6000;
const MAX_ROWS_PER_KIND: usize = 6;

/// True when `query` is addressed to Jev (`jev` or `jev …`).
pub fn is_jev_query(query: &str) -> bool {
    let q = query.trim_start();
    q.len() >= PREFIX.len()
        && q[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
        && q[PREFIX.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == ',' || c == ':')
}

/// The part of the query after the `jev` prefix.
pub fn strip_prefix(query: &str) -> &str {
    let q = query.trim_start();
    if is_jev_query(q) {
        q[PREFIX.len()..].trim_start_matches([',', ':']).trim()
    } else {
        q.trim()
    }
}

/// Well-known places Jev understands by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownDir {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    Home,
    Root,
    Trash,
    Recordings,
    Screenshots,
    Config,
    Temp,
}

impl KnownDir {
    pub fn from_alias(word: &str) -> Option<Self> {
        Some(match word.trim().to_lowercase().as_str() {
            "desktop" => KnownDir::Desktop,
            "downloads" | "download" => KnownDir::Downloads,
            "documents" | "document" | "docs" => KnownDir::Documents,
            "pictures" | "picture" | "photos" | "images" | "pics" => KnownDir::Pictures,
            "music" | "songs" => KnownDir::Music,
            "videos" | "video" | "movies" => KnownDir::Videos,
            "home" | "~" => KnownDir::Home,
            "root" | "/" | "filesystem" => KnownDir::Root,
            "trash" | "bin" | "recycle bin" => KnownDir::Trash,
            "recordings" | "screen recordings" => KnownDir::Recordings,
            "screenshots" => KnownDir::Screenshots,
            "config" | ".config" | "configs" => KnownDir::Config,
            "tmp" | "temp" => KnownDir::Temp,
            _ => return None,
        })
    }

    /// Default locations (XDG user dirs where they exist).
    pub fn default_path(self, home: &Path) -> PathBuf {
        let or = |d: Option<PathBuf>, fallback: &str| d.unwrap_or_else(|| home.join(fallback));
        match self {
            KnownDir::Desktop => or(dirs::desktop_dir(), "Desktop"),
            KnownDir::Downloads => or(dirs::download_dir(), "Downloads"),
            KnownDir::Documents => or(dirs::document_dir(), "Documents"),
            KnownDir::Pictures => or(dirs::picture_dir(), "Pictures"),
            KnownDir::Music => or(dirs::audio_dir(), "Music"),
            KnownDir::Videos => or(dirs::video_dir(), "Videos"),
            KnownDir::Home => home.to_path_buf(),
            KnownDir::Root => PathBuf::from("/"),
            KnownDir::Trash => home.join(".local/share/Trash/files"),
            KnownDir::Recordings => or(dirs::video_dir(), "Videos").join("RustCast"),
            KnownDir::Screenshots => or(dirs::picture_dir(), "Pictures").join("Screenshots"),
            KnownDir::Config => home.join(".config"),
            KnownDir::Temp => std::env::temp_dir(),
        }
    }
}

/// One thing the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Open {
        what: String,
        within: Option<String>,
    },
    Navigate(String),
    Find {
        what: String,
        within: Option<String>,
    },
    WebSearch(String),
    Close(String),
    Focus(String),
    Create {
        folder: bool,
        name: String,
        within: Option<String>,
    },
    ShowDesktop,
    /// `None` = the whole screen, otherwise a window name.
    Record(Option<String>),
    StopRecording,
    /// Bring a window into the running locked recording.
    AddToRecording(String),
    RemoveFromRecording(String),
    Tile(TilePosition),
    Screenshot,
    Clipboard,
    Settings,
    Help,
}

/// Case-insensitive ASCII prefix strip that keeps the original text's case.
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = &s[prefix.len()..];
    // Whole words only: "opener" must not match "open".
    match rest.chars().next() {
        None => Some(""),
        Some(c) if c.is_whitespace() => Some(rest.trim_start()),
        Some(_) if !prefix.ends_with(|c: char| c.is_alphanumeric()) => Some(rest.trim_start()),
        _ => None,
    }
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    let tail = s.get(cut..)?;
    tail.eq_ignore_ascii_case(suffix)
        .then(|| s[..cut].trim_end())
}

fn first_prefix<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| strip_ci(s, p))
}

/// Drop articles/possessives and trailing kind words: "the Projects folder" → "Projects".
fn clean_object(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let before = s;
        if let Some(r) = first_prefix(s, &["the", "my", "a", "an", "app", "application"]) {
            s = r;
        }
        for suffix in [
            " please",
            " folder",
            " directory",
            " dir",
            " file",
            " app",
            " application",
            " window",
            " for me",
        ] {
            if let Some(r) = strip_suffix_ci(s, suffix) {
                s = r;
            }
        }
        if s == before {
            break;
        }
    }
    s.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace())
        .to_string()
}

/// Does `text` name a place ("downloads", "my desktop", "~/code", "desktop/x")?
fn looks_like_place(text: &str) -> bool {
    let t = clean_object(text);
    t.starts_with('/')
        || t.starts_with('~')
        || KnownDir::from_alias(t.split('/').next().unwrap_or("")).is_some()
}

/// Split "report in downloads" into ("report", Some("downloads")) when the
/// tail after the last in/on/from/inside/under is a place.
fn split_within(text: &str) -> (String, Option<String>) {
    // ASCII lowercasing keeps byte offsets identical to `text`.
    let lower = text.to_ascii_lowercase();
    let mut best: Option<(usize, usize)> = None;
    for sep in [" in ", " on ", " from ", " inside ", " under ", " at "] {
        if let Some(i) = lower.rfind(sep)
            && looks_like_place(&text[i + sep.len()..])
            && best.is_none_or(|(b, _)| i > b)
        {
            best = Some((i, sep.len()));
        }
    }
    match best {
        Some((i, len)) => (
            clean_object(&text[..i]),
            Some(clean_object(&text[i + len..])),
        ),
        None => (clean_object(text), None),
    }
}

fn parse_tile(s: &str) -> Option<TilePosition> {
    let s = clean_object(s).to_lowercase();
    let s = s
        .trim_start_matches("to ")
        .trim_start_matches("the ")
        .trim_end_matches(" side")
        .trim_end_matches(" half")
        .trim();
    Some(match s {
        "left" => TilePosition::LeftHalf,
        "right" => TilePosition::RightHalf,
        "top" | "up" => TilePosition::TopHalf,
        "bottom" | "down" => TilePosition::BottomHalf,
        "top left" | "top-left" => TilePosition::TopLeft,
        "top right" | "top-right" => TilePosition::TopRight,
        "bottom left" | "bottom-left" => TilePosition::BottomLeft,
        "bottom right" | "bottom-right" => TilePosition::BottomRight,
        "left third" => TilePosition::LeftThird,
        "center" | "centre" | "middle" | "center third" => TilePosition::CenterThird,
        "right third" => TilePosition::RightThird,
        "max" | "maximize" | "maximise" | "full" | "fullscreen" | "full screen" => {
            TilePosition::Maximize
        }
        _ => return None,
    })
}

/// Parse a single clause (no chaining).
fn parse_clause(clause: &str) -> Option<Intent> {
    let c = clause
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim_start_matches(',')
        .trim();
    let c = first_prefix(c, &["please", "can you", "could you", "hey"]).unwrap_or(c);
    let c = strip_suffix_ci(c, "please").unwrap_or(c).trim();
    let lower = c.to_lowercase();

    if lower.is_empty() || matches!(lower.as_str(), "help" | "what can you do" | "?") {
        return Some(Intent::Help);
    }

    // Fixed phrases first.
    match lower.as_str() {
        "show desktop"
        | "show the desktop"
        | "show my desktop"
        | "minimize all"
        | "minimise all"
        | "minimize everything"
        | "hide all windows"
        | "hide windows"
        | "clear desktop"
        | "clear the desktop"
        | "peek desktop" => {
            return Some(Intent::ShowDesktop);
        }
        "stop recording" | "stop the recording" | "end recording" | "finish recording"
        | "stop record" | "stop" => return Some(Intent::StopRecording),
        "screenshot" | "take screenshot" | "take a screenshot" | "capture screen"
        | "screen shot" => return Some(Intent::Screenshot),
        "clipboard" | "show clipboard" | "open clipboard" | "clipboard history"
        | "paste history" => return Some(Intent::Clipboard),
        "settings" | "preferences" | "open settings" | "open preferences" | "rustcast settings" => {
            return Some(Intent::Settings);
        }
        "maximize" | "maximise" | "maximize window" | "maximise window" => {
            return Some(Intent::Tile(TilePosition::Maximize));
        }
        _ => {}
    }

    // "add firefox to the recording", "bring terminal into recording"
    for (verbs, add) in [
        (&["add", "bring", "include", "put", "pull"][..], true),
        (&["remove", "take", "drop", "exclude"][..], false),
    ] {
        if let Some(rest) = first_prefix(c, verbs) {
            let lower_rest = rest.to_ascii_lowercase();
            for tail in [
                " to the recording",
                " to recording",
                " into the recording",
                " into recording",
                " in the recording",
                " in recording",
                " out of the recording",
                " out of recording",
                " from the recording",
                " from recording",
                " to the video",
                " into the video",
                " from the video",
            ] {
                if lower_rest.ends_with(tail) {
                    let name = clean_object(&rest[..rest.len() - tail.len()]);
                    return Some(if add {
                        Intent::AddToRecording(name)
                    } else {
                        Intent::RemoveFromRecording(name)
                    });
                }
            }
        }
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "start recording",
            "screen record",
            "record my",
            "record the",
            "record",
        ],
    ) {
        let what = clean_object(rest);
        let lw = what.to_lowercase();
        return Some(Intent::Record(
            if what.is_empty()
                || matches!(
                    lw.as_str(),
                    "screen"
                        | "desktop"
                        | "full screen"
                        | "fullscreen"
                        | "whole screen"
                        | "entire screen"
                        | "display"
                        | "monitor"
                )
            {
                None
            } else {
                Some(what)
            },
        ));
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "search the web for",
            "search web for",
            "search the web",
            "search web",
            "web search",
            "search online for",
            "google",
            "look up",
        ],
    ) {
        return Some(Intent::WebSearch(rest.trim().to_string()));
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "navigate to",
            "take me to",
            "go to",
            "goto",
            "go into",
            "navigate",
            "browse",
            "cd",
            "show me",
        ],
    ) {
        let (what, within) = split_within(rest);
        if let Some(base) = within.filter(|_| !looks_like_place(&what)) {
            // "go to projects on desktop" → look for "projects" inside desktop.
            return Some(Intent::Find {
                what,
                within: Some(base),
            });
        }
        return Some(Intent::Navigate(what));
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "search for",
            "search",
            "look for",
            "find me",
            "find",
            "locate",
            "where is",
            "where's",
        ],
    ) {
        let (what, within) = split_within(rest);
        return Some(Intent::Find { what, within });
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "switch to",
            "bring up",
            "bring",
            "focus on",
            "focus",
            "activate",
        ],
    ) {
        return Some(Intent::Focus(clean_object(rest)));
    }

    if let Some(rest) = first_prefix(c, &["close", "quit", "kill", "exit"]) {
        return Some(Intent::Close(clean_object(rest)));
    }

    if let Some(rest) = first_prefix(
        c,
        &[
            "move window to",
            "move the window to",
            "put window on",
            "tile window",
            "snap window",
            "tile",
            "snap",
        ],
    ) && let Some(pos) = parse_tile(rest)
    {
        return Some(Intent::Tile(pos));
    }

    if let Some(rest) = first_prefix(c, &["create", "make", "new", "add"]) {
        let rest = first_prefix(rest, &["a new", "a", "an", "new"]).unwrap_or(rest);
        let (folder, rest) = if let Some(r) = first_prefix(rest, &["folder", "directory", "dir"]) {
            (true, r)
        } else if let Some(r) = first_prefix(rest, &["text file", "file", "document", "note"]) {
            (false, r)
        } else {
            // "make notes.txt" → a file if it has an extension, else a folder.
            (!rest.contains('.'), rest)
        };
        let rest = first_prefix(rest, &["called", "named", "titled"]).unwrap_or(rest);
        let (name, within) = split_within(rest);
        if !name.is_empty() {
            return Some(Intent::Create {
                folder,
                name,
                within,
            });
        }
    }

    let verbs = [
        "open up", "open", "launch", "start", "run", "show", "view", "play", "edit",
    ];
    let rest = first_prefix(c, &verbs).unwrap_or(c);
    let (what, within) = split_within(rest);
    if what.is_empty() {
        return within.map(Intent::Navigate).or(Some(Intent::Help));
    }
    Some(Intent::Open { what, within })
}

/// The verb a clause starts with, used to carry it across "and": "open
/// downloads and firefox" → open downloads, open firefox.
fn leading_verb(clause: &str) -> Option<&'static str> {
    const VERBS: [&str; 13] = [
        "open",
        "launch",
        "start",
        "close",
        "quit",
        "focus",
        "record",
        "find",
        "go to",
        "show",
        "create",
        "make",
        "switch to",
    ];
    VERBS
        .into_iter()
        .find(|v| strip_ci(clause.trim(), v).is_some())
}

fn starts_with_command(clause: &str) -> bool {
    let c = clause.trim();
    leading_verb(c).is_some()
        || [
            "google",
            "search",
            "tile",
            "snap",
            "stop",
            "screenshot",
            "take",
            "navigate",
            "browse",
            "kill",
            "exit",
            "bring",
            "new",
            "maximize",
            "maximise",
            "minimize",
            "minimise",
            "hide",
            "clear",
            "settings",
            "clipboard",
            "look",
            "where",
            "cd",
            "goto",
        ]
        .iter()
        .any(|v| strip_ci(c, v).is_some())
}

/// Parse a full Jev sentence (without the `jev` prefix) into intents. Clauses
/// are chained with "and", "then", "," or ";".
pub fn parse(input: &str) -> Vec<Intent> {
    let text = input.trim();
    if text.is_empty() {
        return vec![Intent::Help];
    }

    // Split on separators, keeping each piece's original text.
    let mut pieces: Vec<String> = vec![String::new()];
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        let mut matched = None;
        for sep in [" and then ", ", then ", " then ", " and ", "; ", ", "] {
            if lower[i..].starts_with(sep) {
                matched = Some(sep.len());
                break;
            }
        }
        match matched {
            Some(n) => {
                pieces.push(String::new());
                i += n;
            }
            None => {
                let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                pieces
                    .last_mut()
                    .expect("pieces is never empty")
                    .push_str(&text[i..i + ch_len]);
                i += ch_len;
            }
        }
    }

    // Re-glue pieces that aren't commands: inherit the previous verb for short
    // objects ("open x and y"), otherwise they were part of a name.
    let mut clauses: Vec<String> = Vec::new();
    for piece in pieces.into_iter().filter(|p| !p.trim().is_empty()) {
        match clauses.last_mut() {
            None => clauses.push(piece),
            Some(_) if starts_with_command(&piece) => clauses.push(piece),
            Some(prev) => match leading_verb(prev) {
                Some(verb) if piece.split_whitespace().count() <= 3 => {
                    clauses.push(format!("{verb} {}", piece.trim()));
                }
                _ => {
                    prev.push_str(" and ");
                    prev.push_str(piece.trim());
                }
            },
        }
    }

    let intents: Vec<Intent> = clauses.iter().filter_map(|c| parse_clause(c)).collect();
    if intents.is_empty() {
        vec![Intent::Help]
    } else {
        intents
    }
}

// ----------------------------------------------------------------------------
// Resolution
// ----------------------------------------------------------------------------

/// Everything Jev needs to know about the machine to resolve intents.
pub trait World {
    fn home(&self) -> PathBuf;
    fn known_dir(&self, dir: KnownDir) -> PathBuf {
        dir.default_path(&self.home())
    }
    /// Launchable apps matching a (lowercase) query, best first.
    fn apps(&self, query: &str) -> Vec<App>;
    /// Open application windows.
    fn windows(&self) -> Vec<ClientWindow>;
    /// What "record the screen" should record.
    fn screen_target(&self) -> RecordTarget;
    /// True on Wayland, where native windows are picked through the portal.
    fn wayland(&self) -> bool;
}

/// Rows to show, plus a query to hand to the background file index (whose
/// results stream in below the rows).
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<App>,
    pub file_query: Option<String>,
}

fn row(
    title: String,
    detail: String,
    icon: Option<iced::widget::image::Handle>,
    cmd: AppCommand,
) -> App {
    App {
        ranking: 0,
        open_command: cmd,
        desc: format!("Jev · {detail}"),
        icons: icon,
        search_name: format!("jev {}", title.to_lowercase()),
        display_name: title,
    }
}

fn jev_icon() -> Option<iced::widget::image::Handle> {
    icns_data_to_handle(ICNS_ICON.to_vec())
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// How well `name` matches `needle` (both compared lowercase); lower is better.
fn match_rank(name: &str, needle: &str) -> Option<u8> {
    let name = name.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
    if name == needle || stem == needle {
        Some(0)
    } else if name.starts_with(&needle) {
        Some(1)
    } else if name.contains(&needle) {
        Some(2)
    } else {
        // Every word of the needle appears somewhere ("q3 report" ~ "report-Q3.pdf").
        let words: Vec<&str> = needle.split_whitespace().collect();
        (words.len() > 1 && words.iter().all(|w| name.contains(w))).then_some(3)
    }
}

/// Breadth-first search under `base` for entries whose name matches `needle`,
/// best matches first (folders before files on ties). Hidden entries are skipped.
pub fn scan(base: &Path, needle: &str, max_depth: usize, limit: usize) -> Vec<(PathBuf, bool)> {
    let mut hits: Vec<(u8, usize, PathBuf, bool)> = Vec::new();
    let mut queue = std::collections::VecDeque::from([(base.to_path_buf(), 0usize)]);
    let mut visited = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > SCAN_BUDGET {
                queue.clear();
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if let Some(rank) = match_rank(&name, needle) {
                hits.push((rank, depth, entry.path(), is_dir));
            }
            if is_dir && depth + 1 < max_depth {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    hits.sort_by(|a, b| {
        (a.0, a.1, !a.3, a.2.as_os_str().len()).cmp(&(b.0, b.1, !b.3, b.2.as_os_str().len()))
    });
    hits.into_iter()
        .take(limit)
        .map(|(_, _, p, d)| (p, d))
        .collect()
}

/// Resolve a place phrase to an existing directory or file:
/// "downloads", "my desktop", "~/code", "/etc", "desktop/projects/rustcast".
pub fn resolve_place(text: &str, world: &dyn World) -> Option<PathBuf> {
    let t = clean_object(text);
    if t.is_empty() {
        return None;
    }
    let home = world.home();
    if t.starts_with('/') {
        let p = PathBuf::from(&t);
        return p.exists().then_some(p);
    }
    if let Some(rest) = t.strip_prefix('~') {
        let p = home.join(rest.trim_start_matches('/'));
        return p.exists().then_some(p);
    }
    let mut segs = t.split('/').filter(|s| !s.is_empty());
    let first = segs.next()?;
    let mut path = world.known_dir(KnownDir::from_alias(first)?);
    for seg in segs {
        let exact = path.join(seg);
        path = if exact.exists() {
            exact
        } else {
            scan(&path, seg, 1, 1).into_iter().next()?.0
        };
    }
    path.exists().then_some(path)
}

fn open_path_row(path: &Path, is_dir: bool, home: &Path) -> App {
    let verb = if is_dir { "Go to" } else { "Open" };
    row(
        format!("{verb} {}", name_of(path)),
        tilde(path, home),
        file_result_icon(is_dir),
        AppCommand::Function(Function::OpenApp(path.to_string_lossy().to_string())),
    )
}

fn window_matches(windows: &[ClientWindow], name: &str) -> Vec<ClientWindow> {
    let needle = name.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, ClientWindow)> = windows
        .iter()
        // Jev never offers RustCast's own windows (never recorded, focused, …).
        .filter(|w| !crate::recorder::is_rustcast_window(w))
        .filter_map(|w| {
            let rank = match_rank(&w.class, &needle)
                .or_else(|| match_rank(&w.title, &needle).map(|r| r + 1))?;
            Some((rank, w.clone()))
        })
        .collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, w)| w).collect()
}

fn window_title(w: &ClientWindow) -> String {
    crate::recorder::window_label(w)
}

fn help_rows() -> Vec<App> {
    [
        (
            "jev open downloads",
            "Open a folder — desktop, documents, ~/code, desktop/projects…",
        ),
        (
            "jev open report.pdf in documents",
            "Find a file and open it",
        ),
        (
            "jev launch firefox · jev switch to terminal",
            "Start apps or jump to their windows",
        ),
        (
            "jev create folder Ideas on desktop",
            "Make folders and files",
        ),
        (
            "jev record firefox · jev record screen",
            "Locked-window or full-screen recording",
        ),
        (
            "jev add terminal to recording",
            "Bring another window into a locked recording",
        ),
        (
            "jev show desktop · jev tile left · jev close spotify",
            "Manage windows",
        ),
        (
            "jev open downloads and firefox",
            "Chain steps with “and” / “then”",
        ),
    ]
    .into_iter()
    .map(|(title, detail)| {
        row(
            title.to_string(),
            detail.to_string(),
            jev_icon(),
            AppCommand::Display,
        )
    })
    .collect()
}

fn message_for(cmd: &AppCommand) -> Option<Message> {
    match cmd {
        AppCommand::Function(f) => Some(Message::RunFunction(f.clone())),
        AppCommand::Message(m) => Some(m.clone()),
        AppCommand::Display => None,
    }
}

/// Resolve one intent into rows (best first). File-name queries that should
/// also go to the background index are written to `file_query`.
fn resolve(intent: &Intent, world: &dyn World, file_query: &mut Option<String>) -> Vec<App> {
    let home = world.home();
    let icon = jev_icon;
    let mut rows = Vec::new();

    match intent {
        Intent::Help => rows = help_rows(),

        Intent::Open { what, within } | Intent::Find { what, within } => {
            let is_find = matches!(intent, Intent::Find { .. });
            if let Some(within) = within {
                match resolve_place(within, world) {
                    Some(base) => {
                        let hits = scan(&base, what, SCAN_DEPTH, MAX_ROWS_PER_KIND * 2);
                        if hits.is_empty() {
                            rows.push(row(
                                format!("Nothing called “{what}” in {}", name_of(&base)),
                                format!("searched {}", tilde(&base, &home)),
                                icon(),
                                AppCommand::Display,
                            ));
                            rows.push(open_path_row(&base, true, &home));
                        }
                        rows.extend(hits.iter().map(|(p, d)| open_path_row(p, *d, &home)));
                    }
                    None => rows.push(row(
                        format!("Can't find the place “{within}”"),
                        "try downloads, desktop, documents, ~/path…".to_string(),
                        icon(),
                        AppCommand::Display,
                    )),
                }
                return rows;
            }

            if let Some(place) = resolve_place(what, world) {
                rows.push(open_path_row(&place, place.is_dir(), &home));
            }
            if !is_find {
                rows.extend(
                    world
                        .apps(&what.to_lowercase())
                        .into_iter()
                        .take(MAX_ROWS_PER_KIND)
                        .map(|app| {
                            row(
                                format!("Launch {}", app.display_name),
                                app.desc.clone(),
                                app.icons.clone(),
                                app.open_command.clone(),
                            )
                        }),
                );
                rows.extend(
                    window_matches(&world.windows(), what)
                        .into_iter()
                        .take(3)
                        .map(|w| {
                            row(
                                format!("Switch to {}", window_title(&w)),
                                "open window".to_string(),
                                icon(),
                                AppCommand::Function(Function::FocusWindow(w.id)),
                            )
                        }),
                );
            }
            // "Navigate in my desktop": things on the desktop come first.
            let desktop = world.known_dir(KnownDir::Desktop);
            rows.extend(
                scan(&desktop, what, 3, MAX_ROWS_PER_KIND)
                    .iter()
                    .map(|(p, d)| open_path_row(p, *d, &home)),
            );
            *file_query = Some(what.clone());
            if rows.is_empty() && !is_find {
                rows.push(row(
                    format!("Search the web for “{what}”"),
                    "nothing local matched yet — files may still appear below".to_string(),
                    icon(),
                    AppCommand::Function(Function::GoogleSearch(what.clone())),
                ));
            }
        }

        Intent::Navigate(place) => match resolve_place(place, world) {
            Some(path) => rows.push(open_path_row(&path, path.is_dir(), &home)),
            None => {
                // Not a known place: look for folders with that name.
                let mut hits: Vec<(PathBuf, bool)> =
                    scan(&world.known_dir(KnownDir::Desktop), place, 3, 4);
                hits.extend(scan(&home, place, 2, 4));
                hits.dedup_by(|a, b| a.0 == b.0);
                hits.sort_by_key(|(_, d)| !*d);
                rows.extend(hits.iter().map(|(p, d)| open_path_row(p, *d, &home)));
                *file_query = Some(place.clone());
                if rows.is_empty() {
                    rows.push(row(
                        format!("No folder called “{place}” yet"),
                        "matching files will appear below".to_string(),
                        icon(),
                        AppCommand::Display,
                    ));
                }
            }
        },

        Intent::WebSearch(q) => rows.push(row(
            format!("Search the web for “{q}”"),
            "web search".to_string(),
            icon(),
            AppCommand::Function(Function::GoogleSearch(q.clone())),
        )),

        Intent::Close(name) => {
            let hits = window_matches(&world.windows(), name);
            if hits.is_empty() {
                rows.push(row(
                    format!("No open window matches “{name}”"),
                    "nothing to close".to_string(),
                    icon(),
                    AppCommand::Display,
                ));
            }
            rows.extend(hits.into_iter().take(MAX_ROWS_PER_KIND).map(|w| {
                row(
                    format!("Close {}", window_title(&w)),
                    w.class
                        .split_whitespace()
                        .last()
                        .unwrap_or("window")
                        .to_string(),
                    icon(),
                    AppCommand::Function(Function::CloseWindow(w.id)),
                )
            }));
        }

        Intent::Focus(name) => {
            let hits = window_matches(&world.windows(), name);
            rows.extend(hits.into_iter().take(MAX_ROWS_PER_KIND).map(|w| {
                row(
                    format!("Switch to {}", window_title(&w)),
                    if w.minimized {
                        "minimized window"
                    } else {
                        "open window"
                    }
                    .to_string(),
                    icon(),
                    AppCommand::Function(Function::FocusWindow(w.id)),
                )
            }));
            if rows.is_empty() {
                // Not running — offer to launch it instead.
                rows.extend(
                    world
                        .apps(&name.to_lowercase())
                        .into_iter()
                        .take(3)
                        .map(|app| {
                            row(
                                format!("Launch {}", app.display_name),
                                "not running yet".to_string(),
                                app.icons.clone(),
                                app.open_command.clone(),
                            )
                        }),
                );
            }
        }

        Intent::Create {
            folder,
            name,
            within,
        } => {
            let base = within
                .as_deref()
                .and_then(|w| resolve_place(w, world))
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| {
                    let desktop = world.known_dir(KnownDir::Desktop);
                    if desktop.is_dir() {
                        desktop
                    } else {
                        home.clone()
                    }
                });
            let path = base.join(name.trim_matches('/'));
            let kind = if *folder { "folder" } else { "file" };
            rows.push(row(
                format!("Create {kind} “{name}”"),
                format!("in {}", tilde(&base, &home)),
                file_result_icon(*folder),
                AppCommand::Function(Function::CreatePath {
                    path: path.to_string_lossy().to_string(),
                    folder: *folder,
                }),
            ));
        }

        Intent::ShowDesktop => rows.push(row(
            "Show Desktop".to_string(),
            "minimize / restore all windows".to_string(),
            icon(),
            AppCommand::Function(Function::ShowDesktop),
        )),

        Intent::Record(None) => rows.push(row(
            "Record Full Screen".to_string(),
            "screen recorder".to_string(),
            icon(),
            AppCommand::Message(Message::RecorderStart(world.screen_target())),
        )),

        Intent::Record(Some(name)) => {
            rows.extend(
                window_matches(&world.windows(), name)
                    .into_iter()
                    .take(MAX_ROWS_PER_KIND)
                    .map(|w| {
                        let title = window_title(&w);
                        row(
                            format!("Record {title}"),
                            "locked window — overlapping windows won't show".to_string(),
                            icon(),
                            AppCommand::Message(Message::RecorderStart(RecordTarget::Window {
                                xid: w.id,
                                title,
                            })),
                        )
                    }),
            );
            if world.wayland() || rows.is_empty() {
                rows.push(row(
                    "Pick a window to record…".to_string(),
                    format!("no X11 window called “{name}” — use the system picker"),
                    icon(),
                    AppCommand::Message(Message::RecorderStart(RecordTarget::Portal(
                        crate::recorder::portal::PortalSource::Window,
                    ))),
                ));
            }
        }

        Intent::StopRecording => rows.push(row(
            "Stop Recording".to_string(),
            "screen recorder".to_string(),
            icon(),
            AppCommand::Message(Message::RecorderStop),
        )),

        Intent::AddToRecording(name) => {
            rows.extend(
                window_matches(&world.windows(), name)
                    .into_iter()
                    .take(MAX_ROWS_PER_KIND)
                    .map(|w| {
                        let title = window_title(&w);
                        row(
                            format!("Add {title} to Recording"),
                            "drawn into the locked recording".to_string(),
                            icon(),
                            AppCommand::Message(Message::RecorderAddWindow(w.id, title)),
                        )
                    }),
            );
            if rows.is_empty() {
                rows.push(row(
                    format!("No open window matches “{name}”"),
                    "open it first, then add it".to_string(),
                    icon(),
                    AppCommand::Display,
                ));
            }
        }

        Intent::RemoveFromRecording(name) => {
            rows.extend(
                window_matches(&world.windows(), name)
                    .into_iter()
                    .take(MAX_ROWS_PER_KIND)
                    .map(|w| {
                        row(
                            format!("Remove {} from Recording", window_title(&w)),
                            "screen recorder".to_string(),
                            icon(),
                            AppCommand::Message(Message::RecorderRemoveWindow(w.id)),
                        )
                    }),
            );
        }

        Intent::Tile(pos) => rows.push(row(
            format!("Tile window: {pos:?}"),
            "window you were using".to_string(),
            icon(),
            AppCommand::Function(Function::TileWindow(pos.clone())),
        )),

        Intent::Screenshot => rows.push(row(
            "Take a Screenshot".to_string(),
            "select a region".to_string(),
            icon(),
            AppCommand::Function(Function::Screenshot),
        )),

        Intent::Clipboard => rows.push(row(
            "Clipboard History".to_string(),
            "RustCast".to_string(),
            icon(),
            AppCommand::Message(Message::SwitchToPage(crate::app::Page::ClipboardHistory)),
        )),

        Intent::Settings => rows.push(row(
            "Open RustCast Settings".to_string(),
            "RustCast".to_string(),
            icon(),
            AppCommand::Message(Message::SwitchToPage(crate::app::Page::Settings)),
        )),
    }
    rows
}

/// True when the parser could only fall back to its catch-all ("open <the
/// whole sentence>") or to help for real input, so the model should be asked.
pub fn needs_model(input: &str) -> bool {
    let text = input.trim().to_lowercase();
    if text.split_whitespace().count() < 2 || matches!(text.as_str(), "what can you do") {
        return false;
    }
    matches!(
        parse(input).as_slice(),
        [Intent::Open { within: None, .. }] | [Intent::Help]
    )
}

/// Rows for a single intent (used for the model's pick).
pub fn rows_for(intent: &Intent, world: &dyn World) -> Vec<App> {
    let mut ignored = None;
    resolve(intent, world, &mut ignored)
}

/// Turn a Jev sentence into launcher rows.
pub fn plan(input: &str, world: &dyn World) -> Plan {
    let intents = parse(input);
    let mut file_query = None;

    if intents.len() == 1 {
        let rows = resolve(&intents[0], world, &mut file_query);
        return Plan { rows, file_query };
    }

    // Several steps: a "Run all" row (best guess for each step) on top, then
    // each step's alternatives so any single one can be picked instead.
    let mut groups = Vec::new();
    for intent in &intents {
        let mut ignored = None;
        groups.push(resolve(intent, world, &mut ignored));
    }
    let steps: Vec<(String, Message)> = groups
        .iter()
        .filter_map(|g| {
            g.iter()
                .find_map(|r| message_for(&r.open_command).map(|m| (r.display_name.clone(), m)))
        })
        .collect();

    let mut rows = Vec::new();
    if steps.len() > 1 {
        let summary = steps
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" → ");
        rows.push(row(
            format!("Run all {} steps", steps.len()),
            summary,
            jev_icon(),
            AppCommand::Message(Message::JevRunAll(
                steps.into_iter().map(|(_, m)| m).collect(),
            )),
        ));
    }
    rows.extend(groups.into_iter().flatten());
    Plan {
        rows,
        file_query: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeWorld {
        home: PathBuf,
        windows: Vec<ClientWindow>,
    }

    impl World for FakeWorld {
        fn home(&self) -> PathBuf {
            self.home.clone()
        }
        fn known_dir(&self, dir: KnownDir) -> PathBuf {
            match dir {
                KnownDir::Desktop => self.home.join("Desktop"),
                KnownDir::Downloads => self.home.join("Downloads"),
                KnownDir::Documents => self.home.join("Documents"),
                other => other.default_path(&self.home),
            }
        }
        fn apps(&self, query: &str) -> Vec<App> {
            if "firefox".starts_with(query) {
                vec![App::new(
                    "Firefox".to_string(),
                    None,
                    "Application".to_string(),
                    AppCommand::Function(Function::OpenApp("firefox.desktop".to_string())),
                )]
            } else {
                vec![]
            }
        }
        fn windows(&self) -> Vec<ClientWindow> {
            self.windows.clone()
        }
        fn screen_target(&self) -> RecordTarget {
            RecordTarget::Portal(crate::recorder::portal::PortalSource::Monitor)
        }
        fn wayland(&self) -> bool {
            false
        }
    }

    fn world() -> (tempfile::TempDir, FakeWorld) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        for d in [
            "Desktop/Projects/rustcast",
            "Desktop/Ideas",
            "Downloads/invoices",
            "Documents",
        ] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("Documents/Report-Q3.pdf"), b"x").unwrap();
        std::fs::write(home.join("Desktop/notes.txt"), b"x").unwrap();
        let windows = vec![ClientWindow {
            id: 42,
            title: "Mozilla Firefox".to_string(),
            class: "Navigator firefox".to_string(),
            pid: Some(1),
            minimized: false,
        }];
        (dir, FakeWorld { home, windows })
    }

    #[test]
    fn detects_jev_prefix() {
        assert!(is_jev_query("jev"));
        assert!(is_jev_query("Jev open downloads"));
        assert!(is_jev_query("  jev, open"));
        assert!(!is_jev_query("jevons"));
        assert!(!is_jev_query("open jev"));
        assert_eq!(strip_prefix("jev: open downloads"), "open downloads");
    }

    #[test]
    fn model_is_asked_only_when_the_parser_has_to_guess() {
        assert!(needs_model("snap this window to the left side"));
        assert!(needs_model("hide everything so i can see my wallpaper"));
        assert!(!needs_model("downloads"));
        assert!(!needs_model("help"));
        assert!(!needs_model("tile left"));
        assert!(!needs_model("record screen"));
        assert!(!needs_model("open notes in documents"));
        assert!(!needs_model("open downloads and firefox"));
    }

    #[test]
    fn parses_core_intents() {
        assert_eq!(
            parse("open downloads"),
            vec![Intent::Open {
                what: "downloads".into(),
                within: None
            }]
        );
        assert_eq!(
            parse("open the report in my documents folder"),
            vec![Intent::Open {
                what: "report".into(),
                within: Some("documents".into())
            }]
        );
        assert_eq!(
            parse("go to desktop/projects"),
            vec![Intent::Navigate("desktop/projects".into())]
        );
        assert_eq!(parse("show desktop"), vec![Intent::ShowDesktop]);
        assert_eq!(parse("record screen"), vec![Intent::Record(None)]);
        assert_eq!(
            parse("record the firefox window"),
            vec![Intent::Record(Some("firefox".into()))]
        );
        assert_eq!(parse("stop recording"), vec![Intent::StopRecording]);
        assert_eq!(
            parse("add the terminal window to the recording"),
            vec![Intent::AddToRecording("terminal".into())]
        );
        assert_eq!(
            parse("remove firefox from recording"),
            vec![Intent::RemoveFromRecording("firefox".into())]
        );
        // Plain "add" still creates things.
        assert!(matches!(
            parse("add folder Ideas").as_slice(),
            [Intent::Create { .. }]
        ));
        assert_eq!(
            parse("tile top left"),
            vec![Intent::Tile(TilePosition::TopLeft)]
        );
        assert_eq!(
            parse("google rust iced"),
            vec![Intent::WebSearch("rust iced".into())]
        );
        assert_eq!(
            parse("switch to terminal"),
            vec![Intent::Focus("terminal".into())]
        );
        assert_eq!(
            parse("close the spotify app"),
            vec![Intent::Close("spotify".into())]
        );
        assert_eq!(parse(""), vec![Intent::Help]);
    }

    #[test]
    fn parses_create_with_location() {
        assert_eq!(
            parse("create a new folder called Ideas on desktop"),
            vec![Intent::Create {
                folder: true,
                name: "Ideas".into(),
                within: Some("desktop".into())
            }]
        );
        assert_eq!(
            parse("make todo.txt"),
            vec![Intent::Create {
                folder: false,
                name: "todo.txt".into(),
                within: None
            }]
        );
    }

    #[test]
    fn chains_steps_and_inherits_verbs() {
        assert_eq!(
            parse("open downloads and firefox then show desktop"),
            vec![
                Intent::Open {
                    what: "downloads".into(),
                    within: None
                },
                Intent::Open {
                    what: "firefox".into(),
                    within: None
                },
                Intent::ShowDesktop,
            ]
        );
        // "and" inside a long name is not a separator.
        assert_eq!(
            parse("find the lord of the rings and the return of the king"),
            vec![Intent::Find {
                what: "lord of the rings and the return of the king".into(),
                within: None
            }]
        );
    }

    #[test]
    fn place_words_only_split_when_they_are_places() {
        // "in the morning" is not a place → stays part of the name.
        assert_eq!(
            parse("open notes in the morning"),
            vec![Intent::Open {
                what: "notes in the morning".into(),
                within: None
            }]
        );
    }

    #[test]
    fn resolves_places_and_nested_paths() {
        let (_tmp, w) = world();
        assert_eq!(
            resolve_place("downloads", &w),
            Some(w.home.join("Downloads"))
        );
        assert_eq!(
            resolve_place("my desktop/projects/rust", &w),
            Some(w.home.join("Desktop/Projects/rustcast"))
        );
        assert_eq!(
            resolve_place("~/Documents", &w),
            Some(w.home.join("Documents"))
        );
        assert_eq!(resolve_place("nowhere", &w), None);
    }

    #[test]
    fn open_file_inside_a_place() {
        let (_tmp, w) = world();
        let plan = plan("open q3 report in documents", &w);
        let first = &plan.rows[0];
        assert_eq!(first.display_name, "Open Report-Q3.pdf");
        assert!(matches!(
            &first.open_command,
            AppCommand::Function(Function::OpenApp(p)) if p.ends_with("Documents/Report-Q3.pdf")
        ));
    }

    #[test]
    fn open_prefers_places_then_apps_and_searches_files() {
        let (_tmp, w) = world();
        let p = plan("open downloads", &w);
        assert_eq!(p.rows[0].display_name, "Go to Downloads");
        assert_eq!(p.file_query.as_deref(), Some("downloads"));

        let p = plan("open firefox", &w);
        assert_eq!(p.rows[0].display_name, "Launch Firefox");
        assert!(
            p.rows
                .iter()
                .any(|r| r.display_name == "Switch to Mozilla Firefox")
        );

        // Things on the desktop are found without a location.
        let p = plan("open ideas", &w);
        assert!(p.rows.iter().any(|r| r.display_name == "Go to Ideas"));
    }

    #[test]
    fn record_window_targets_the_matching_window() {
        let (_tmp, w) = world();
        let p = plan("record firefox", &w);
        assert!(matches!(
            &p.rows[0].open_command,
            AppCommand::Message(Message::RecorderStart(RecordTarget::Window { xid: 42, .. }))
        ));
    }

    #[test]
    fn create_defaults_to_desktop() {
        let (_tmp, w) = world();
        let p = plan("create folder Plans", &w);
        assert!(matches!(
            &p.rows[0].open_command,
            AppCommand::Function(Function::CreatePath { path, folder: true })
                if path.ends_with("Desktop/Plans")
        ));
    }

    #[test]
    fn chained_plan_has_run_all_row() {
        let (_tmp, w) = world();
        let p = plan("open downloads and firefox", &w);
        assert_eq!(p.rows[0].display_name, "Run all 2 steps");
        match &p.rows[0].open_command {
            AppCommand::Message(Message::JevRunAll(steps)) => assert_eq!(steps.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }
}
