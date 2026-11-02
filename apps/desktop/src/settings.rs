//! Fenêtre de réglages (provisoire, en attendant l'interface principale) :
//! saisie et test de la clé API du LLM.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vtt_core::{Config, PostProcessor};
use vtt_llm::{ApiKeyProvider, LlmCleaner};

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
        .inner_size(520.0, 460.0)
        .resizable(false)
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
}

#[tauri::command]
pub fn settings_view(config: State<'_, Arc<Config>>) -> SettingsView {
    SettingsView {
        has_api_key: vtt_platform::load_secret(API_KEY_SECRET).is_some(),
        llm_enabled: config.llm.enabled,
        model: config.llm.model.clone(),
        base_url: config.llm.base_url.clone(),
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
}

/// Envoie une phrase d'exemple au LLM. Exécutée hors du thread principal
/// (`async`) pour ne pas figer l'interface pendant l'appel réseau.
#[tauri::command(async)]
pub fn test_cleanup(config: State<'_, Arc<Config>>) -> Result<CleanupTest, String> {
    let cleaner = LlmCleaner::new(&config.llm, &config.dictionary, api_key_provider());
    let started = Instant::now();
    let output = cleaner.process(SAMPLE).map_err(|e| e.to_string())?;
    let duration_ms = started.elapsed().as_millis();
    log::info!("test du nettoyage : {duration_ms} ms");
    Ok(CleanupTest {
        input: SAMPLE.into(),
        output,
        duration_ms,
    })
}
