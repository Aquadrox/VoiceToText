//! « Réessayer » : retraite l'audio de la dernière dictée, gardé en mémoire
//! seulement. Utile après une erreur (transcription, réseau, collage
//! refusé) ou pour coller la dictée dans une autre fenêtre.
//!
//! Depuis le raccourci, le texte est collé là où se trouve le curseur ;
//! depuis le menu de l'icône, il est seulement copié (coller viserait le
//! menu lui-même).

use std::str::FromStr;
use std::thread;

use tauri::App;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use vtt_core::{Error, TextSink};

use crate::controller::{Work, WorkerHandle};
use crate::raw_paste;

/// Destination du texte retraité.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Collé dans la fenêtre active (raccourci).
    Paste,
    /// Copié dans le presse-papier seulement (menu de l'icône).
    Clipboard,
}

/// Copie le texte sans le coller.
pub struct ClipboardCopy;

impl TextSink for ClipboardCopy {
    fn insert(&self, text: &str) -> Result<(), Error> {
        vtt_platform::copy_text(text).map_err(Error::Sink)
    }
}

/// Raccourci saisi dans les réglages : vide ou compris par Tauri.
pub fn validate_shortcut(shortcut: &str, label: &str, example: &str) -> Result<(), String> {
    let shortcut = shortcut.trim();
    if !shortcut.is_empty() && Shortcut::from_str(shortcut).is_err() {
        return Err(format!(
            "raccourci « {label} » invalide : {shortcut} (ex. {example})"
        ));
    }
    Ok(())
}

/// Enregistre le raccourci ; un raccourci vide le désactive. Un échec (déjà
/// pris par une autre application, nom invalide) est journalisé, pas fatal.
pub fn register(app: &App, shortcut: &str, worker: WorkerHandle) {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return;
    }
    let registered = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| {
            if event.state == ShortcutState::Pressed {
                let worker = worker.clone();
                thread::spawn(move || {
                    // Le collage final ne doit pas devenir Ctrl+Alt+Maj+V.
                    if !raw_paste::wait_modifiers_released() {
                        log::warn!("réessayer : touches du raccourci toujours enfoncées");
                        return;
                    }
                    send(&worker, Delivery::Paste);
                });
            }
        });
    match registered {
        Ok(()) => log::info!("réessayer la dernière dictée : {shortcut}"),
        Err(e) => log::warn!("raccourci « {shortcut} » indisponible : {e}"),
    }
}

/// Confie le nouvel essai au thread de traitement.
pub fn send(worker: &WorkerHandle, delivery: Delivery) {
    let sender = worker.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match sender {
        Some(sender) => {
            let _ = sender.send(Work::Retry(delivery));
        }
        None => log::info!("réessayer : modèle encore en chargement"),
    }
}

#[cfg(test)]
mod tests {
    use vtt_core::config::HotkeyConfig;

    use super::*;

    #[test]
    fn default_shortcut_is_valid() {
        let default = HotkeyConfig::default().retry;
        assert!(validate_shortcut(&default, "réessayer", "Alt+Shift+R").is_ok());
        assert!(validate_shortcut("", "réessayer", "Alt+Shift+R").is_ok());
        assert!(validate_shortcut("Alt+Truc", "réessayer", "Alt+Shift+R").is_err());
    }
}
