//! Raccourcis globaux « Coller la version brute » et « Réessayer » :
//! réenregistrés sans redémarrer quand ils changent dans les réglages.

use std::sync::Mutex;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use vtt_core::config::HotkeyConfig;

use crate::controller::WorkerHandle;
use crate::raw_paste::{self, LastRaw};
use crate::retry;

/// Raccourcis actuellement enregistrés (vide : aucun).
#[derive(Default)]
struct Registered {
    paste_raw: String,
    retry: String,
}

pub struct Shortcuts {
    registered: Mutex<Registered>,
    last_raw: LastRaw,
    worker: WorkerHandle,
}

impl Shortcuts {
    pub fn new(last_raw: LastRaw, worker: WorkerHandle) -> Self {
        Self {
            registered: Mutex::default(),
            last_raw,
            worker,
        }
    }

    /// Enregistre les raccourcis de `hotkey` à la place des précédents ;
    /// renvoie les échecs (raccourci déjà pris par une autre application…).
    pub fn apply(&self, app: &AppHandle, hotkey: &HotkeyConfig) -> Vec<String> {
        let mut registered = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        let paste_raw = hotkey.paste_raw.trim();
        let retry = hotkey.retry.trim();
        // Libérer d'abord les anciens : deux raccourcis ont pu être échangés.
        for (old, new) in [
            (&registered.paste_raw, paste_raw),
            (&registered.retry, retry),
        ] {
            if old != new
                && !old.is_empty()
                && let Err(e) = app.global_shortcut().unregister(old.as_str())
            {
                log::warn!("libération du raccourci « {old} » : {e}");
            }
        }
        let mut errors = Vec::new();
        if registered.paste_raw != paste_raw {
            registered.paste_raw = match raw_paste::register(app, paste_raw, self.last_raw.clone())
            {
                Ok(()) => paste_raw.to_owned(),
                Err(e) => {
                    errors.push(e);
                    String::new()
                }
            };
        }
        if registered.retry != retry {
            registered.retry = match retry::register(app, retry, self.worker.clone()) {
                Ok(()) => retry.to_owned(),
                Err(e) => {
                    errors.push(e);
                    String::new()
                }
            };
        }
        errors
    }
}
