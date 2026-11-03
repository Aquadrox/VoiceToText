//! « Coller la version brute » : raccourci global qui colle la transcription
//! Whisper de la dernière dictée, sans le nettoyage du LLM (utile quand le
//! nettoyage a trop réécrit : Ctrl+Z puis le raccourci).
//!
//! La dernière transcription reste en mémoire seulement, jamais sur disque
//! (l'historique, lui, peut être désactivé).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::App;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vtt_core::TextSink;
use vtt_platform::ClipboardPaster;

/// Transcription brute de la dernière dictée insérée.
pub type LastRaw = Arc<Mutex<Option<String>>>;

/// Délai maximal pour que l'utilisateur relâche les touches du raccourci.
const MODIFIERS_TIMEOUT: Duration = Duration::from_secs(2);

pub fn remember(last: &LastRaw, raw: &str) {
    *last.lock().unwrap_or_else(|e| e.into_inner()) = Some(raw.to_owned());
}

fn take_copy(last: &LastRaw) -> Option<String> {
    last.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Enregistre le raccourci ; un raccourci vide le désactive. Un échec (déjà
/// pris par une autre application, nom invalide) est journalisé, pas fatal.
pub fn register(app: &App, shortcut: &str, last: LastRaw) {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return;
    }
    let registered = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| {
            if event.state == ShortcutState::Pressed {
                let last = Arc::clone(&last);
                thread::spawn(move || paste(&last));
            }
        });
    match registered {
        Ok(()) => log::info!("coller la version brute : {shortcut}"),
        Err(e) => log::warn!("raccourci « {shortcut} » indisponible : {e}"),
    }
}

fn paste(last: &LastRaw) {
    let Some(raw) = take_copy(last) else {
        log::info!("coller la version brute : aucune dictée depuis le démarrage");
        return;
    };
    let started = Instant::now();
    while vtt_platform::modifiers_down() {
        if started.elapsed() > MODIFIERS_TIMEOUT {
            log::warn!("coller la version brute : touches du raccourci toujours enfoncées");
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    match ClipboardPaster.insert(&raw) {
        Ok(()) => log::info!("version brute collée ({} caractères)", raw.chars().count()),
        Err(e) => log::warn!("coller la version brute : {e}"),
    }
}

/// Menu de l'icône : copie dans le presse-papier (coller depuis le menu
/// viserait le menu lui-même, pas l'application).
pub fn copy(last: &LastRaw) {
    let Some(raw) = take_copy(last) else {
        return;
    };
    if let Err(e) = vtt_platform::copy_text(&raw) {
        log::warn!("copie de la version brute : {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use tauri_plugin_global_shortcut::Shortcut;
    use vtt_core::config::HotkeyConfig;

    use super::*;

    #[test]
    fn default_shortcut_is_valid() {
        let default = HotkeyConfig::default().paste_raw;
        assert!(Shortcut::from_str(&default).is_ok(), "{default}");
    }

    #[test]
    fn remembers_last_raw_only() {
        let last = LastRaw::default();
        assert_eq!(take_copy(&last), None);
        remember(&last, "euh bonjour");
        remember(&last, "deuxième");
        assert_eq!(take_copy(&last).as_deref(), Some("deuxième"));
    }
}
