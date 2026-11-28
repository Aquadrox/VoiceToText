//! Mode traduction : activé ou coupé par raccourci (Alt+Maj+T) ou depuis le
//! menu de l'icône. Chaque dictée commencée pendant qu'il est actif est
//! nettoyée puis traduite par le LLM avant l'insertion.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, State, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::controller::ControllerState;
use crate::model;

/// Événement envoyé à la pastille : langue d'arrivée, ou `null` si coupé.
const EVENT: &str = "translation";

/// Événement envoyé à la fenêtre principale : état du pack local M2M100.
const PACK_EVENT: &str = "translation-pack";

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

/// État du pack de traduction local M2M100 (100 langues, ONNX Runtime).
#[derive(Debug, Clone, Serialize)]
pub struct PackStatus {
    pub installed: bool,
    pub missing_mb: u64,
    pub downloading: bool,
    pub progress: Option<u8>,
    pub error: Option<String>,
}

#[derive(Default)]
struct PackProgress {
    downloading: bool,
    progress: Option<u8>,
    error: Option<String>,
}

/// Gestionnaire du téléchargement et de la suppression du pack local M2M100.
pub struct TranslationPack {
    app: AppHandle,
    models_dir: PathBuf,
    state: Mutex<PackProgress>,
}

pub type TranslationPackState = Arc<TranslationPack>;

impl TranslationPack {
    pub fn new(app: AppHandle, models_dir: PathBuf) -> TranslationPackState {
        Arc::new(Self {
            app,
            models_dir,
            state: Mutex::new(PackProgress::default()),
        })
    }

    pub fn status(&self) -> PackStatus {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let installed = model::translation_local(&self.models_dir).is_some();
        PackStatus {
            installed,
            missing_mb: model::translation_missing_mb(&self.models_dir),
            downloading: guard.downloading,
            progress: guard.progress,
            error: guard.error.clone(),
        }
    }

    fn emit(&self) {
        let _ = self.app.emit(PACK_EVENT, self.status());
    }

    pub fn download(self: &Arc<Self>, controller: ControllerState) -> PackStatus {
        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.downloading {
                return self.status_locked(&guard);
            }
            if model::translation_local(&self.models_dir).is_some() {
                guard.error = None;
                guard.progress = None;
                controller.reload_translator();
                return self.status_locked(&guard);
            }
            guard.downloading = true;
            guard.progress = Some(0);
            guard.error = None;
        }
        self.emit();

        let pack = Arc::clone(self);
        thread::Builder::new()
            .name("translation-pack".into())
            .spawn(move || {
                log::info!("téléchargement du pack de traduction local M2M100…");
                let result = model::ensure_translation(&pack.models_dir, |percent| {
                    {
                        let mut guard = pack.state.lock().unwrap_or_else(|e| e.into_inner());
                        guard.progress = Some(percent);
                    }
                    pack.emit();
                });
                {
                    let mut guard = pack.state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.downloading = false;
                    guard.progress = None;
                    match result {
                        Ok(_) => {
                            guard.error = None;
                            log::info!("pack de traduction local M2M100 installé");
                        }
                        Err(e) => {
                            log::error!("téléchargement du pack de traduction local : {e}");
                            guard.error = Some(e);
                        }
                    }
                }
                if model::translation_local(&pack.models_dir).is_some() {
                    controller.reload_translator();
                }
                pack.emit();
            })
            .expect("création du thread pack de traduction");

        self.status()
    }

    pub fn delete(&self, controller: &ControllerState) -> Result<PackStatus, String> {
        {
            let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.downloading {
                return Err("téléchargement du pack déjà en cours".into());
            }
        }
        controller.clear_translator();
        model::delete_translation(&self.models_dir)?;
        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.error = None;
            guard.progress = None;
        }
        log::info!("pack de traduction local M2M100 supprimé");
        let status = self.status();
        self.emit();
        Ok(status)
    }

    fn status_locked(&self, guard: &PackProgress) -> PackStatus {
        PackStatus {
            installed: model::translation_local(&self.models_dir).is_some(),
            missing_mb: model::translation_missing_mb(&self.models_dir),
            downloading: guard.downloading,
            progress: guard.progress,
            error: guard.error.clone(),
        }
    }
}

#[tauri::command]
pub fn translation_pack_status(pack: State<'_, TranslationPackState>) -> PackStatus {
    pack.status()
}

#[tauri::command]
pub fn download_translation_pack(
    pack: State<'_, TranslationPackState>,
    controller: State<'_, ControllerState>,
) -> PackStatus {
    pack.download(Arc::clone(&controller))
}

#[tauri::command]
pub fn delete_translation_pack(
    pack: State<'_, TranslationPackState>,
    controller: State<'_, ControllerState>,
) -> Result<PackStatus, String> {
    pack.delete(&controller)
}
