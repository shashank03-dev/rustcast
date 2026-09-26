//! Font family lookup: the macOS system font (SF Pro) when it is installed,
//! and interning of configured family names.
//!
//! iced wants `&'static str` family names. Names are interned once instead of
//! leaked on every `Theme::font()` call, which runs for each text widget on
//! every frame.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use iced::font::Family;
use once_cell::sync::Lazy;

/// Families installed on this system, lowercased. Read once from fontconfig.
static INSTALLED: Lazy<HashSet<String>> = Lazy::new(installed_families);

static INTERNED: Lazy<Mutex<HashMap<String, &'static str>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// macOS' UI fonts first, then the closest open lookalikes.
const SYSTEM_PREFERENCE: &[&str] = &[
    "SF Pro Text",
    "SF Pro",
    ".AppleSystemUIFont",
    "Inter",
    "Inter Variable",
];

/// A `&'static str` for `name`, allocated at most once per distinct name.
pub fn intern(name: &str) -> &'static str {
    let mut map = INTERNED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(name) {
        return s;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    map.insert(name.to_string(), leaked);
    leaked
}

pub fn is_installed(family: &str) -> bool {
    INSTALLED.contains(&family.to_lowercase())
}

/// The default UI family: SF Pro when available, else Inter, else the
/// desktop's sans-serif.
pub fn system_family() -> Family {
    static CHOSEN: Lazy<Family> = Lazy::new(|| {
        SYSTEM_PREFERENCE
            .iter()
            .find(|name| is_installed(name))
            .map(|name| Family::Name(name))
            .unwrap_or(Family::SansSerif)
    });
    *CHOSEN
}

/// The Display optical size for a Text family (SF Pro Text → SF Pro Display),
/// only if that family is installed. Apple uses Display from 20pt up.
pub fn display_variant(family: Family) -> Family {
    match family {
        Family::Name(name) if name.starts_with("SF Pro") && is_installed("SF Pro Display") => {
            Family::Name("SF Pro Display")
        }
        other => other,
    }
}

fn installed_families() -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(out) = std::process::Command::new("fc-list")
        .args([":", "family"])
        .output()
    else {
        return set;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // One font per line; localized names are comma-separated.
        for name in line.split(',') {
            let name = name.trim();
            if !name.is_empty() {
                set.insert(name.to_lowercase());
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_reuses_allocation() {
        let a = intern("Some Font");
        let b = intern("Some Font");
        assert!(std::ptr::eq(a, b));
        assert_eq!(a, "Some Font");
    }

    #[test]
    fn display_variant_leaves_other_families_alone() {
        assert_eq!(display_variant(Family::SansSerif), Family::SansSerif);
        assert_eq!(
            display_variant(Family::Name("Inter")),
            Family::Name("Inter")
        );
    }
}
