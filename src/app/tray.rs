//! Linux tray icon, driven on a dedicated GTK thread.
//!
//! `tray-icon` on Linux requires a running GTK/glib main loop and the
//! `TrayIcon` must be created and mutated on that loop's thread. iced owns the
//! main thread (winit), so we spawn a singleton GTK thread that owns the tray
//! and accept commands over a channel. [`TrayHandle`] is the `Send` handle the
//! UI code holds; it mirrors the small slice of the macOS `TrayIcon` API the
//! app actually uses (`set_visible`, menu rebuild, hide-on-drop semantics).

use std::sync::mpsc::{Sender, channel};

use once_cell::sync::OnceCell;

use crate::app::menubar::{menu_builder, tray_image};
use crate::app::tile::ExtSender;
use crate::config::Config;

enum TrayCommand {
    SetVisible(bool),
    SetMenu { config: Box<Config> },
}

static TRAY_TX: OnceCell<Sender<TrayCommand>> = OnceCell::new();

/// A lightweight, cloneable handle to the singleton tray thread.
#[derive(Clone, Debug)]
pub struct TrayHandle;

impl TrayHandle {
    pub fn set_visible(&self, visible: bool) {
        send(TrayCommand::SetVisible(visible));
    }

    pub fn set_menu(&self, config: Config) {
        send(TrayCommand::SetMenu {
            config: Box::new(config),
        });
    }
}

fn send(cmd: TrayCommand) {
    if let Some(tx) = TRAY_TX.get() {
        let _ = tx.send(cmd);
    }
}

/// Create (or reconfigure) the tray icon and return a handle. The GTK thread is
/// started once on first call; subsequent calls just push a fresh menu.
pub fn menu_icon(config: Config, sender: ExtSender) -> TrayHandle {
    let tx = TRAY_TX.get_or_init(|| {
        let (tx, rx) = channel::<TrayCommand>();
        std::thread::Builder::new()
            .name("rustcast-tray".to_string())
            .spawn(move || gtk_thread(rx, sender))
            .expect("failed to spawn tray thread");
        tx
    });

    let _ = tx.send(TrayCommand::SetMenu {
        config: Box::new(config),
    });
    let _ = tx.send(TrayCommand::SetVisible(true));

    TrayHandle
}

fn gtk_thread(rx: std::sync::mpsc::Receiver<TrayCommand>, sender: ExtSender) {
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    if gtk::init().is_err() {
        log::error!("GTK init failed; tray icon disabled");
        return;
    }

    let image = tray_image();
    let icon = Icon::from_rgba(image.as_bytes().to_vec(), image.width(), image.height()).ok();

    let tray: std::rc::Rc<std::cell::RefCell<Option<TrayIcon>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    {
        let tray = tray.clone();
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    TrayCommand::SetVisible(visible) => {
                        if !visible {
                            *tray.borrow_mut() = None;
                        } else if tray.borrow().is_none() {
                            if let Some(ic) = icon.clone() {
                                let built = TrayIconBuilder::new().with_icon(ic).build().ok();
                                *tray.borrow_mut() = built;
                            }
                        }
                    }
                    TrayCommand::SetMenu { config } => {
                        let menu = menu_builder(*config, sender.clone());
                        if tray.borrow().is_none() {
                            if let Some(ic) = icon.clone() {
                                if let Ok(built) = TrayIconBuilder::new()
                                    .with_icon(ic)
                                    .with_menu(Box::new(menu))
                                    .build()
                                {
                                    *tray.borrow_mut() = Some(built);
                                }
                            }
                        } else if let Some(t) = tray.borrow().as_ref() {
                            t.set_menu(Some(Box::new(menu)));
                        }
                    }
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    gtk::main();
}
