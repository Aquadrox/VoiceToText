//! Fenêtre de réglages (provisoire, en attendant l'interface principale) :
//! fournisseur LLM, clé API, test, consommation.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vtt_core::config::LlmConfig;
use vtt_core::usage::{Summary, Usage};
use vtt_core::{Config, PostProcessor};
use vtt_llm::{ApiKeyProvider, LlmCleaner};

use crate::costs::{self, LedgerStore};
use crate::paths::Paths;

/// Secret historique (DeepSeek) dans le Gestionnaire d'identifiants :
/// `VoiceToText/llm-api-key`.
const LEGACY_API_KEY_SECRET: &str = "llm-api-key";

/// Champs gérés par l'application : `extra_body` ne doit pas les remplacer.
const RESERVED_FIELDS: &[&str] = &["model", "messages", "stream"];

const SAMPLE: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";

/// Une clé par fournisseur (`VoiceToText/llm-api-key/api.z.ai`…) : changer de
/// fournisseur ne fait pas perdre la clé du précédent. DeepSeek garde le nom
/// historique.
fn secret_name(llm: &LlmConfig) -> String {
    match llm.host() {
        None | Some("api.deepseek.com") => LEGACY_API_KEY_SECRET.into(),
        Some(host) => format!("{LEGACY_API_KEY_SECRET}/{host}"),
    }
}

pub fn api_key_provider(llm: &LlmConfig) -> ApiKeyProvider {
    let name = secret_name(llm);
    Box::new(move || vtt_platform::load_secret(&name))
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
        .inner_size(560.0, 860.0)
        .min_inner_size(420.0, 400.0)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("ouverture des réglages : {e}");
    }
}

#[derive(Serialize)]
pub struct SettingsView {
    llm: LlmConfig,
    has_api_key: bool,
    /// Modèle local : la clé est facultative.
    key_optional: bool,
    key_host: String,
    usage: Summary,
}

#[tauri::command]
pub fn settings_view(
    config: State<'_, Arc<Config>>,
    ledger: State<'_, LedgerStore>,
) -> SettingsView {
    SettingsView {
        llm: config.llm.clone(),
        has_api_key: vtt_platform::load_secret(&secret_name(&config.llm)).is_some(),
        key_optional: config.llm.is_local(),
        key_host: config.llm.host().unwrap_or_default().to_owned(),
        usage: costs::summary(&ledger),
    }
}

#[tauri::command]
pub fn save_api_key(config: State<'_, Arc<Config>>, key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("clé vide".into());
    }
    vtt_platform::store_secret(&secret_name(&config.llm), key)?;
    log::info!(
        "clé API enregistrée pour {}",
        config.llm.host().unwrap_or("?")
    );
    Ok(())
}

#[tauri::command]
pub fn delete_api_key(config: State<'_, Arc<Config>>) -> Result<(), String> {
    vtt_platform::delete_secret(&secret_name(&config.llm))?;
    log::info!(
        "clé API supprimée pour {}",
        config.llm.host().unwrap_or("?")
    );
    Ok(())
}

/// Enregistre le fournisseur dans config.toml puis redémarre l'application
/// pour l'appliquer.
#[tauri::command]
pub fn save_llm_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    llm: LlmConfig,
) -> Result<(), String> {
    llm.validate()?;
    if let Some(field) = RESERVED_FIELDS
        .iter()
        .find(|f| llm.extra_body.contains_key(**f))
    {
        return Err(format!(
            "paramètres supplémentaires : « {field} » est géré par l'application"
        ));
    }
    // Repartir du fichier sur disque pour garder les autres sections ; s'il
    // est invalide, ne pas l'écraser avec les valeurs par défaut.
    let (mut config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    config.llm = llm;
    paths.write_config(&config)?;
    log::info!(
        "fournisseur LLM : {} via {}, redémarrage",
        config.llm.model,
        config.llm.base_url
    );
    // Laisser la réponse revenir à la fenêtre avant de redémarrer.
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        app.restart();
    });
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
    let cleaner = LlmCleaner::new(
        &config.llm,
        &config.dictionary,
        api_key_provider(&config.llm),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn llm(url: &str) -> LlmConfig {
        LlmConfig {
            base_url: url.into(),
            ..LlmConfig::default()
        }
    }

    #[test]
    fn deepseek_keeps_legacy_secret_others_get_their_own() {
        assert_eq!(secret_name(&llm("https://api.deepseek.com")), "llm-api-key");
        assert_eq!(
            secret_name(&llm("https://api.z.ai/api/paas/v4")),
            "llm-api-key/api.z.ai"
        );
        assert_eq!(
            secret_name(&llm("http://localhost:11434/v1")),
            "llm-api-key/localhost"
        );
    }
}
