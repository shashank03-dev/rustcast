//! This has the menubar icon logic for the app

use std::{collections::HashMap, io::Cursor};

use image::{DynamicImage, ImageReader};
use log::info;
use tray_icon::menu::{
    AboutMetadataBuilder, Icon as Ico, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem,
    Submenu,
};

use crate::{
    app::{Message, tile::ExtSender},
    commands::Function,
    config::Config,
    platform::launching::Shortcut,
    utils::open_url,
};

const REPO_URL: &str = "https://github.com/shashank03-dev/rustcast";

use tokio::runtime::Runtime;

pub fn menu_builder(config: Config, sender: ExtSender) -> Menu {
    let shortcut =
        Shortcut::parse(&config.toggle_hotkey).unwrap_or(Shortcut::parse("opt+space").unwrap());

    let mut modes = config.modes.clone();
    if !modes.contains_key("default") {
        modes.insert("Default".to_string(), "default".to_string());
    }

    init_event_handler(sender, shortcut);

    let header = version_item();
    let open = item("show_rustcast", "Open RustCast");
    let clipboard = item("open_clipboard", "Clipboard History");
    let screenshot = item("take_screenshot", "Take Screenshot");
    let ocr = item("copy_text", "Copy Text from Screen");
    let recorder = item("open_recorder", "Screen Recorder");
    let modes = (modes.len() > 1).then(|| mode_item(modes));
    let preferences = item("open_preferences", "Preferences…");
    let reload = item("refresh_rustcast", "Reload Config");
    let star = item("open_github_page", "Star on GitHub");
    let issue = item("open_issue_page", "Report an Issue…");
    let help = item("open_help_page", "Help");
    let about = about_item(tray_image());
    let hide = item("hide_tray_icon", "Hide Tray Icon");
    let quit = item("quit_rustcast", "Quit RustCast");
    let (s1, s2, s3, s4) = (
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::separator(),
        PredefinedMenuItem::separator(),
    );

    let mut items: Vec<&dyn IsMenuItem> = vec![
        &header,
        &open,
        &s1,
        &clipboard,
        &screenshot,
        &ocr,
        &recorder,
        &s2,
    ];
    if let Some(modes) = &modes {
        items.push(modes);
    }
    items.extend([
        &preferences as &dyn IsMenuItem,
        &reload,
        &s3,
        &star,
        &issue,
        &help,
        &s4,
        &about,
        &hide,
        &quit,
    ]);
    Menu::with_items(&items).unwrap()
}

pub fn tray_image() -> DynamicImage {
    ImageReader::new(Cursor::new(menubar_icon().unwrap_or_default()))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
}

fn init_event_handler(sender: ExtSender, shortcut: Shortcut) {
    let runtime = Runtime::new().unwrap();

    MenuEvent::set_event_handler(Some(move |x: MenuEvent| {
        let sender = sender.clone();
        let sender = sender.0.clone();
        info!("Menubar event called: {}", x.id.0);
        match x.id().0.as_str() {
            "refresh_rustcast" => {
                runtime.spawn(async move {
                    sender.clone().try_send(Message::ReloadConfig).unwrap();
                });
            }
            "hide_tray_icon" => {
                runtime
                    .spawn(async move { sender.clone().try_send(Message::HideTrayIcon).unwrap() });
            }
            "open_issue_page" => {
                open_url(&format!("{REPO_URL}/issues/new"));
            }
            "show_rustcast" => {
                runtime.spawn(async move {
                    sender
                        .clone()
                        .try_send(Message::KeyPressed(shortcut))
                        .unwrap();
                });
            }
            "open_help_page" => {
                open_url(&format!("{REPO_URL}#readme"));
            }
            "open_preferences" => {
                runtime.spawn(async move {
                    sender.clone().try_send(Message::OpenToSettings).unwrap();
                });
            }
            "open_github_page" => {
                open_url(REPO_URL);
            }
            // The tool shortcuts reuse the rustcast:// actions the hotkeys use.
            id @ ("open_clipboard" | "take_screenshot" | "copy_text" | "open_recorder") => {
                let action = match id {
                    "open_clipboard" => "clipboard",
                    "take_screenshot" => "screenshot",
                    "copy_text" => "ocr",
                    _ => "recorder",
                };
                let uri = format!("rustcast://{action}");
                runtime.spawn(async move {
                    sender.clone().try_send(Message::UriReceived(uri)).unwrap();
                });
            }
            // PredefinedMenuItem::quit is unsupported on Linux, so quit ourselves
            // (this also finishes a running recording cleanly).
            "quit_rustcast" => {
                runtime.spawn(async move {
                    sender
                        .clone()
                        .try_send(Message::RunFunction(Function::Quit))
                        .unwrap();
                });
            }
            id => {
                if id.starts_with("mode_switch_") {
                    let id = id.to_string();
                    runtime.spawn(async move {
                        sender
                            .clone()
                            .try_send(Message::SwitchMode(
                                id.strip_prefix("mode_switch_").unwrap_or("").to_string(),
                            ))
                            .unwrap();
                    });
                }
            }
        }
    }));
}

fn version_item() -> MenuItem {
    let version = "RustCast ".to_string() + option_env!("APP_VERSION").unwrap_or("");
    MenuItem::new(version.trim_end(), false, None)
}

fn item(id: &str, label: &str) -> MenuItem {
    MenuItem::with_id(id, label, true, None)
}

fn mode_item(modes: HashMap<String, String>) -> Submenu {
    let owned_items: Vec<MenuItem> = modes
        .keys()
        .map(|key| {
            MenuItem::with_id(
                format!("mode_switch_{}", key), // id uses the key
                format!("{}{}", key.split_at(1).0.to_uppercase(), key.split_at(1).1),
                true,
                None,
            )
        })
        .collect();

    let items: Vec<&dyn IsMenuItem> = owned_items.iter().map(|x| x as &dyn IsMenuItem).collect();

    Submenu::with_items("Modes", true, &items).unwrap()
}

fn about_item(image: DynamicImage) -> PredefinedMenuItem {
    let about_metadata_builder = AboutMetadataBuilder::new()
        .name(Some("RustCast"))
        .version(Some(
            option_env!("APP_VERSION").unwrap_or("Unknown Version"),
        ))
        .authors(Some(vec!["shashank03-dev".to_string()]))
        .credits(Some("shashank03-dev".to_string()))
        .comments(Some("Productivity launcher for Linux"))
        .icon({
            // The tray PNG is 512 px; GTK's About dialog shows icons at their
            // real size, so scale it down to a normal dialog icon.
            let icon = image
                .resize(128, 128, image::imageops::FilterType::Lanczos3)
                .into_rgba8();
            Ico::from_rgba(icon.as_raw().clone(), icon.width(), icon.height()).ok()
        })
        .website(Some(REPO_URL))
        .license(Some("MIT"))
        .build();

    PredefinedMenuItem::about(Some("About RustCast"), Some(about_metadata_builder))
}

fn menubar_icon() -> Option<Vec<u8>> {
    // The RustCast mark (see assets/icons/, rendered from assets/brand/).
    Some(include_bytes!("../../assets/icons/rustcast.png").to_vec())
}
