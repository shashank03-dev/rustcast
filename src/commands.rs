//! This handles all the different commands that rustcast can perform, such as opening apps,
//! copying to clipboard, etc.
use std::{process::Command, thread};

use arboard::Clipboard;

use crate::{
    app::apps::{App, AppCommand},
    calculator::Expr,
    clipboard::ClipBoardContentType,
    config::Config,
    quit::{terminate_all_apps, terminate_app},
};

/// Launch a freedesktop `.desktop` entry, or open a plain path/URL with the
/// desktop's default handler.
fn open_target(target: &str) {
    let target = target.to_owned();
    thread::spawn(move || {
        if target.ends_with(".desktop") {
            // Launch a desktop entry; fall back to xdg-open if gio is missing.
            if Command::new("gio")
                .arg("launch")
                .arg(&target)
                .spawn()
                .is_err()
            {
                Command::new("xdg-open").arg(&target).spawn().ok();
            }
        } else {
            Command::new("xdg-open").arg(&target).spawn().ok();
        }
    });
}

fn open_url_string(url: &str) {
    let url = url.to_owned();
    thread::spawn(move || {
        Command::new("xdg-open").arg(&url).spawn().ok();
    });
}

/// The different functions that rustcast can perform
#[derive(Debug, Clone, PartialEq)]
pub enum Function {
    OpenApp(String),
    QuitApp(String),
    OpenRawUrl(String),
    QuitAllApps,
    RunShellCommand(String),
    OpenWebsite(String),
    RandomVar(i32), // Easter egg function
    CopyToClipboard(ClipBoardContentType),
    GoogleSearch(String),
    Calculate(Expr),
    Quit,
    TileWindow(crate::platform::window::TilePosition),
}

impl Function {
    /// Run the command
    pub fn execute(&self, config: &Config) {
        match self {
            Function::OpenApp(path) => open_target(path),

            Function::OpenRawUrl(url) => open_url_string(url),

            Function::RunShellCommand(command) => {
                Command::new("sh").arg("-c").arg(command).spawn().ok();
            }
            Function::RandomVar(var) => {
                Clipboard::new()
                    .unwrap()
                    .set_text(var.to_string())
                    .unwrap_or(());
            }

            Function::QuitAllApps => {
                terminate_all_apps();
            }

            Function::QuitApp(name) => {
                terminate_app(name.to_owned());
            }

            Function::GoogleSearch(query_string) => {
                let query_args = query_string.replace(" ", "+");
                let query = config.search_url.replace("%s", &query_args);
                let query = query.strip_suffix("?").unwrap_or(&query).to_string();
                open_url_string(&query);
            }

            Function::OpenWebsite(url) => {
                let open = if url.starts_with("http") {
                    url.to_owned()
                } else {
                    format!("https://{}", url)
                };
                open_url_string(&open);
            }

            Function::Calculate(expr) => {
                Clipboard::new()
                    .unwrap()
                    .set_text(expr.eval().map(|x| x.to_string()).unwrap_or("".to_string()))
                    .unwrap_or(());
            }

            Function::CopyToClipboard(clipboard_content) => match clipboard_content {
                ClipBoardContentType::Text(text) => {
                    Clipboard::new().unwrap().set_text(text).ok();
                }
                ClipBoardContentType::Image(img) => {
                    Clipboard::new().unwrap().set_image(img.to_owned_img()).ok();
                    // Also surface it as a draggable thumbnail (drop into any
                    // app), the same overlay used for screenshots.
                    if let Some(path) = clipboard_image_to_temp_png(img) {
                        crate::platform::linux::overlay::show_thumbnail(path);
                    }
                }
            },

            Function::Quit => std::process::exit(0),

            // TileWindow is intercepted in the RunFunction handler which has
            // access to the focused window id; nothing to do here.
            Function::TileWindow(_) => {}
        }
    }
}

/// Encode a clipboard image (raw RGBA) to a PNG in the screenshots directory so
/// it can be handed to the draggable overlay. Returns the written path.
fn clipboard_image_to_temp_png(img: &arboard::ImageData) -> Option<std::path::PathBuf> {
    let (w, h) = (img.width as u32, img.height as u32);
    let buf = image::RgbaImage::from_raw(w, h, img.bytes.clone().into_owned())?;
    let dir = crate::persist::screenshots_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("clipboard-{ts}.png"));
    buf.save(&path).ok()?;
    Some(path)
}

/// Convert an absolute file path into an App for display in file search results.
/// `is_dir` selects the folder vs document icon and lets results be sorted with
/// folders first (see [`crate::app::apps::file_result_icon`]).
///
/// Returns None for dotfiles or paths that cannot be parsed.
pub fn path_to_app(absolute_path: &str, home_dir: &str, is_dir: bool) -> Option<App> {
    assert!(!home_dir.is_empty(), "Home directory must not be empty.");
    let path = absolute_path.trim();
    if path.is_empty() {
        return None;
    }

    let filename = std::path::Path::new(path).file_name()?.to_str()?;
    if filename.starts_with('.') {
        return None;
    }

    let display_path = if let Some(suffix) = path.strip_prefix(home_dir) {
        format!("~{suffix}")
    } else {
        path.to_string()
    };

    Some(App {
        ranking: 0,
        open_command: AppCommand::Function(Function::OpenApp(path.to_string())),
        desc: display_path,
        icons: crate::app::apps::file_result_icon(is_dir),
        display_name: filename.to_string(),
        search_name: filename.to_lowercase(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_app_rewrites_home_prefix_and_uses_filename() {
        let app = path_to_app("/home/test/Documents/report.pdf", "/home/test", false).unwrap();

        assert_eq!(app.display_name, "report.pdf");
        assert_eq!(app.search_name, "report.pdf");
        assert_eq!(app.desc, "~/Documents/report.pdf");
        assert!(!app.is_folder());
        assert!(matches!(
            app.open_command,
            AppCommand::Function(Function::OpenApp(path))
                if path == "/home/test/Documents/report.pdf"
        ));
    }

    #[test]
    fn path_to_app_marks_directories_as_folders() {
        let app = path_to_app("/home/test/Documents", "/home/test", true).unwrap();
        assert!(app.is_folder());
    }

    #[test]
    fn path_to_app_rejects_empty_and_dotfile_paths() {
        assert!(path_to_app("", "/home/test", false).is_none());
        assert!(path_to_app("/home/test/.env", "/home/test", false).is_none());
        assert!(path_to_app("   ", "/home/test", false).is_none());
    }
}
