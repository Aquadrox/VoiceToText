//! Intégration système : capture micro (toutes plateformes), hook clavier et
//! insertion de texte (Windows ; les autres OS viendront plus tard).

pub mod audio;

#[cfg(windows)]
mod secrets_windows;
#[cfg(windows)]
pub use secrets_windows::{delete_secret, load_secret, store_secret};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    ClipboardPaster, companion_keys, copy_selection, copy_text, foreground_target, key_code,
    key_is_down, modifiers_down, spawn_keyboard_hook, suppress_menu_activation,
};

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use unsupported::{
    ClipboardPaster, companion_keys, copy_selection, copy_text, delete_secret, foreground_target,
    key_code, key_is_down, load_secret, modifiers_down, spawn_keyboard_hook, store_secret,
    suppress_menu_activation,
};

use std::sync::OnceLock;
use std::time::Instant;

/// Appui ou relâchement d'une touche physique.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    /// Code de touche de la plateforme (virtual-key sous Windows).
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
