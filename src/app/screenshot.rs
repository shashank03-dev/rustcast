//! Screenshot capture + a file watcher that surfaces new screenshots as a
//! draggable bottom-left thumbnail (via the X11 overlay) and adds them to the
//! clipboard history.
//!
//! Capture is triggered by the configurable screenshot hotkey and opens RustCast's
//! own capture overlay (see [`crate::snap`]). The watcher additionally catches
//! screenshots taken by other tools (PrintScreen → `~/Pictures/Screenshots`).

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

/// Capture modes accepted by `rustcast://capture/<mode>` and the launcher.
pub const CAPTURE_MODES: [&str; 5] = ["area", "window", "fullscreen", "quick", "ocr"];

/// Directories watched for new screenshots.
fn watch_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        screenshots_dir(),
        crate::snap::load_config().screenshot.save_dir(),
    ];
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

/// Open the capture overlay (region select + annotate).
pub fn trigger_capture() {
    crate::snap::spawn(&["area"]);
}

fn is_image_file(path: &std::path::Path) -> bool {
    // Dot-files are in-progress writes or RustCast's own temporary grabs.
    let hidden = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'));
    !hidden
        && matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .as_deref(),
            Some("png") | Some("jpg") | Some("jpeg") | Some("webp")
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
                    let modified = entry.metadata().and_then(|m| m.modified()).ok();
                    // Give other tools a moment to finish writing the file.
                    let settled = modified
                        .and_then(|m| m.elapsed().ok())
                        .is_some_and(|age| age >= Duration::from_millis(300));
                    if !settled {
                        continue;
                    }
                    seen.insert(path.clone());

                    // Only react to files created after startup.
                    let fresh = modified.is_some_and(|m| m >= start);
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
