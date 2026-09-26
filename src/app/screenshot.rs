//! Screenshot capture + a file watcher that surfaces new screenshots as a
//! draggable bottom-left thumbnail (via the X11 overlay) and adds them to the
//! clipboard history.
//!
//! Capture is triggered by the configurable screenshot hotkey and shells out to
//! `gnome-screenshot -a` (interactive region select). The watcher additionally
//! catches screenshots taken by other tools (PrintScreen → `~/Pictures/Screenshots`).

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use arboard::ImageData;
use iced::futures::{self, SinkExt};
use iced::stream;

use crate::app::{Editable, Message};
use crate::clipboard::ClipBoardContentType;
use crate::persist::screenshots_dir;

/// Directories watched for new screenshots.
fn watch_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![screenshots_dir()];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Pictures/Screenshots"));
        dirs.push(home.join("Pictures"));
    }
    if let Some(pics) = dirs::picture_dir() {
        dirs.push(pics.join("Screenshots"));
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Trigger an interactive region capture. The resulting PNG lands in the
/// RustCast screenshots directory, where the watcher picks it up.
pub fn trigger_capture() {
    std::thread::spawn(|| {
        let dir = screenshots_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("rustcast-{ts}.png"));

        let status = std::process::Command::new("gnome-screenshot")
            .arg("-a")
            .arg("-f")
            .arg(&path)
            .status();

        match status {
            Ok(s) if s.success() && path.exists() => {
                // Watcher will display + record it.
            }
            _ => {
                log::warn!("gnome-screenshot capture failed or was cancelled");
                let _ = std::fs::remove_file(&path);
            }
        }
    });
}

fn is_image_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg")
    )
}

fn load_image_data(path: &std::path::Path) -> Option<ImageData<'static>> {
    let img = image::ImageReader::open(path).ok()?.decode().ok()?;
    let rgba = img.to_rgba8();
    Some(ImageData {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        bytes: Cow::Owned(rgba.into_raw()),
    })
}

/// Subscription that watches screenshot directories and, for each newly created
/// image, shows the draggable thumbnail and records it in clipboard history.
pub fn watch_subscription() -> impl futures::Stream<Item = Message> {
    stream::channel(100, async |mut output| {
        let dirs = watch_dirs();
        let start = SystemTime::now();

        // Seed with files already present so existing screenshots are ignored.
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for dir in &dirs {
            if let Ok(read) = std::fs::read_dir(dir) {
                for entry in read.flatten() {
                    seen.insert(entry.path());
                }
            }
        }

        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;

            for dir in &dirs {
                let Ok(read) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in read.flatten() {
                    let path = entry.path();
                    if !is_image_file(&path) || seen.contains(&path) {
                        continue;
                    }
                    seen.insert(path.clone());

                    // Only react to files created after startup.
                    let fresh = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .map(|m| m >= start)
                        .unwrap_or(false);
                    if !fresh {
                        continue;
                    }

                    log::info!("New screenshot detected: {}", path.display());
                    crate::platform::linux::overlay::show_thumbnail(path.clone());

                    if let Some(data) = load_image_data(&path) {
                        output
                            .send(Message::EditClipboardHistory(Editable::Create(
                                ClipBoardContentType::Image(data),
                            )))
                            .await
                            .ok();
                    }
                }
            }
        }
    })
}
