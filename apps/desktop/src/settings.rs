//! Fenêtre de réglages (provisoire, en attendant l'interface principale) :
//! saisie et test de la clé API du LLM, consommation.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vtt_core::usage::{Summary, Usage};
use vtt_core::{Config, PostProcessor};
use vtt_llm::{ApiKeyProvider, LlmCleaner};

use crate::costs::{self, LedgerStore};

/// Nom du secret dans le Gestionnaire d'identifiants (`VoiceToText/llm-api-key`).
const API_KEY_SECRET: &str = "llm-api-key";

const SAMPLE: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";

pub fn api_key_provider() -> ApiKeyProvider {
    Box::new(|| vtt_platform::load_secret(API_KEY_SECRET))
}

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("VoiceToText — Réglages")
        .inner_size(520.0, 820.0)
        .min_inner_size(420.0, 400.0)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("ouverture des réglages : {e}");
    }
}

#[derive(Serialize)]
pub struct SettingsView {
    has_api_key: bool,
    llm_enabled: bool,
    model: String,
    base_url: String,
    usage: Summary,
    currency: String,
}

#[tauri::command]
pub fn settings_view(
    config: State<'_, Arc<Config>>,
    ledger: State<'_, LedgerStore>,
) -> SettingsView {
    SettingsView {
        has_api_key: vtt_platform::load_secret(API_KEY_SECRET).is_some(),
        llm_enabled: config.llm.enabled,
        model: config.llm.model.clone(),
        base_url: config.llm.base_url.clone(),
        usage: costs::summary(&ledger),
        currency: config.llm.pricing.currency.clone(),
    }
}

#[tauri::command]
pub fn save_api_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("clé vide".into());
    }
    vtt_platform::store_secret(API_KEY_SECRET, key)?;
    log::info!("clé API enregistrée");
    Ok(())
}

#[tauri::command]
pub fn delete_api_key() -> Result<(), String> {
    vtt_platform::delete_secret(API_KEY_SECRET)?;
    log::info!("clé API supprimée");
    Ok(())
}

#[derive(Serialize)]
pub struct CleanupTest {
    input: String,
    output: String,
    duration_ms: u128,
    usage: Option<Usage>,
    cost: Option<f64>,
}

/// Envoie une phrase d'exemple au LLM. Exécutée hors du thread principal
/// (`async`) pour ne pas figer l'interface pendant l'appel réseau. Le test
/// est facturé : il compte dans la consommation.
#[tauri::command(async)]
pub fn test_cleanup(
    config: State<'_, Arc<Config>>,
    ledger: State<'_, LedgerStore>,
) -> Result<CleanupTest, String> {
    let cleaner = LlmCleaner::new(&config.llm, &config.dictionary, api_key_provider());
    let started = Instant::now();
    let cleaned = cleaner.process(SAMPLE).map_err(|e| e.to_string())?;
    let duration_ms = started.elapsed().as_millis();
    log::info!("test du nettoyage : {duration_ms} ms");
    if let Some(usage) = &cleaned.usage {
        costs::record(&ledger, usage, &config.llm.pricing);
    }
    Ok(CleanupTest {
        input: SAMPLE.into(),
        output: cleaned.text,
        duration_ms,
        cost: cleaned.usage.map(|u| config.llm.pricing.cost(&u)),
        usage: cleaned.usage,
    })
}
