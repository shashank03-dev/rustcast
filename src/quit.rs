//! Enumerate and close other applications' windows via EWMH (`_NET_CLIENT_LIST`,
//! `_NET_CLOSE_WINDOW`). Replaces the macOS NSRunningApplication approach.

use std::collections::HashMap;

use crate::{
    app::apps::{App, AppCommand},
    commands::Function,
    platform::linux::x11,
};

/// Best display name for a toplevel window: prefer the WM_CLASS instance,
/// falling back to the window title.
fn app_name(title: &str, class: &str) -> String {
    let class = class.trim();
    if !class.is_empty() {
        // WM_CLASS is "instance\0class"; take the last token, capitalised.
        let last = class.split_whitespace().last().unwrap_or(class);
        let mut chars = last.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => last.to_string(),
        }
    } else {
        title.to_string()
    }
}

pub fn get_open_apps(_store_icons: bool) -> Vec<App> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    x11::client_list()
        .into_iter()
        .filter_map(|(_win, title, class)| {
            let name = app_name(&title, &class);
            if name.is_empty() || seen.insert(name.clone(), ()).is_some() {
                return None;
            }
            Some(App {
                ranking: 0,
                open_command: AppCommand::Function(Function::QuitApp(name.clone())),
                display_name: format!("Quit {}", name),
                icons: None,
                search_name: format!("quit {}", name.to_lowercase()),
                desc: name,
            })
        })
        .collect()
}

pub fn terminate_app(name: String) {
    let target = name.to_lowercase();
    for (win, title, class) in x11::client_list() {
        if app_name(&title, &class).to_lowercase() == target {
            x11::close_window(win);
        }
    }
}

pub fn terminate_all_apps() {
    for (win, _title, _class) in x11::client_list() {
        x11::close_window(win);
    }
}
