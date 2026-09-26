//! Global hotkey handling for Linux (X11/XWayland) via the `global-hotkey` crate.
//!
//! A single background thread owns the [`GlobalHotKeyManager`] and listens on
//! the global hotkey event channel. Registration requests are sent to it over
//! a channel. This mirrors the macOS `launching` module's public surface
//! (`Shortcut`, `EventTapHandle`, `global_handler`) so the shared UI code is
//! unchanged.

use std::sync::mpsc::{Sender as StdSender, channel};

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use once_cell::sync::OnceCell;

use crate::app::{Message, tile::ExtSender};

/// A registered global shortcut. Wraps a [`HotKey`]; identity (Eq/Hash) comes
/// from the underlying hotkey definition, so equal shortcuts compare equal —
/// which the shared code relies on (e.g. `shells` HashMap keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub hotkey: HotKey,
}

impl Shortcut {
    #[allow(dead_code)]
    pub fn id(&self) -> u32 {
        self.hotkey.id()
    }

    /// Parse a "cmd+shift+c" style string into a [`Shortcut`].
    ///
    /// Modifier aliases match the macOS build: cmd/command/super → Super,
    /// opt/option/alt → Alt, ctrl/control → Control, shift → Shift.
    /// `fn`/`capslock` are accepted but ignored (no X11 global-grab equivalent).
    pub fn parse(s: &str) -> Result<Shortcut, String> {
        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();

        let mut mods = Modifiers::empty();
        let mut code: Option<Code> = None;

        for part in &parts {
            match part.to_lowercase().as_str() {
                "cmd" | "command" | "super" | "meta" | "logo" => mods |= Modifiers::SUPER,
                "opt" | "option" | "alt" => mods |= Modifiers::ALT,
                "ctrl" | "control" => mods |= Modifiers::CONTROL,
                "shift" => mods |= Modifiers::SHIFT,
                "fn" | "function" | "capslock" | "caps" | "caps lock" => {}
                key => {
                    if code.is_some() {
                        return Err(format!("Multiple keys specified: '{}'", s));
                    }
                    code = Some(str_to_code(key)?);
                }
            }
        }

        let code = code.ok_or_else(|| format!("No key specified in shortcut: '{}'", s))?;
        let mods = if mods.is_empty() { None } else { Some(mods) };

        Ok(Shortcut {
            hotkey: HotKey::new(mods, code),
        })
    }
}

fn str_to_code(s: &str) -> Result<Code, String> {
    let code = match s.to_lowercase().as_str() {
        "a" => Code::KeyA,
        "b" => Code::KeyB,
        "c" => Code::KeyC,
        "d" => Code::KeyD,
        "e" => Code::KeyE,
        "f" => Code::KeyF,
        "g" => Code::KeyG,
        "h" => Code::KeyH,
        "i" => Code::KeyI,
        "j" => Code::KeyJ,
        "k" => Code::KeyK,
        "l" => Code::KeyL,
        "m" => Code::KeyM,
        "n" => Code::KeyN,
        "o" => Code::KeyO,
        "p" => Code::KeyP,
        "q" => Code::KeyQ,
        "r" => Code::KeyR,
        "s" => Code::KeyS,
        "t" => Code::KeyT,
        "u" => Code::KeyU,
        "v" => Code::KeyV,
        "w" => Code::KeyW,
        "x" => Code::KeyX,
        "y" => Code::KeyY,
        "z" => Code::KeyZ,

        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,

        "return" | "enter" => Code::Enter,
        "tab" => Code::Tab,
        "space" => Code::Space,
        "delete" | "backspace" => Code::Backspace,
        "escape" | "esc" => Code::Escape,
        "left" | "arrowleft" => Code::ArrowLeft,
        "right" | "arrowright" => Code::ArrowRight,
        "down" | "arrowdown" => Code::ArrowDown,
        "up" | "arrowup" => Code::ArrowUp,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,

        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,

        "-" | "minus" => Code::Minus,
        "=" | "equal" => Code::Equal,
        "[" | "bracketleft" => Code::BracketLeft,
        "]" | "bracketright" => Code::BracketRight,
        "\\" | "backslash" => Code::Backslash,
        ";" | "semicolon" => Code::Semicolon,
        "'" | "quote" => Code::Quote,
        "`" | "backquote" | "grave" => Code::Backquote,
        "," | "comma" => Code::Comma,
        "." | "period" => Code::Period,
        "/" | "slash" => Code::Slash,

        _ => return Err(format!("Unknown key: '{}'", s)),
    };

    Ok(code)
}

/// Opaque handle returned by [`global_handler`]. Kept in the Tile to mirror the
/// macOS `EventTapHandle`; the real resources live on the hotkey thread.
#[derive(Clone, Debug)]
pub struct EventTapHandle;

/// Command sent to the hotkey thread.
struct SetTargets {
    targets: Vec<Shortcut>,
    sender: ExtSender,
}

static CMD_TX: OnceCell<StdSender<SetTargets>> = OnceCell::new();

/// Register the given shortcuts globally and forward presses as
/// [`Message::KeyPressed`] through `sender`. Replaces any previously-registered
/// set. Starts the hotkey thread on first call.
pub fn global_handler(sender: ExtSender, targets: Vec<Shortcut>) -> Result<EventTapHandle, String> {
    let tx = CMD_TX.get_or_init(|| {
        let (tx, rx) = channel::<SetTargets>();
        std::thread::Builder::new()
            .name("rustcast-hotkeys".to_string())
            .spawn(move || hotkey_thread(rx))
            .expect("failed to spawn hotkey thread");
        tx
    });

    tx.send(SetTargets { targets, sender })
        .map_err(|e| format!("hotkey thread gone: {e}"))?;

    Ok(EventTapHandle)
}

fn hotkey_thread(rx: std::sync::mpsc::Receiver<SetTargets>) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Could not create global hotkey manager: {e}");
            return;
        }
    };

    let event_rx = GlobalHotKeyEvent::receiver();

    let mut registered: Vec<HotKey> = Vec::new();
    let mut map: std::collections::HashMap<u32, Shortcut> = std::collections::HashMap::new();
    let mut current_sender: Option<ExtSender> = None;

    loop {
        // Apply any pending registration changes (drain everything queued).
        while let Ok(set) = rx.try_recv() {
            for hk in registered.drain(..) {
                let _ = manager.unregister(hk);
            }
            map.clear();
            for sc in &set.targets {
                match manager.register(sc.hotkey) {
                    Ok(()) => {
                        registered.push(sc.hotkey);
                        map.insert(sc.hotkey.id(), *sc);
                    }
                    Err(e) => log::warn!("Could not register hotkey {:?}: {e}", sc),
                }
            }
            current_sender = Some(set.sender);
            log::info!("Registered {} global hotkeys", registered.len());
        }

        // Drain hotkey events.
        while let Ok(event) = event_rx.try_recv() {
            if event.state != HotKeyState::Pressed {
                continue;
            }
            if let (Some(sc), Some(sender)) = (map.get(&event.id), current_sender.as_ref()) {
                let mut s = sender.0.clone();
                let _ = s.try_send(Message::KeyPressed(*sc));
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_builds_equal_shortcuts_for_equal_strings() {
        let a = Shortcut::parse("cmd+shift+c").unwrap();
        let b = Shortcut::parse("super+shift+c").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn parse_rejects_unknown_key() {
        assert!(Shortcut::parse("ctrl+£").is_err());
    }

    #[test]
    fn parse_distinguishes_different_keys() {
        let a = Shortcut::parse("alt+space").unwrap();
        let b = Shortcut::parse("alt+enter").unwrap();
        assert_ne!(a, b);
    }
}
