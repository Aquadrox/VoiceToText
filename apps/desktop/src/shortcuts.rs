//! Raccourcis globaux « Coller la version brute », « Réessayer » et « Mode
//! traduction » : réenregistrés sans redémarrer quand ils changent dans les
//! réglages.

use std::sync::Mutex;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use vtt_core::config::HotkeyConfig;

use crate::controller::WorkerHandle;
use crate::raw_paste::{self, LastRaw};
use crate::retry;
use crate::translation::{self, TranslationState};

#[derive(Clone, Copy)]
enum Kind {
    PasteRaw,
    Retry,
    Translate,
}

const KINDS: [Kind; 3] = [Kind::PasteRaw, Kind::Retry, Kind::Translate];

fn wanted(hotkey: &HotkeyConfig, kind: Kind) -> &str {
    match kind {
        Kind::PasteRaw => hotkey.paste_raw.trim(),
        Kind::Retry => hotkey.retry.trim(),
        Kind::Translate => hotkey.translate.trim(),
    }
}

pub struct Shortcuts {
    /// Raccourcis actuellement enregistrés, dans l'ordre de `KINDS` (vide :
    /// aucun).
    registered: Mutex<[String; 3]>,
    last_raw: LastRaw,
    worker: WorkerHandle,
    translation: TranslationState,
}

impl Shortcuts {
    pub fn new(last_raw: LastRaw, worker: WorkerHandle, translation: TranslationState) -> Self {
        Self {
            registered: Mutex::default(),
            last_raw,
            worker,
            translation,
        }
    }

    fn register(&self, app: &AppHandle, kind: Kind, shortcut: &str) -> Result<(), String> {
        match kind {
            Kind::PasteRaw => raw_paste::register(app, shortcut, self.last_raw.clone()),
            Kind::Retry => retry::register(app, shortcut, self.worker.clone()),
            Kind::Translate => translation::register(app, shortcut, self.translation.clone()),
        }
    }

    /// Enregistre les raccourcis de `hotkey` à la place des précédents ;
    /// renvoie les échecs (raccourci déjà pris par une autre application…).
    pub fn apply(&self, app: &AppHandle, hotkey: &HotkeyConfig) -> Vec<String> {
        let mut registered = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        // Libérer d'abord les anciens : deux raccourcis ont pu être échangés.
        for (old, kind) in registered.iter().zip(KINDS) {
            let new = wanted(hotkey, kind);
            if old != new
                && !old.is_empty()
                && let Err(e) = app.global_shortcut().unregister(old.as_str())
            {
                log::warn!("libération du raccourci « {old} » : {e}");
            }
        }
        let mut errors = Vec::new();
        for (old, kind) in registered.iter_mut().zip(KINDS) {
            let new = wanted(hotkey, kind);
            if old == new {
                continue;
            }
            *old = match self.register(app, kind, new) {
                Ok(()) => new.to_owned(),
                Err(e) => {
                    errors.push(e);
                    String::new()
                }
            };
        }
        errors
    }
}
