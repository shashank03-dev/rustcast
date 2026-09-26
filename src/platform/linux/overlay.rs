//! Screenshot thumbnail overlay launcher.
//!
//! The overlay runs as a short-lived subprocess (`rustcast --overlay <png>`)
//! using GTK on the **X11/XWayland** backend. X11 is required so the overlay can
//! pin itself to the bottom-left corner and cascade-stack multiple screenshots —
//! GNOME/Wayland forbids clients from self-positioning. GTK's drag is a standard
//! X11 drag, which XWayland bridges to native-Wayland apps too. The window +
//! drag source live in [`super::overlay_gtk`].

use std::path::PathBuf;
use std::process::Command;

/// Show the screenshot thumbnail for `path` by spawning the overlay subprocess.
pub fn show_thumbnail(path: PathBuf) {
    let Ok(exe) = std::env::current_exe() else {
        log::warn!("cannot locate own exe to spawn overlay");
        return;
    };

    let mut cmd = Command::new(exe);
    cmd.arg("--overlay").arg(&path);

    // Force the overlay onto X11/XWayland (the main process already runs there),
    // so GTK can position the window in the corner. The real WAYLAND_DISPLAY was
    // stashed away by `main`; deliberately do NOT restore it here.
    cmd.env("GDK_BACKEND", "x11");
    cmd.env_remove("WAYLAND_DISPLAY");

    if let Err(e) = cmd.spawn() {
        log::warn!("failed to spawn screenshot overlay: {e}");
    }
}
