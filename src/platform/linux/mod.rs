//! Linux (X11/XWayland) platform implementation. Public surface mirrors the
//! macOS backend so the shared UI code is platform-agnostic.

pub mod discovery;
pub mod events;
pub mod gnome_hotkeys;
pub mod launching;
pub mod overlay;
pub mod overlay_gtk;
pub mod urlscheme;
pub mod window;
pub mod x11;

pub use discovery::{default_app_paths, get_installed_apps};

use iced::wgpu::rwh::{RawWindowHandle, WindowHandle};

fn autostart_desktop_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart/rustcast.desktop"))
}

pub fn start_at_login() {
    let Some(path) = autostart_desktop_path() else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=RustCast\nExec={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        exe.display()
    );
    let _ = std::fs::write(path, desktop);
}

pub fn stop_at_login() {
    if let Some(path) = autostart_desktop_path() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn get_autostart_status() -> bool {
    autostart_desktop_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Drop AppImage/sharun library-path overrides inherited from the launching
/// shell.
///
/// When RustCast is started from inside an AppImage-bundled environment (e.g. a
/// Ghostty AppImage terminal), that shell exports `GIO_MODULE_DIR`,
/// `GDK_PIXBUF_MODULE_FILE`, `LD_LIBRARY_PATH`, `GSETTINGS_SCHEMA_DIR`, … all
/// pointing into the bundle's own libraries. A normally-linked binary then
/// dlopen's the *bundle's* GIO/GTK/GL modules, which are built against a
/// different glibc — so the tray dies, wgpu/GL contexts fail (the launcher and
/// clipboard/screenshot windows never render), and `gsettings` cannot load the
/// dconf backend, meaning hotkey registration silently writes to a throwaway
/// backend instead of the real GNOME settings.
///
/// If we detect such a bundle environment, remove the module-search overrides
/// so this process — and every `gsettings` subprocess we spawn — uses the
/// system libraries. Must run at the very start of `main`, before any threads.
pub fn sanitize_inherited_env() {
    let bundled = std::env::var_os("APPDIR").is_some()
        || std::env::var_os("SHARUN_DIR").is_some()
        || std::env::var_os("APPIMAGE").is_some();
    if !bundled {
        return;
    }

    const POLLUTERS: &[&str] = &[
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "GIO_MODULE_DIR",
        "GIO_EXTRA_MODULES",
        "GSETTINGS_SCHEMA_DIR",
        "GSETTINGS_BACKEND",
        "GDK_PIXBUF_MODULE_FILE",
        "GDK_PIXBUF_MODULEDIR",
        "GCONV_PATH",
        "GTK_PATH",
        "GTK_IM_MODULE_FILE",
        "LIBGL_DRIVERS_PATH",
        "LIBVA_DRIVERS_PATH",
        "GBM_BACKENDS_PATH",
        "__EGL_VENDOR_LIBRARY_DIRS",
    ];
    for var in POLLUTERS {
        // SAFETY: called at the very start of main, before any threads spawn.
        unsafe { std::env::remove_var(var) };
    }
    log::info!("Sanitized AppImage-inherited library paths from environment");
}

/// Make the launcher window always-on-top + sticky across workspaces.
/// Extracts the X11 window id from the raw window handle.
pub fn window_config(handle: &WindowHandle) {
    let xid = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    };
    if let Some(xid) = xid {
        x11::set_overlay_states(xid);
    } else {
        log::warn!("Window is not X11; always-on-top/sticky not applied (run under X11/XWayland)");
    }
}

/// Position the launcher window Spotlight/Raycast-style: horizontally centred on
/// the primary monitor with its top edge in the upper third of the screen. The
/// window grows downward as results stream in (winit keeps the top-left corner
/// fixed across resizes), so anchoring the top keeps it visually stable. No-op
/// for non-X11 handles.
pub fn position_launcher(handle: &WindowHandle) {
    let xid = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    };
    let Some(xid) = xid else { return };
    let Some(mon) = x11::primary_monitor() else {
        return;
    };

    let (width, height) = crate::app::launcher_size();
    let x = mon.x + ((mon.w as i32 - width as i32) / 2).max(0);
    // Upper third for the slim launcher; big pages are centred so they fit.
    let top = (mon.h as f32 * 0.18) as i32;
    let centred = ((mon.h as i32 - height as i32) / 2).max(0);
    let y = mon.y + top.min(centred.max(top.min(24)));
    // `move_window` waits for the WM to start managing the freshly-opened window
    // before issuing the move (otherwise the move races window mapping and the
    // launcher lands at the compositor's default top-left spot). That wait would
    // block the iced runtime, so do it on a background thread.
    std::thread::spawn(move || {
        x11::move_window(xid, x, y);
    });
}

/// Blur the desktop behind the launcher (KWin's blur-behind protocol), clipped
/// to its rounded corners. No-op for non-X11 handles.
pub fn blur_behind(handle: &WindowHandle, logical_width: f32, radius: f32) {
    if let RawWindowHandle::Xlib(h) = handle.as_raw() {
        x11::set_blur_behind(h.window as u32, logical_width, radius);
    } else if let RawWindowHandle::Xcb(h) = handle.as_raw() {
        x11::set_blur_behind(h.window.get(), logical_width, radius);
    }
}

/// winit already focuses the launcher window when it opens; nothing to do.
pub fn focus_this_app() {}

/// Explicitly take keyboard focus for our own launcher window. On GNOME/Wayland
/// (XWayland) a freshly-mapped launcher window briefly focuses and then mutter
/// pulls focus back, which the app reads as a click-away and hides the window.
/// Asking the WM to activate us (and setting input focus) keeps the window up
/// and able to receive keystrokes. No-op for non-X11 handles.
pub fn grab_focus(handle: &WindowHandle) {
    let xid = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    };
    if let Some(xid) = xid {
        x11::focus_window(xid);
    }
}

/// Paste into the currently-focused window via XTEST. The `pid` argument is
/// ignored on Linux (focus has already been restored to the target window by
/// `restore_frontmost`). A brief delay lets the focus change settle.
pub fn simulate_paste(_pid: i32) {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(90));
        x11::send_paste();
    });
}

/// Detect the system dark-mode preference via GNOME's `color-scheme` gsetting.
pub fn is_dark_mode() -> bool {
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("prefer-dark"))
        .unwrap_or(false)
}
