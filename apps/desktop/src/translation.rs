//! Mode traduction : activé ou coupé par raccourci (Alt+Maj+T) ou depuis le
//! menu de l'icône. Chaque dictée commencée pendant qu'il est actif est
//! nettoyée puis traduite par le LLM avant l'insertion.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, State, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Événement envoyé à la pastille : langue d'arrivée, ou `null` si coupé.
const EVENT: &str = "translation";

pub struct Translation {
    active: AtomicBool,
    language: Mutex<String>,
    app: AppHandle,
    tray_item: CheckMenuItem<Wry>,
}

pub type TranslationState = Arc<Translation>;

impl Translation {
    pub fn new(app: AppHandle, tray_item: CheckMenuItem<Wry>, language: &str) -> TranslationState {
        let translation = Arc::new(Self {
            active: AtomicBool::new(false),
            language: Mutex::new(language.trim().to_owned()),
            app,
            tray_item,
        });
        translation.refresh_menu();
        translation
    }

    fn language(&self) -> String {
        self.language
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Langue d'arrivée si le mode est actif.
    pub fn current(&self) -> Option<String> {
        self.active.load(Ordering::SeqCst).then(|| self.language())
    }

    pub fn toggle(&self) {
        let active = !self.active.fetch_xor(true, Ordering::SeqCst);
        log::info!(
            "mode traduction {}",
            if active { "activé" } else { "coupé" }
        );
        self.refresh_menu();
        let _ = self.app.emit(EVENT, self.current());
    }

    /// Langue choisie dans les réglages.
    pub fn set_language(&self, language: &str) {
        let language = language.trim();
        {
            let mut current = self.language.lock().unwrap_or_else(|e| e.into_inner());
            if *current == language {
                return;
            }
            *current = language.to_owned();
        }
        self.refresh_menu();
        if self.active.load(Ordering::SeqCst) {
            let _ = self.app.emit(EVENT, self.current());
        }
    }

    fn refresh_menu(&self) {
        let _ = self
            .tray_item
            .set_text(format!("Mode traduction ({})", self.language()));
        let _ = self
            .tray_item
            .set_checked(self.active.load(Ordering::SeqCst));
    }
}

/// Enregistre le raccourci qui active ou coupe le mode ; un raccourci vide le
/// désactive. Un échec n'est pas fatal : il est journalisé et renvoyé.
pub fn register(
    app: &AppHandle,
    shortcut: &str,
    translation: TranslationState,
) -> Result<(), String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Ok(());
    }
    let registered = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| {
            if event.state == ShortcutState::Pressed {
                translation.toggle();
            }
        });
    match registered {
        Ok(()) => {
            log::info!("mode traduction : {shortcut}");
            Ok(())
        }
        Err(e) => {
            log::warn!("raccourci « {shortcut} » indisponible : {e}");
            Err(format!(
                "raccourci « {shortcut} » indisponible (déjà pris par une autre application ?)"
            ))
        }
    }
}

/// État initial pour la pastille (langue d'arrivée, ou `None` si coupé).
#[tauri::command]
pub fn translation_state(translation: State<'_, TranslationState>) -> Option<String> {
    translation.current()
}
