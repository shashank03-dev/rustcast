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

    Menu::with_items(&[
        &version_item(),
        &about_item(tray_image()),
        &open_github_item(),
        &PredefinedMenuItem::separator(),
        &refresh_item(),
        &open_item(),
        &mode_item(modes),
        &PredefinedMenuItem::separator(),
        &open_issue_item(),
        &get_help_item(),
        &PredefinedMenuItem::separator(),
        &open_settings_item(),
        &hide_tray_icon(),
        &quit_item(),
    ])
    .unwrap()
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
    let shortcut = shortcut.clone();

    MenuEvent::set_event_handler(Some(move |x: MenuEvent| {
        let shortcut = shortcut.clone();
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
                        .try_send(Message::KeyPressed(shortcut.clone()))
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
    let version = "RustCast: ".to_string() + option_env!("APP_VERSION").unwrap_or("Unknown");
    MenuItem::new(version, false, None)
}

fn hide_tray_icon() -> MenuItem {
    MenuItem::with_id("hide_tray_icon", "Hide Tray Icon", true, None)
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

fn open_item() -> MenuItem {
    MenuItem::with_id("show_rustcast", "Toggle View", true, None)
}

fn open_github_item() -> MenuItem {
    MenuItem::with_id("open_github_page", "Star on Github", true, None)
}

fn open_issue_item() -> MenuItem {
    MenuItem::with_id("open_issue_page", "Report an Issue", true, None)
}

fn refresh_item() -> MenuItem {
    MenuItem::with_id("refresh_rustcast", "Refresh", true, None)
}

fn open_settings_item() -> MenuItem {
    MenuItem::with_id("open_preferences", "Open Preferences", true, None)
}

fn get_help_item() -> MenuItem {
    MenuItem::with_id("open_help_page", "Help", true, None)
}

fn quit_item() -> PredefinedMenuItem {
    PredefinedMenuItem::quit(Some("Quit"))
}

fn about_item(image: DynamicImage) -> PredefinedMenuItem {
    let about_metadata_builder = AboutMetadataBuilder::new()
        .name(Some("RustCast"))
        .version(Some(
            option_env!("APP_VERSION").unwrap_or("Unknown Version"),
        ))
        .authors(Some(vec!["shashank03-dev".to_string()]))
        .credits(Some("shashank03-dev".to_string()))
        .icon(Ico::from_rgba(image.as_bytes().to_vec(), image.width(), image.height()).ok())
        .website(Some(REPO_URL))
        .license(Some("MIT"))
        .build();

    PredefinedMenuItem::about(Some("About.."), Some(about_metadata_builder))
}

fn menubar_icon() -> Option<Vec<u8>> {
    // The RustCast mark (see assets/icons/, rendered from assets/brand/).
    Some(include_bytes!("../../assets/icons/rustcast.png").to_vec())
}
