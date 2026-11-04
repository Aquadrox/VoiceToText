//! Mises à jour : vérification régulière des Releases GitHub (ou sur demande
//! depuis les réglages), avertissement (notification, menu de l'icône,
//! accueil) et installation sur demande.
//!
//! L'installeur téléchargé n'est exécuté qu'après vérification de sa
//! signature (clé publique dans tauri.conf.json) et de la version qu'elle
//! couvre (`requireSignedVersion`, contre le retour forcé à une ancienne
//! version). Rien n'est installé sans clic de l'utilisateur.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, State, Wry};
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
    /// Vérification régulière (`updates.check`), modifiable sans redémarrer.
    auto: AtomicBool,
}

pub type UpdatesState = Arc<Updates>;

const CHECK_LABEL: &str = "Rechercher une mise à jour";

impl Updates {
    pub fn new(app: &AppHandle, tray_item: MenuItem<Wry>, auto: bool) -> UpdatesState {
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
            auto: AtomicBool::new(auto),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_label(&self, text: &str) {
        let _ = self.tray_item.set_text(text);
    }
}

/// Lance les vérifications périodiques (sautées tant que `updates.check`
/// est désactivé).
pub fn spawn_checks(app: AppHandle, updates: UpdatesState) {
    if !updates.configured {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if updates.auto.load(Ordering::SeqCst) {
                let _ = check(&app, &updates).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Active ou coupe la vérification régulière ; à l'activation, vérifie tout
/// de suite.
pub fn set_auto(app: &AppHandle, updates: &UpdatesState, enabled: bool) {
    let was = updates.auto.swap(enabled, Ordering::SeqCst);
    if enabled && !was && updates.configured {
        let app = app.clone();
        let updates = Arc::clone(updates);
        tauri::async_runtime::spawn(async move {
            let _ = check(&app, &updates).await;
        });
    }
}

/// Vérifie une fois ; renvoie la version disponible.
async fn check(app: &AppHandle, updates: &Updates) -> Result<Option<String>, String> {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => {
            log::warn!("mises à jour : {e}");
            return Err(e.to_string());
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
            Ok(Some(version))
        }
        Ok(None) => {
            updates.lock().pending = None;
            updates.set_label(CHECK_LABEL);
            Ok(None)
        }
        Err(e) => {
            log::warn!("vérification des mises à jour : {e}");
            Err(format!("vérification impossible : {e}"))
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
            let _ = install(&app, &updates).await;
            return;
        }
        updates.set_label("Recherche en cours…");
        if !matches!(check(&app, &updates).await, Ok(Some(_))) {
            updates.set_label(&format!(
                "À jour (version {}) — rechercher à nouveau",
                app.package_info().version
            ));
        }
    });
}

/// Progression du téléchargement, envoyée aux pages (`update-progress`, en %).
const PROGRESS_EVENT: &str = "update-progress";

async fn install(app: &AppHandle, updates: &Updates) -> Result<(), String> {
    let update = {
        let mut inner = updates.lock();
        if inner.installing {
            return Err("installation déjà en cours".into());
        }
        let Some(update) = inner.pending.take() else {
            return Err("aucune mise à jour à installer : vérifiez d'abord".into());
        };
        inner.installing = true;
        update
    };
    log::info!("installation de la version {}", update.version);
    updates.set_label("Téléchargement de la mise à jour…");
    let mut received: u64 = 0;
    let mut last_percent = None;
    // Signature vérifiée avant l'exécution de l'installeur ; sous Windows,
    // l'installeur ferme l'application puis la relance.
    let result = update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                let percent = total
                    .filter(|t| *t > 0)
                    .map(|t| (received * 100 / t).min(100));
                if percent != last_percent {
                    last_percent = percent;
                    let _ = app.emit(PROGRESS_EVENT, percent);
                }
            },
            || {},
        )
        .await;
    if let Err(e) = result {
        log::error!("mise à jour : {e}");
        updates.set_label("Échec de la mise à jour — réessayer");
        let mut inner = updates.lock();
        inner.installing = false;
        inner.pending = Some(update);
        return Err(format!("échec de la mise à jour : {e}"));
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
    status(&app, &updates)
}

fn status(app: &AppHandle, updates: &Updates) -> UpdateStatus {
    UpdateStatus {
        current: app.package_info().version.to_string(),
        available: updates.lock().pending.as_ref().map(|u| u.version.clone()),
        configured: updates.configured,
    }
}

/// Bouton « Vérifier les mises à jour » des réglages.
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    updates: State<'_, UpdatesState>,
) -> Result<UpdateStatus, String> {
    let updates = updates.inner().clone();
    if updates.configured {
        check(&app, &updates).await?;
    }
    Ok(status(&app, &updates))
}

/// Boutons « Mettre à jour » de l'accueil et « Télécharger et installer »
/// des réglages : télécharge, vérifie la signature, installe et relance.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    updates: State<'_, UpdatesState>,
) -> Result<(), String> {
    let updates = updates.inner().clone();
    install(&app, &updates).await
}
