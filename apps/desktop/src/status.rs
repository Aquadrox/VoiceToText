//! État affiché à l'utilisateur : pastille flottante et menu de l'icône.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, State, Wry};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", content = "message", rename_all = "lowercase")]
pub enum Status {
    Idle,
    Listening,
    /// Enregistrement mains libres ; contient le nom du raccourci.
    #[serde(rename = "handsfree")]
    HandsFree(String),
    /// Enregistrement d'une consigne (Command Mode).
    Command,
    Processing,
    Loading(String),
    Error(String),
}

impl Status {
    fn tray_label(&self) -> String {
        match self {
            Status::Idle => "Prêt — maintenir le raccourci pour dicter".into(),
            Status::Listening => "Écoute…".into(),
            Status::HandsFree(key) => {
                format!("Mains libres — {key} pour terminer, Échap pour annuler")
            }
            Status::Command => "Commande : dictez la consigne pour le texte sélectionné…".into(),
            Status::Processing => "Transcription…".into(),
            Status::Loading(message) => message.clone(),
            Status::Error(message) => format!("Erreur : {message}"),
        }
    }
}

pub struct StatusReporter {
    app: AppHandle,
    tray_item: MenuItem<Wry>,
    current: Mutex<Status>,
}

impl StatusReporter {
    pub fn new(app: AppHandle, tray_item: MenuItem<Wry>) -> Self {
        Self {
            app,
            tray_item,
            current: Mutex::new(Status::Idle),
        }
    }

    pub fn set(&self, status: Status) {
        if let Status::Error(message) = &status {
            log::error!("{message}");
        }
        let _ = self.tray_item.set_text(status.tray_label());
        let _ = self.app.emit_to("overlay", "status", &status);
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }
}

/// Appelé par la pastille à son chargement pour récupérer l'état courant.
#[tauri::command]
pub fn current_status(reporter: State<'_, Arc<StatusReporter>>) -> Status {
    reporter
        .current
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
