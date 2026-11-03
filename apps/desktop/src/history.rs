//! Fenêtre d'historique (provisoire, en attendant l'interface principale) :
//! dernières dictées, copie, effacement.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vtt_core::history::{Entry, History};
use vtt_core::{AudioClip, Outcome};

/// Nombre de dictées affichées dans la fenêtre.
const SHOWN: usize = 500;

/// Partagé entre le thread du pipeline (écriture) et la fenêtre (lecture).
/// `None` si l'historique est désactivé dans config.toml.
pub type HistoryStore = Arc<Mutex<Option<History>>>;

pub fn record(store: &HistoryStore, clip: &AudioClip, outcome: &Outcome) {
    let Outcome::Inserted {
        raw,
        text,
        post_process_error,
        snippet,
    } = outcome
    else {
        return;
    };
    let entry = Entry {
        at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
        duration_ms: clip.duration_ms(),
        raw: raw.clone(),
        text: text.clone(),
        snippet: *snippet,
        cleanup_failed: post_process_error.is_some(),
    };
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(history) = guard.as_mut()
        && let Err(e) = history.append(&entry)
    {
        // Jamais bloquant : la dictée a déjà été insérée.
        log::warn!("historique : {e}");
    }
}

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("history") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "history", WebviewUrl::App("history.html".into()))
        .title("VoiceToText — Historique")
        .inner_size(640.0, 620.0)
        .min_inner_size(420.0, 300.0)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("ouverture de l'historique : {e}");
    }
}

#[derive(serde::Serialize)]
pub struct HistoryView {
    enabled: bool,
    entries: Vec<Entry>,
}

#[tauri::command]
pub fn history_view(store: State<'_, HistoryStore>) -> Result<HistoryView, String> {
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        None => Ok(HistoryView {
            enabled: false,
            entries: Vec::new(),
        }),
        Some(history) => Ok(HistoryView {
            enabled: true,
            entries: history.recent(SHOWN).map_err(|e| e.to_string())?,
        }),
    }
}

#[tauri::command]
pub fn clear_history(store: State<'_, HistoryStore>) -> Result<(), String> {
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(history) = guard.as_mut() {
        history.clear().map_err(|e| e.to_string())?;
        log::info!("historique effacé");
    }
    Ok(())
}
