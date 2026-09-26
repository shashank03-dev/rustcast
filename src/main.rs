#![deny(clippy::dbg_macro)]

mod app;
mod calculator;
mod clipboard;
mod commands;
mod config;
mod debounce;
mod jev;
mod persist;
mod platform;
mod quit;
mod recorder;
mod styles;
mod unit_conversion;
mod utils;

use std::{collections::HashMap, fs::OpenOptions, path::Path};

use crate::{
    app::tile::{self, Hotkeys, Tile},
    config::Config,
    platform::{get_autostart_status, launching::Shortcut},
};

use log::info;
use tracing_subscriber::{EnvFilter, Layer, util::SubscriberInitExt};

fn main() -> iced::Result {
    // Strip AppImage/sharun library-path overrides inherited from the launching
    // shell (e.g. a Ghostty AppImage terminal) before anything loads GIO/GTK/GL
    // modules or shells out to gsettings — otherwise the tray, GL rendering, and
    // GNOME hotkey registration all break. Must be first, before any threads.
    crate::platform::sanitize_inherited_env();

    // The screenshot thumbnail overlay runs as a short-lived subprocess
    // (`--overlay <png>`) on the *Wayland* backend so its drag-and-drop reaches
    // every app — including native-Wayland windows like terminals — which an X11
    // drag source cannot. The main process below stays on X11. Handle this mode
    // before any X11 forcing so GTK picks Wayland.
    let cli_args: Vec<String> = std::env::args().collect();
    if let Some(pos) = cli_args.iter().position(|a| a == "--overlay") {
        if let Some(path) = cli_args.get(pos + 1) {
            crate::platform::linux::overlay_gtk::run(std::path::PathBuf::from(path));
        }
        return Ok(());
    }

    // The floating "● REC" pill is another short-lived GTK subprocess.
    if let Some(pos) = cli_args.iter().position(|a| a == "--rec-indicator") {
        crate::recorder::indicator::run(&cli_args[pos + 1..]);
        return Ok(());
    }

    // RustCast's global hotkeys, window tiling (EWMH), and paste injection
    // (XTEST) all rely on X11. On a Wayland session we run consistently through
    // XWayland: force winit, GTK (tray) and the clipboard onto the X11 backend
    // so every component agrees on one server. The real WAYLAND_DISPLAY is
    // stashed first so the overlay subprocess can run on Wayland.
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // SAFETY: set at the very start of main, before any threads are spawned.
        unsafe {
            if let Some(wd) = std::env::var_os("WAYLAND_DISPLAY") {
                std::env::set_var("RUSTCAST_WAYLAND_DISPLAY", wd);
            }
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    // Single-instance: if launched with a rustcast:// URL and another instance
    // is already running, forward the URL to it and exit.
    if let Some(url) = std::env::args().find(|a| a.starts_with("rustcast://")) {
        // Maintenance actions run locally without a daemon (used by uninstall).
        if url == "rustcast://unregister-hotkeys" {
            crate::platform::linux::gnome_hotkeys::unregister();
            return Ok(());
        }
        if crate::platform::urlscheme::forward_if_running(&url) {
            return Ok(());
        }
    }

    let home = std::env::var("HOME").unwrap();

    let file_path = home.clone() + "/.config/rustcast/config.toml";
    if !Path::new(&file_path).exists() {
        std::fs::create_dir_all(home.clone() + "/.config/rustcast").unwrap();
        std::fs::write(
            &file_path,
            toml::to_string(&Config::default()).unwrap_or_else(|x| x.to_string()),
        )
        .unwrap();
    }

    let mut config: Config = match std::fs::read_to_string(&file_path) {
        Ok(a) => toml::from_str(&a).unwrap_or(Config::default()),
        Err(_) => Config::default(),
    };

    config.start_at_login = get_autostart_status();

    if cfg!(debug_assertions) {
        let sub = tracing_subscriber::fmt().finish();
        EnvFilter::new("rustcast=info").with_subscriber(sub).init();
    } else {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(config.log_path.replace("~", &home))
            .unwrap();

        let sub = tracing_subscriber::fmt().with_writer(file).finish();
        EnvFilter::new("rustcast=info").with_subscriber(sub).init();
    };

    info!("Config loaded");

    let show_hide =
        Shortcut::parse(&config.toggle_hotkey).unwrap_or(Shortcut::parse("option+space").unwrap());

    let cbhist = Shortcut::parse(&config.clipboard_hotkey.to_lowercase())
        .unwrap_or_else(|_| Shortcut::parse("cmd+shift+c").unwrap());

    let screenshot = Shortcut::parse(&config.screenshot_hotkey.to_lowercase())
        .unwrap_or_else(|_| Shortcut::parse("super+shift+s").unwrap());

    let recorder = Shortcut::parse(&config.recorder_hotkey.to_lowercase())
        .unwrap_or_else(|_| Shortcut::parse("super+shift+r").unwrap());

    let mut shell_map = HashMap::new();

    for shell in &config.shells {
        if let Some(hk_str) = &shell.hotkey
            && let Ok(hk) = Shortcut::parse(hk_str)
        {
            shell_map.insert(hk, shell.clone());
        }
    }

    let hotkeys = Hotkeys {
        toggle: show_hide,
        clipboard_hotkey: cbhist,
        screenshot_hotkey: screenshot,
        recorder_hotkey: recorder,
        shells: shell_map,
        handle: None,
    };

    info!("Hotkeys loaded");
    info!("Starting rustcast");

    iced::daemon(
        move || tile::elm::new(hotkeys.clone(), &config),
        tile::update::handle_update,
        tile::elm::view,
    )
    .subscription(Tile::subscription)
    .theme(Tile::theme)
    .run()
}
