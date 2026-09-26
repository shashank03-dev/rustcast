//! Platform abstraction. This is the Linux (X11/XWayland) build; the module
//! re-exports the Linux backend under stable names so the rest of the codebase
//! is platform-agnostic.

pub mod linux;

pub use self::linux::{
    default_app_paths, events, get_autostart_status, get_installed_apps, is_dark_mode, launching,
    sanitize_inherited_env, start_at_login, stop_at_login, urlscheme, window,
};

use iced::wgpu::rwh::WindowHandle;

/// Apply platform window configuration (always-on-top / sticky).
pub fn window_config(handle: &WindowHandle) {
    self::linux::window_config(handle);
}

pub fn focus_this_app() {
    self::linux::focus_this_app();
}

/// Take keyboard focus for our own launcher window (see the Linux impl).
pub fn grab_focus(handle: &WindowHandle) {
    self::linux::grab_focus(handle);
}

/// Position the launcher window Spotlight-style (centred, upper third).
pub fn position_launcher(handle: &WindowHandle) {
    self::linux::position_launcher(handle);
}

pub fn simulate_paste(pid: i32) {
    self::linux::simulate_paste(pid);
}
