//! GNOME global-hotkey backend via `gsettings` custom keybindings.
//!
//! On GNOME/Wayland the in-process X11 `XGrabKey` grabs used by the
//! `global-hotkey` crate only fire while an X11 window is focused, so they
//! silently break whenever a Wayland-native app has focus. GNOME's own
//! custom-keybindings, by contrast, fire regardless of the focused client.
//!
//! Each registered keybinding runs `rustcast rustcast://<host>`, which the
//! single-instance URL socket forwards to the running instance (see
//! [`crate::platform::linux::urlscheme`]); the instance then dispatches it
//! through the normal hotkey path.

use std::path::Path;
use std::process::Command;

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const BASE_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

/// The three keybindings we manage: (id, human name, `rustcast://` host).
const ENTRIES: [(&str, &str, &str); 3] = [
    ("rustcast-toggle", "RustCast Toggle", "toggle"),
    (
        "rustcast-clipboard",
        "RustCast Clipboard History",
        "clipboard",
    ),
    ("rustcast-screenshot", "RustCast Screenshot", "screenshot"),
];

/// True when running under a GNOME session with `gsettings` available, i.e.
/// when this backend should be used in place of the in-process X11 grabs.
pub fn is_gnome() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is_gnome = desktop
        .split(':')
        .any(|d| d.eq_ignore_ascii_case("GNOME") || d.eq_ignore_ascii_case("ubuntu"));
    is_gnome && which_gsettings()
}

fn which_gsettings() -> bool {
    Command::new("gsettings")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gsettings(args: &[&str]) -> Option<String> {
    // Strip AppImage-bundled module/schema overrides so gsettings loads the
    // system dconf backend and writes reach the real GNOME settings (see
    // `sanitize_inherited_env`). Belt-and-suspenders alongside the startup
    // sanitizer in case this process was launched some other polluted way.
    let mut cmd = Command::new("gsettings");
    cmd.args(args);
    for var in [
        "LD_LIBRARY_PATH",
        "GIO_MODULE_DIR",
        "GIO_EXTRA_MODULES",
        "GSETTINGS_SCHEMA_DIR",
        "GSETTINGS_BACKEND",
        "GCONV_PATH",
    ] {
        cmd.env_remove(var);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        log::warn!(
            "gsettings {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn set(schema_path: &str, key: &str, value: &str) {
    gsettings(&["set", schema_path, key, value]);
}

/// Register (or refresh) the GNOME keybindings for the given accelerators.
/// The accelerators are RustCast hotkey strings (e.g. `"ALT+SPACE"`); empty or
/// unparseable ones are skipped. `exe` is the absolute path invoked by the
/// keybinding. Safe to call on every launch — it is idempotent.
pub fn register(exe: &Path, toggle: &str, clipboard: &str, screenshot: &str) {
    let accels = [toggle, clipboard, screenshot];
    let mut managed_paths = Vec::new();

    for ((id, name, host), accel) in ENTRIES.iter().zip(accels) {
        let Some(accel) = to_accelerator(accel) else {
            log::warn!("Skipping GNOME keybinding {id}: cannot parse '{accel}'");
            continue;
        };
        let path = format!("{BASE_PATH}/{id}/");
        let schema_path = format!("{CUSTOM_SCHEMA}:{path}");
        set(&schema_path, "name", name);
        set(
            &schema_path,
            "command",
            &format!("{} rustcast://{host}", exe.display()),
        );
        set(&schema_path, "binding", &accel);
        clear_wm_conflict(&accel);
        managed_paths.push(path);
    }

    merge_custom_list(&managed_paths, true);
    log::info!(
        "Registered {} GNOME custom keybindings",
        managed_paths.len()
    );
}

/// Remove all RustCast-managed GNOME keybindings (used on uninstall).
pub fn unregister() {
    let paths: Vec<String> = ENTRIES
        .iter()
        .map(|(id, _, _)| format!("{BASE_PATH}/{id}/"))
        .collect();
    merge_custom_list(&paths, false);
}

/// Read the current `custom-keybindings` object-path list, then add (or remove)
/// our managed paths while preserving the user's other custom keybindings.
fn merge_custom_list(managed: &[String], add: bool) {
    let current = gsettings(&["get", MEDIA_KEYS, "custom-keybindings"]).unwrap_or_default();
    let mut paths: Vec<String> = parse_path_list(&current);

    paths.retain(|p| !managed.contains(p));
    if add {
        paths.extend(managed.iter().cloned());
    }

    let value = if paths.is_empty() {
        "[]".to_string()
    } else {
        let inner = paths
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{inner}]")
    };
    set(MEDIA_KEYS, "custom-keybindings", &value);
}

/// Parse a gsettings `as` value like `['/a/', '/b/']` (or `@as []`) into paths.
fn parse_path_list(raw: &str) -> Vec<String> {
    // Quoted segments land at odd indices when splitting on the quote char.
    raw.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// If `accel` is currently bound to GNOME's window menu (the classic Alt+Space
/// clash), drop it so our keybinding actually fires.
fn clear_wm_conflict(accel: &str) {
    const WM: &str = "org.gnome.desktop.wm.keybindings";
    for key in ["activate-window-menu"] {
        if let Some(cur) = gsettings(&["get", WM, key])
            && cur.contains(accel)
        {
            set(WM, key, "[]");
            log::info!("Cleared conflicting GNOME wm keybinding {key} ({accel})");
        }
    }
}

/// Convert a RustCast hotkey string (`"SUPER+SHIFT+C"`) into a GNOME / GTK
/// accelerator (`"<Super><Shift>c"`). Returns `None` if no key is present.
fn to_accelerator(s: &str) -> Option<String> {
    let mut mods = String::new();
    let mut key: Option<String> = None;

    for part in s.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods.push_str("<Control>"),
            "alt" | "opt" | "option" => mods.push_str("<Alt>"),
            "shift" => mods.push_str("<Shift>"),
            "cmd" | "command" | "super" | "meta" | "logo" => mods.push_str("<Super>"),
            "fn" | "function" | "capslock" | "caps" => {}
            k => key = Some(keysym_name(k)),
        }
    }

    key.map(|k| format!("{mods}{k}"))
}

/// Map a key token to its GDK keysym name (as accepted by gtk_accelerator_parse).
fn keysym_name(k: &str) -> String {
    match k {
        "space" => "space".to_string(),
        "enter" | "return" => "Return".to_string(),
        "tab" => "Tab".to_string(),
        "backspace" | "delete" => "BackSpace".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "up" | "arrowup" => "Up".to_string(),
        "down" | "arrowdown" => "Down".to_string(),
        "left" | "arrowleft" => "Left".to_string(),
        "right" | "arrowright" => "Right".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" => "Page_Up".to_string(),
        "pagedown" => "Page_Down".to_string(),
        "minus" | "-" => "minus".to_string(),
        "equal" | "=" => "equal".to_string(),
        "comma" | "," => "comma".to_string(),
        "period" | "." => "period".to_string(),
        "slash" | "/" => "slash".to_string(),
        "backslash" | "\\" => "backslash".to_string(),
        "semicolon" | ";" => "semicolon".to_string(),
        "grave" | "backquote" | "`" => "grave".to_string(),
        // f1..f12 → F1..F12; single letters and digits pass through verbatim.
        other if other.len() <= 3 && other.starts_with('f') && other[1..].parse::<u8>().is_ok() => {
            format!("F{}", &other[1..])
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerator_conversion() {
        assert_eq!(to_accelerator("ALT+SPACE").as_deref(), Some("<Alt>space"));
        assert_eq!(
            to_accelerator("SUPER+SHIFT+C").as_deref(),
            Some("<Super><Shift>c")
        );
        assert_eq!(to_accelerator("ctrl+f5").as_deref(), Some("<Control>F5"));
        assert_eq!(
            to_accelerator("super+shift+s").as_deref(),
            Some("<Super><Shift>s")
        );
        assert_eq!(to_accelerator("shift+alt").as_deref(), None);
    }

    #[test]
    fn parses_existing_path_list() {
        assert_eq!(parse_path_list("@as []"), Vec::<String>::new());
        assert_eq!(parse_path_list("[]"), Vec::<String>::new());
        assert_eq!(
            parse_path_list("['/org/a/custom0/', '/org/a/custom1/']"),
            vec!["/org/a/custom0/", "/org/a/custom1/"]
        );
    }
}
