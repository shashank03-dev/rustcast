//! Linux application discovery via freedesktop `.desktop` entries.
//! Mirrors the macOS `discovery`/`cross` modules' public surface
//! (`default_app_paths`, `get_installed_apps`).

use std::fs;
use std::path::{Path, PathBuf};

use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::app::apps::{App, AppCommand};
use crate::commands::Function;
use crate::utils::handle_from_image_path;

/// Directories that hold `.desktop` application entries, in XDG precedence.
pub fn default_app_paths() -> Vec<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(data_home) = dirs::data_dir() {
        dirs.push(data_home.join("applications"));
    }
    let xdg_data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in xdg_data_dirs.split(':') {
        if !d.is_empty() {
            dirs.push(Path::new(d).join("applications"));
        }
    }
    // Common flatpak/snap locations.
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));

    // De-duplicate while preserving order.
    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

pub fn get_installed_apps(store_icons: bool) -> Vec<App> {
    default_app_paths()
        .into_par_iter()
        .flat_map(|dir| discover_dir(dir, store_icons))
        .collect()
}

fn discover_dir(dir: String, store_icons: bool) -> Vec<App> {
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };

    entries
        .into_par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                return None;
            }
            parse_desktop_entry(&path, store_icons)
        })
        .collect()
}

struct DesktopEntry {
    name: String,
    icon: Option<String>,
    no_display: bool,
    is_application: bool,
}

fn parse_desktop_entry(path: &Path, store_icons: bool) -> Option<App> {
    let contents = fs::read_to_string(path).ok()?;
    let entry = parse_desktop_contents(&contents);

    if entry.no_display || !entry.is_application || entry.name.is_empty() {
        return None;
    }

    let icons = if store_icons {
        entry
            .icon
            .as_deref()
            .and_then(resolve_icon)
            .and_then(|p| handle_from_image_path(&p))
    } else {
        None
    };

    let path_str = path.to_string_lossy().to_string();
    Some(App {
        ranking: 0,
        open_command: AppCommand::Function(Function::OpenApp(path_str)),
        desc: "Application".to_string(),
        icons,
        search_name: entry.name.to_lowercase(),
        display_name: entry.name,
    })
}

/// Parse the `[Desktop Entry]` group of a `.desktop` file.
fn parse_desktop_contents(contents: &str) -> DesktopEntry {
    let mut name = String::new();
    let mut icon = None;
    let mut no_display = false;
    let mut hidden = false;
    let mut type_is_app = false;
    let mut in_group = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Name" if name.is_empty() => name = value.to_string(),
            "Icon" => icon = Some(value.to_string()),
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "Type" => type_is_app = value == "Application",
            _ => {}
        }
    }

    DesktopEntry {
        name,
        icon,
        no_display: no_display || hidden,
        is_application: type_is_app,
    }
}

/// Resolve an `Icon=` value (absolute path or themed name) to a loadable PNG.
fn resolve_icon(icon: &str) -> Option<PathBuf> {
    let p = Path::new(icon);
    if p.is_absolute() {
        return if p.exists() {
            Some(p.to_path_buf())
        } else {
            None
        };
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share/icons"));
        roots.push(home.join(".icons"));
    }
    roots.push(PathBuf::from("/usr/share/icons"));
    roots.push(PathBuf::from("/usr/local/share/icons"));

    // Prefer larger sizes for crisp thumbnails.
    let sizes = [
        "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "scalable",
    ];
    let themes = ["hicolor", "Adwaita", "gnome", "Yaru"];

    for root in &roots {
        for theme in &themes {
            for size in &sizes {
                let candidate = root
                    .join(theme)
                    .join(size)
                    .join("apps")
                    .join(format!("{icon}.png"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    // pixmaps fallback
    for ext in ["png", "xpm"] {
        let candidate = PathBuf::from(format!("/usr/share/pixmaps/{icon}.{ext}"));
        if candidate.exists() && ext == "png" {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_application_entry() {
        let contents =
            "[Desktop Entry]\nType=Application\nName=Firefox\nIcon=firefox\nExec=firefox %u\n";
        let e = parse_desktop_contents(contents);
        assert_eq!(e.name, "Firefox");
        assert_eq!(e.icon.as_deref(), Some("firefox"));
        assert!(e.is_application);
        assert!(!e.no_display);
    }

    #[test]
    fn respects_nodisplay_and_hidden() {
        let hidden = "[Desktop Entry]\nType=Application\nName=Daemon\nHidden=true\n";
        assert!(parse_desktop_contents(hidden).no_display);
        let nodisplay = "[Desktop Entry]\nType=Application\nName=Daemon\nNoDisplay=true\n";
        assert!(parse_desktop_contents(nodisplay).no_display);
    }

    #[test]
    fn ignores_non_application_type() {
        let link = "[Desktop Entry]\nType=Link\nName=Site\nURL=https://x\n";
        assert!(!parse_desktop_contents(link).is_application);
    }

    #[test]
    fn only_reads_desktop_entry_group() {
        let contents =
            "[Desktop Entry]\nType=Application\nName=Real\n[Desktop Action New]\nName=Other\n";
        assert_eq!(parse_desktop_contents(contents).name, "Real");
    }
}
