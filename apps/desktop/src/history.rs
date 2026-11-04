//! Historique des dictées : enregistrement après chaque dictée, lecture et
//! effacement depuis la fenêtre principale.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::State;
use vtt_core::config::Pricing;
use vtt_core::history::{Entry, History};
use vtt_core::{AudioClip, Outcome, Target};

/// Nombre de dictées affichées dans la fenêtre.
const SHOWN: usize = 500;

/// Partagé entre le thread du pipeline (écriture) et la fenêtre (lecture).
/// `None` si l'historique est désactivé dans config.toml.
pub type HistoryStore = Arc<Mutex<Option<History>>>;

pub fn record(
    store: &HistoryStore,
    clip: &AudioClip,
    target: &Target,
    outcome: &Outcome,
    pricing: &Pricing,
) {
    let Outcome::Inserted {
        raw,
        text,
        post_process_error,
        snippet,
        command,
        usage,
        fallback,
        translated,
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
        command: *command,
        // Exécutable seulement : le titre de la fenêtre peut être personnel.
        app: target.app.clone(),
        usage: *usage,
        cost: usage.map(|u| pricing.cost(&u)),
        currency: usage.map(|_| pricing.currency.clone()),
        fallback: fallback.clone(),
        translated: translated.clone(),
    };
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(history) = guard.as_mut()
        && let Err(e) = history.append(&entry)
    {
        // Jamais bloquant : la dictée a déjà été insérée.
        log::warn!("historique : {e}");
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
