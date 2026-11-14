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
    key_is_down, modifier_of, modifiers_down, paste_image, play_sound, pressed_keys, screenshot,
    shortcut_key, spawn_keyboard_hook, suppress_menu_activation,
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
    fullscreen_app_running, is_steam_deck, key_code, key_is_down, modifier_of, modifiers_down,
    pressed_keys, session_report, shortcut_key, spawn_keyboard_hook, suppress_menu_activation,
};
#[cfg(target_os = "linux")]
mod microphone_linux;
#[cfg(target_os = "linux")]
pub use microphone_linux::microphone_apps;
#[cfg(target_os = "linux")]
mod media_linux;
#[cfg(target_os = "linux")]
pub use media_linux::MediaPauser;
#[cfg(target_os = "linux")]
mod secrets_linux;
#[cfg(target_os = "linux")]
mod uinput;
#[cfg(target_os = "linux")]
pub use secrets_linux::{
    delete_secret, list_secrets, load_secret, protect, store_secret, unprotect,
};
#[cfg(target_os = "linux")]
mod sound;
#[cfg(target_os = "linux")]
pub use sound::play_sound;
// Linux : la capture commentée viendra plus tard.
#[cfg(target_os = "linux")]
pub use unsupported::{
    ClickRecorder, exclude_from_capture, foreground_rect, paste_image, screenshot,
};

/// Steam Deck : seulement sous Linux.
#[cfg(not(target_os = "linux"))]
pub fn is_steam_deck() -> bool {
    false
}

#[cfg(not(windows))]
#[cfg_attr(target_os = "linux", allow(dead_code))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::{
    ClickRecorder, ClipboardPaster, MediaPauser, ascii_path, autostart_enabled, companion_keys,
    copy_selection, copy_text, delete_secret, exclude_from_capture, foreground_rect,
    foreground_target, fullscreen_app_running, key_code, key_is_down, list_secrets, load_secret,
    microphone_apps, modifier_of, modifiers_down, paste_image, play_sound, pressed_keys, protect,
    screenshot, set_autostart, shortcut_key, spawn_keyboard_hook, store_secret,
    suppress_menu_activation, unprotect,
};

use std::sync::OnceLock;
use std::time::{Duration, Instant};

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

/// Attend qu'une touche du clavier soit enfoncée (détection de la touche de
/// dictée dans les réglages) et renvoie son code, ou `None` après `timeout`.
/// Les touches déjà enfoncées au départ ne comptent qu'une fois relâchées
/// puis enfoncées de nouveau. Fonctionne sans le suivi du clavier : la
/// détection marche même si le modèle n'est pas encore chargé.
pub fn capture_key(timeout: Duration) -> Option<u32> {
    let started = Instant::now();
    let mut held = pressed_keys();
    while started.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(15));
        let now = pressed_keys();
        let new: Vec<u32> = now.iter().copied().filter(|k| !held.contains(k)).collect();
        if !new.is_empty() {
            // AltGr sous Windows : Ctrl gauche + Alt droit d'un coup ; on
            // garde la touche qui a un nom.
            return Some(
                new.iter()
                    .copied()
                    .find(|&k| named_key(k).is_some())
                    .unwrap_or(new[0]),
            );
        }
        held = now;
    }
    None
}

/// Nom de la configuration pour un code de touche : « AltGr », « F13 »… ou
/// le code brut (`#183`) pour une touche sans nom.
pub fn key_name(code: u32) -> String {
    named_key(code).map_or_else(|| format!("#{code}"), str::to_owned)
}

fn named_key(code: u32) -> Option<&'static str> {
    vtt_core::config::HOTKEY_NAMES
        .iter()
        .copied()
        .find(|name| key_code(name) == Some(code))
}

/// Touche de modification d'un raccourci global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

impl Modifier {
    /// Nom dans la syntaxe des raccourcis (« Alt+Shift+V »).
    fn token(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Super => "Super",
        }
    }
}

/// Attend une combinaison (touches de modification puis une touche) et la
/// renvoie dans la syntaxe des raccourcis globaux : « Alt+Shift+V »,
/// « Ctrl+F9 »… Échap seule annule. Une lettre, un chiffre ou Espace sans
/// modificateur est refusé : le raccourci empêcherait de les taper.
pub fn capture_shortcut(timeout: Duration) -> Result<String, String> {
    let started = Instant::now();
    let mut held = pressed_keys();
    while started.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(15));
        let now = pressed_keys();
        let pressed = now
            .iter()
            .copied()
            .find(|k| !held.contains(k) && modifier_of(*k).is_none());
        if let Some(code) = pressed {
            let mut modifiers: Vec<Modifier> = now.iter().filter_map(|&k| modifier_of(k)).collect();
            modifiers.sort();
            modifiers.dedup();
            return shortcut_text(&modifiers, code);
        }
        held = now;
    }
    Err("aucun raccourci détecté en 10 secondes".into())
}

fn shortcut_text(modifiers: &[Modifier], code: u32) -> Result<String, String> {
    if modifiers.is_empty() && key_code("Escape") == Some(code) {
        return Err("détection annulée".into());
    }
    let key = shortcut_key(code).ok_or(
        "touche non utilisable dans un raccourci : essayez une lettre, un chiffre ou F1 à F24",
    )?;
    let typing = key.len() == 1 || key == "Space" || key == "Enter";
    if modifiers.is_empty() && typing {
        return Err(format!(
            "« {key} » seule empêcherait de taper ce caractère : ajoutez Ctrl, Alt ou Maj"
        ));
    }
    let mut parts: Vec<&str> = modifiers.iter().map(|m| m.token()).collect();
    parts.push(&key);
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        for name in vtt_core::config::HOTKEY_NAMES {
            if let Some(code) = key_code(name) {
                assert_eq!(key_name(code), *name);
            }
        }
        assert_eq!(key_name(1000), "#1000");
        assert_eq!(key_code(&key_name(1000)), Some(1000));
    }

    #[test]
    fn shortcut_texts() {
        let escape = key_code("Escape");
        if let Some(escape) = escape {
            assert!(shortcut_text(&[], escape).is_err());
        }
        if let Some(f9) = key_code("F9") {
            assert_eq!(shortcut_text(&[], f9).unwrap(), "F9");
            assert_eq!(
                shortcut_text(&[Modifier::Ctrl, Modifier::Shift], f9).unwrap(),
                "Ctrl+Shift+F9"
            );
        }
    }
}
