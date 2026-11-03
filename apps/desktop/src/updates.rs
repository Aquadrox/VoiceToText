//! Mises à jour : vérification régulière des Releases GitHub, avertissement
//! (notification, menu de l'icône, accueil) et installation sur demande.
//!
//! L'installeur téléchargé n'est exécuté qu'après vérification de sa
//! signature (clé publique dans tauri.conf.json) et de la version qu'elle
//! couvre (`requireSignedVersion`, contre le retour forcé à une ancienne
//! version). Rien n'est installé sans clic de l'utilisateur.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager, State, Wry};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

/// Première vérification après le démarrage (ne pas ralentir le lancement).
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Default)]
struct Inner {
    pending: Option<Update>,
    /// Version déjà annoncée par notification (une seule fois par version).
    notified: Option<String>,
    installing: bool,
}

/// État partagé : mise à jour trouvée, entrée du menu de l'icône.
pub struct Updates {
    inner: Mutex<Inner>,
    tray_item: MenuItem<Wry>,
    /// Clé publique présente dans la configuration : sans elle, aucune
    /// vérification (les releases ne seraient pas vérifiables).
    configured: bool,
}

pub type UpdatesState = Arc<Updates>;

const CHECK_LABEL: &str = "Rechercher une mise à jour";

impl Updates {
    pub fn new(app: &AppHandle, tray_item: MenuItem<Wry>) -> UpdatesState {
        let configured = app
            .config()
            .plugins
            .0
            .get("updater")
            .and_then(|u| u.get("pubkey"))
            .and_then(|k| k.as_str())
            .is_some_and(|k| !k.trim().is_empty());
        if !configured {
            log::info!("mises à jour : clé publique absente, vérification désactivée");
        }
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            tray_item,
            configured,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_label(&self, text: &str) {
        let _ = self.tray_item.set_text(text);
    }
}

/// Lance les vérifications périodiques.
pub fn spawn_checks(app: AppHandle, updates: UpdatesState) {
    if !updates.configured {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(&app, &updates).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Vérifie une fois ; renvoie la version disponible.
async fn check(app: &AppHandle, updates: &Updates) -> Option<String> {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => {
            log::warn!("mises à jour : {e}");
            return None;
        }
    };
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let notify = {
                let mut inner = updates.lock();
                inner.pending = Some(update);
                let first = inner.notified.as_deref() != Some(version.as_str());
                inner.notified = Some(version.clone());
                first
            };
            updates.set_label(&format!("Installer la version {version}…"));
            if notify {
                log::info!("mise à jour disponible : {version}");
                let _ = app
                    .notification()
                    .builder()
                    .title("VoiceToText : mise à jour disponible")
                    .body(format!(
                        "Version {version} prête. Menu de l'icône → « Installer la version {version} »."
                    ))
                    .show();
            }
            Some(version)
        }
        Ok(None) => {
            updates.lock().pending = None;
            updates.set_label(CHECK_LABEL);
            None
        }
        Err(e) => {
            log::warn!("vérification des mises à jour : {e}");
            None
        }
    }
}

/// Menu de l'icône : installe la mise à jour trouvée, sinon vérifie tout de suite.
pub fn on_tray_click(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updates = app.state::<UpdatesState>().inner().clone();
        if !updates.configured {
            updates.set_label("Mises à jour non configurées");
            return;
        }
        if updates.lock().pending.is_some() {
            install(&app, &updates).await;
            return;
        }
        updates.set_label("Recherche en cours…");
        if check(&app, &updates).await.is_none() {
            updates.set_label(&format!(
                "À jour (version {}) — rechercher à nouveau",
                app.package_info().version
            ));
        }
    });
}

async fn install(app: &AppHandle, updates: &Updates) {
    let update = {
        let mut inner = updates.lock();
        if inner.installing {
            return;
        }
        let Some(update) = inner.pending.take() else {
            return;
        };
        inner.installing = true;
        update
    };
    log::info!("installation de la version {}", update.version);
    updates.set_label("Téléchargement de la mise à jour…");
    // Signature vérifiée avant l'exécution de l'installeur ; sous Windows,
    // l'installeur ferme l'application puis la relance.
    let result = update.download_and_install(|_, _| {}, || {}).await;
    if let Err(e) = result {
        log::error!("mise à jour : {e}");
        updates.set_label("Échec de la mise à jour — réessayer");
        let mut inner = updates.lock();
        inner.installing = false;
        inner.pending = Some(update);
        return;
    }
    app.restart();
}

#[derive(Serialize)]
pub struct UpdateStatus {
    current: String,
    available: Option<String>,
    configured: bool,
}

#[tauri::command]
pub fn update_status(app: AppHandle, updates: State<'_, UpdatesState>) -> UpdateStatus {
    UpdateStatus {
        current: app.package_info().version.to_string(),
        available: updates.lock().pending.as_ref().map(|u| u.version.clone()),
        configured: updates.configured,
    }
}

/// Bouton « Mettre à jour » de l'accueil.
#[tauri::command]
pub fn install_update(app: AppHandle) {
    on_tray_click(&app);
}
