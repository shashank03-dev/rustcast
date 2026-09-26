//! On-disk persistence for clipboard history and screenshots.
//!
//! History lives under `~/.local/share/rustcast/clipboard/`:
//! - `index.json` — ordered list of entries (newest first)
//! - `<id>.png`   — image payloads
//!
//! Text entries are stored inline in the index; images reference a PNG file.
//! The store is capped at [`MAX_ITEMS`] entries.

use std::borrow::Cow;
use std::path::PathBuf;

use arboard::ImageData;
use serde::{Deserialize, Serialize};

use crate::clipboard::ClipBoardContentType;

/// Maximum number of history entries kept on disk / in memory.
pub const MAX_ITEMS: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
enum Entry {
    #[serde(rename = "text")]
    Text { content: String },
    #[serde(rename = "image")]
    Image {
        file: String,
        width: usize,
        height: usize,
    },
}

fn store_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("rustcast/clipboard")
}

fn index_path() -> PathBuf {
    store_dir().join("index.json")
}

fn ensure_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(store_dir())
}

/// Load the persisted clipboard history (newest first).
pub fn load_history() -> Vec<ClipBoardContentType> {
    let Ok(raw) = std::fs::read_to_string(index_path()) else {
        return Vec::new();
    };
    let entries: Vec<Entry> = serde_json::from_str(&raw).unwrap_or_default();

    entries
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Text { content } => Some(ClipBoardContentType::Text(content)),
            Entry::Image {
                file,
                width,
                height,
            } => {
                let path = store_dir().join(&file);
                let img = image::ImageReader::open(&path).ok()?.decode().ok()?;
                let rgba = img.to_rgba8();
                Some(ClipBoardContentType::Image(ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(rgba.into_raw()),
                }))
            }
        })
        .collect()
}

/// Persist the full clipboard history (newest first), capping at [`MAX_ITEMS`]
/// and writing image payloads as PNG files. Stale PNGs are pruned.
pub fn save_history(history: &[ClipBoardContentType]) {
    if ensure_dir().is_err() {
        return;
    }

    let mut kept_files = std::collections::HashSet::new();
    let mut entries = Vec::new();

    for (i, item) in history.iter().take(MAX_ITEMS).enumerate() {
        match item {
            ClipBoardContentType::Text(content) => {
                entries.push(Entry::Text {
                    content: content.clone(),
                });
            }
            ClipBoardContentType::Image(data) => {
                let file = format!("img-{i}-{}.png", stable_hash(&data.bytes));
                let path = store_dir().join(&file);
                if !path.exists()
                    && let Some(buf) = image::RgbaImage::from_raw(
                        data.width as u32,
                        data.height as u32,
                        data.bytes.to_vec(),
                    )
                {
                    let _ = buf.save(&path);
                }
                kept_files.insert(file.clone());
                entries.push(Entry::Image {
                    file,
                    width: data.width,
                    height: data.height,
                });
            }
        }
    }

    if let Ok(json) = serde_json::to_string(&entries) {
        let _ = std::fs::write(index_path(), json);
    }

    // Prune image files no longer referenced.
    if let Ok(read) = std::fs::read_dir(store_dir()) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".png") && !kept_files.contains(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    // Sample to keep hashing cheap for large images.
    for chunk in bytes.chunks(4096).take(64) {
        chunk[0].hash(&mut h);
    }
    h.finish()
}

/// Directory where captured screenshots are stored.
pub fn screenshots_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rustcast/screenshots")
}
