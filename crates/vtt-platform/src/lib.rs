//! Intégration système : capture micro (toutes plateformes), hook clavier et
//! insertion de texte (Windows, Linux sous X11 ; pas encore les autres OS).

pub mod audio;
pub mod capture;

#[cfg(windows)]
mod autostart_windows;
#[cfg(windows)]
pub use autostart_windows::{autostart_enabled, set_autostart};
#[cfg(windows)]
mod media_windows;
#[cfg(windows)]
pub use media_windows::MediaPauser;
#[cfg(windows)]
mod microphone_windows;
#[cfg(windows)]
pub use microphone_windows::microphone_apps;
#[cfg(windows)]
mod secrets_windows;
#[cfg(windows)]
pub use secrets_windows::{
    delete_secret, list_secrets, load_secret, protect, store_secret, unprotect,
};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    ClickRecorder, ClipboardPaster, ascii_path, companion_keys, copy_selection, copy_text,
    exclude_from_capture, foreground_rect, foreground_target, fullscreen_app_running, key_code,
    key_is_down, modifiers_down, paste_image, play_sound, screenshot, spawn_keyboard_hook,
    suppress_menu_activation,
};

#[cfg(target_os = "linux")]
mod autostart_linux;
#[cfg(target_os = "linux")]
pub use autostart_linux::{autostart_enabled, set_autostart};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    ClipboardPaster, ascii_path, companion_keys, copy_selection, copy_text, foreground_target,
    fullscreen_app_running, key_code, key_is_down, modifiers_down, spawn_keyboard_hook,
    suppress_menu_activation,
};
#[cfg(target_os = "linux")]
mod secrets_linux;
#[cfg(target_os = "linux")]
pub use secrets_linux::{
    delete_secret, list_secrets, load_secret, protect, store_secret, unprotect,
};
#[cfg(target_os = "linux")]
mod sound;
#[cfg(target_os = "linux")]
pub use sound::play_sound;
// Linux : capture commentée, contrôles multimédias et applications qui
// utilisent le micro viendront plus tard.
#[cfg(target_os = "linux")]
pub use unsupported::{
    ClickRecorder, MediaPauser, exclude_from_capture, foreground_rect, microphone_apps,
    paste_image, screenshot,
};

#[cfg(not(windows))]
#[cfg_attr(target_os = "linux", allow(dead_code))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::{
    ClickRecorder, ClipboardPaster, MediaPauser, ascii_path, autostart_enabled, companion_keys,
    copy_selection, copy_text, delete_secret, exclude_from_capture, foreground_rect,
    foreground_target, fullscreen_app_running, key_code, key_is_down, list_secrets, load_secret,
    microphone_apps, modifiers_down, paste_image, play_sound, protect, screenshot, set_autostart,
    spawn_keyboard_hook, store_secret, suppress_menu_activation, unprotect,
};

use std::sync::OnceLock;
use std::time::Instant;

/// Appui ou relâchement d'une touche physique.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    /// Code de touche de la plateforme (virtual-key sous Windows, code evdev
    /// du noyau sous Linux).
    pub key: u32,
    pub down: bool,
    /// Horodatage en millisecondes, même horloge que [`now_ms`].
    pub at_ms: u64,
}

/// Horloge monotone en millisecondes depuis le premier appel.
pub fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}
