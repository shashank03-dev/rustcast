//! `rustcast://` deep-link handling for Linux.
//!
//! Uses a single-instance Unix domain socket: a second launch carrying a
//! `rustcast://` URL forwards it to the already-running instance and exits.
//! The running instance reads URLs off the socket via [`url_stream`].
//! [`install`] registers the URL scheme + an autostart-independent desktop
//! handler so the desktop can route `rustcast://` links here.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use once_cell::sync::Lazy;

use crate::app::Message;

static URL_CHANNEL: Lazy<(Sender<String>, Receiver<String>)> =
    Lazy::new(crossbeam_channel::unbounded);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("rustcast.sock")
}

/// If another RustCast instance owns the socket, forward `url` to it and return
/// true (caller should exit). Returns false if no instance is reachable.
pub fn forward_if_running(url: &str) -> bool {
    if let Ok(mut stream) = UnixStream::connect(socket_path()) {
        let _ = stream.write_all(url.as_bytes());
        return true;
    }
    false
}

/// True when another RustCast instance is listening on the socket.
pub fn is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Bind the single-instance socket and start the desktop URL handler.
/// Safe to call once at startup.
pub fn install() {
    let path = socket_path();
    // Remove a stale socket left by a crashed instance.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }

    match UnixListener::bind(&path) {
        Ok(listener) => {
            std::thread::Builder::new()
                .name("rustcast-url-socket".to_string())
                .spawn(move || accept_loop(listener))
                .ok();
        }
        Err(e) => log::warn!("Could not bind rustcast url socket: {e}"),
    }

    register_scheme_handler();
}

fn accept_loop(listener: UnixListener) {
    for stream in listener.incoming() {
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(mut stream) = stream {
            let mut buf = String::new();
            if stream.read_to_string(&mut buf).is_ok() {
                let url = buf.trim().to_string();
                if url.to_lowercase().starts_with("rustcast://") {
                    let _ = URL_CHANNEL.0.send(url);
                }
            }
        }
    }
}

/// Write a desktop entry registering this binary as the `x-scheme-handler/rustcast`
/// handler so the desktop environment can deliver `rustcast://` links.
fn register_scheme_handler() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(apps_dir) = dirs::data_dir().map(|d| d.join("applications")) else {
        return;
    };
    if std::fs::create_dir_all(&apps_dir).is_err() {
        return;
    }
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=RustCast\nExec={} %u\nNoDisplay=true\nMimeType=x-scheme-handler/rustcast;\n",
        exe.display()
    );
    let desktop_path = apps_dir.join("rustcast-url-handler.desktop");
    if std::fs::write(&desktop_path, desktop).is_ok() {
        // Best-effort default registration.
        std::process::Command::new("xdg-mime")
            .args([
                "default",
                "rustcast-url-handler.desktop",
                "x-scheme-handler/rustcast",
            ])
            .status()
            .ok();
    }
}

#[allow(dead_code)]
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
    // Wake the accept loop and clean up.
    let _ = UnixStream::connect(socket_path());
    let _ = std::fs::remove_file(socket_path());
}

/// Async stream that yields `rustcast://` URLs received on the socket.
pub fn url_stream() -> impl iced::futures::Stream<Item = Message> {
    iced::futures::stream::unfold((), |()| async {
        let url = tokio::task::spawn_blocking(|| {
            loop {
                if SHUTTING_DOWN.load(Ordering::Relaxed) {
                    return None;
                }
                match URL_CHANNEL.1.recv_timeout(Duration::from_millis(500)) {
                    Ok(url) => return Some(url),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return None,
                }
            }
        })
        .await
        .ok()
        .flatten()?;

        Some((Message::UriReceived(url), ()))
    })
}
