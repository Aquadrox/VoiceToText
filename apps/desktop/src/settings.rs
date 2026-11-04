//! Page Réglages de la fenêtre principale : fournisseur LLM (appliqué à
//! chaud), profils LLM, clé API, test avec diagnostic, consommation.

use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::Instant;

use serde::Serialize;
use tauri::State;
use vtt_core::config::{self, LlmConfig, LlmProfile, PromptKind};
use vtt_core::usage::{Summary, Usage};
use vtt_core::{Config, PostProcessor, WithFallback};
use vtt_llm::{ApiKeyProvider, LlmCleaner, Prompts};

use crate::controller::{self, WorkerHandle};
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;

/// Configuration en cours, modifiable depuis l'interface sans redémarrer.
pub type SharedConfig = Arc<RwLock<Config>>;

pub fn read(config: &SharedConfig) -> RwLockReadGuard<'_, Config> {
    config.read().unwrap_or_else(|e| e.into_inner())
}

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

/// Prompts choisis pour `llm` (`None` : prompt intégré).
pub fn prompts_for(config: &Config, llm: &LlmConfig) -> Prompts {
    let text = |kind, name: &str| config.prompt_text(kind, name).map(str::to_owned);
    Prompts {
        cleanup: text(PromptKind::Nettoyage, &llm.cleanup_prompt),
        command: text(PromptKind::Commande, &llm.command_prompt),
    }
}

fn cleaner_for(config: &Config, llm: &LlmConfig) -> Box<dyn PostProcessor> {
    Box::new(LlmCleaner::new(
        llm,
        &config.dictionary,
        &prompts_for(config, llm),
        api_key_provider(llm),
    ))
}

/// Nettoyage construit d'après la configuration (`None` : désactivé), doublé
/// du profil de secours s'il y en a un.
pub fn build_cleaner(config: &Config) -> Option<Box<dyn PostProcessor>> {
    if !config.llm.enabled {
        return None;
    }
    let primary = cleaner_for(config, &config.llm);
    let fallback_name = config.llm.fallback_profile.trim();
    if fallback_name.is_empty() {
        return Some(primary);
    }
    match config.fallback_for(&config.llm) {
        Some(profile) => {
            log::info!(
                "profil de secours : « {} » ({} via {})",
                profile.name,
                profile.llm.model,
                profile.llm.base_url
            );
            Some(Box::new(WithFallback::new(
                primary,
                cleaner_for(config, &profile.llm),
                profile.name.clone(),
            )))
        }
        None => {
            log::warn!("profil de secours « {fallback_name} » inutilisable : ignoré");
            Some(primary)
        }
    }
}

/// Adresse normalisée, configuration valide, champs réservés protégés.
fn check_llm(llm: &mut LlmConfig) -> Result<(), String> {
    llm.base_url = llm.normalized_base_url();
    llm.validate()?;
    if let Some(field) = RESERVED_FIELDS
        .iter()
        .find(|f| llm.extra_body.contains_key(**f))
    {
        return Err(format!(
            "paramètres supplémentaires : « {field} » est géré par l'application"
        ));
    }
    Ok(())
}

/// Relit config.toml (pour garder les autres sections) ; refuse s'il est
/// invalide, pour ne pas l'écraser avec les valeurs par défaut.
fn load_for_update(paths: &Paths) -> Result<Config, String> {
    let (config, error) = paths.load_config();
    match error {
        Some(error) => Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        )),
        None => Ok(config),
    }
}

#[derive(Serialize)]
pub struct SettingsView {
    llm: LlmConfig,
    profiles: Vec<LlmProfile>,
    usage: Summary,
}

#[tauri::command]
pub fn settings_view(
    config: State<'_, SharedConfig>,
    ledger: State<'_, LedgerStore>,
) -> SettingsView {
    let config = read(&config);
    SettingsView {
        llm: config.llm.clone(),
        profiles: config.llm_profiles.clone(),
        usage: costs::summary(&ledger),
    }
}

/// Adresse saisie dans le formulaire (peut différer de celle appliquée) :
/// la clé est rangée par adresse.
fn llm_for(base_url: &str) -> LlmConfig {
    let mut llm = LlmConfig {
        base_url: base_url.to_owned(),
        ..LlmConfig::default()
    };
    llm.base_url = llm.normalized_base_url();
    llm
}

#[derive(Serialize)]
pub struct KeyStatus {
    host: String,
    has_key: bool,
    /// Serveur local : clé facultative.
    optional: bool,
    /// Adresse HTTP hors de ce PC : une clé ne serait pas envoyée.
    cleartext: bool,
}

#[tauri::command]
pub fn key_status(base_url: String) -> KeyStatus {
    let llm = llm_for(&base_url);
    KeyStatus {
        host: llm.host().unwrap_or_default().to_owned(),
        has_key: llm.host().is_some() && vtt_platform::load_secret(&secret_name(&llm)).is_some(),
        optional: llm.is_local(),
        cleartext: llm.host().is_some() && !llm.may_send_key(),
    }
}

#[tauri::command]
pub fn save_api_key(base_url: String, key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("clé vide".into());
    }
    let llm = llm_for(&base_url);
    let host = llm.host().ok_or("adresse de l'API invalide")?;
    vtt_platform::store_secret(&secret_name(&llm), key)?;
    log::info!("clé API enregistrée pour {host}");
    Ok(())
}

#[tauri::command]
pub fn delete_api_key(base_url: String) -> Result<(), String> {
    let llm = llm_for(&base_url);
    vtt_platform::delete_secret(&secret_name(&llm))?;
    log::info!("clé API supprimée pour {}", llm.host().unwrap_or("?"));
    Ok(())
}

#[derive(Serialize)]
pub struct Applied {
    /// `false` : modèle encore en chargement, appliqué au prochain démarrage.
    applied: bool,
    /// Configuration enregistrée (adresse normalisée).
    llm: LlmConfig,
}

/// Enregistre le fournisseur dans config.toml et l'applique tout de suite,
/// sans redémarrer.
#[tauri::command]
pub fn apply_llm_settings(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    worker: State<'_, WorkerHandle>,
    mut llm: LlmConfig,
) -> Result<Applied, String> {
    check_llm(&mut llm)?;
    let mut updated = load_for_update(&paths)?;
    updated.check_prompt_refs(&llm)?;
    updated.check_fallback_ref(&llm)?;
    updated.llm = llm.clone();
    paths.write_config(&updated)?;
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated.clone();
    let applied = controller::reconfigure(&worker, &updated);
    log::info!(
        "fournisseur LLM appliqué : {} via {}",
        llm.model,
        llm.base_url
    );
    Ok(Applied { applied, llm })
}

/// Remplace la liste des profils LLM enregistrés (le profil de secours en
/// fait partie : le nettoyage est reconstruit).
#[tauri::command]
pub fn save_llm_profiles(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    worker: State<'_, WorkerHandle>,
    profiles: Vec<LlmProfile>,
) -> Result<Vec<LlmProfile>, String> {
    let profiles = config::clean_llm_profiles(&profiles)?;
    for profile in &profiles {
        let mut llm = profile.llm.clone();
        check_llm(&mut llm).map_err(|e| format!("profil « {} » : {e}", profile.name))?;
    }
    let mut updated = load_for_update(&paths)?;
    updated.llm_profiles = profiles.clone();
    paths.write_config(&updated)?;
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated.clone();
    controller::reconfigure(&worker, &updated);
    log::info!("{} profil(s) LLM enregistré(s)", profiles.len());
    Ok(profiles)
}

/// Compte rendu du test, affiché tel quel dans les réglages.
#[derive(Serialize)]
pub struct LlmDiagnostic {
    ok: bool,
    /// Adresse réellement appelée.
    endpoint: String,
    /// Adresse corrigée si celle saisie a été modifiée.
    corrected_url: Option<String>,
    model: String,
    /// « envoyée », « aucune », « non envoyée (HTTP hors de ce PC) ».
    key: &'static str,
    duration_ms: u128,
    input: &'static str,
    output: Option<String>,
    error: Option<String>,
    usage: Option<Usage>,
    cost: Option<f64>,
    currency: String,
    /// Modèles annoncés par le serveur (`GET …/models`).
    models: Vec<String>,
    models_error: Option<String>,
    hints: Vec<String>,
}

/// Teste les réglages saisis (sans les enregistrer) : nettoyage d'une phrase
/// d'exemple et liste des modèles du serveur, avec des pistes en cas
/// d'erreur. `key` : clé saisie mais pas encore enregistrée (sinon, celle
/// enregistrée pour cette adresse). Le test est facturé et compté.
#[tauri::command(async)]
pub fn test_llm_settings(
    config: State<'_, SharedConfig>,
    ledger: State<'_, LedgerStore>,
    mut llm: LlmConfig,
    key: Option<String>,
) -> Result<LlmDiagnostic, String> {
    let typed = llm.base_url.trim().trim_end_matches('/').to_owned();
    check_llm(&mut llm)?;
    let corrected_url = (llm.base_url != typed).then(|| llm.base_url.clone());
    let key = key
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| vtt_platform::load_secret(&secret_name(&llm)));
    let (dictionary, prompts) = {
        let config = read(&config);
        (config.dictionary.clone(), prompts_for(&config, &llm))
    };
    let provided = key.clone();
    let cleaner = LlmCleaner::new(
        &llm,
        &dictionary,
        &prompts,
        Box::new(move || provided.clone()),
    );
    let key_status = match (&key, llm.may_send_key()) {
        (None, _) => "aucune",
        (Some(_), true) => "envoyée",
        (Some(_), false) => "non envoyée (HTTP hors de ce PC : jamais en clair sur le réseau)",
    };

    let started = Instant::now();
    let result = cleaner.process(SAMPLE, None);
    let duration_ms = started.elapsed().as_millis();
    let (models, models_error) = match vtt_llm::list_models(&llm, key) {
        Ok(models) => (models, None),
        Err(e) => (Vec::new(), Some(e)),
    };

    let mut diagnostic = LlmDiagnostic {
        ok: result.is_ok(),
        endpoint: cleaner.endpoint().to_owned(),
        corrected_url,
        model: llm.model.clone(),
        key: key_status,
        duration_ms,
        input: SAMPLE,
        output: None,
        error: None,
        usage: None,
        cost: None,
        currency: llm.pricing.currency.clone(),
        models,
        models_error,
        hints: Vec::new(),
    };
    match result {
        Ok(cleaned) => {
            if let Some(usage) = &cleaned.usage {
                costs::record(&ledger, usage, &llm.pricing);
            }
            diagnostic.cost = cleaned.usage.map(|u| llm.pricing.cost(&u));
            diagnostic.usage = cleaned.usage;
            diagnostic.output = Some(cleaned.text);
        }
        Err(e) => {
            let error = e.to_string();
            diagnostic.hints = vtt_llm::hints(&llm, &error);
            diagnostic.error = Some(error);
        }
    }
    if !diagnostic.models.is_empty() && !diagnostic.models.iter().any(|m| m == &llm.model) {
        diagnostic.hints.push(format!(
            "Le modèle « {} » n'est pas dans la liste annoncée par le serveur : vérifiez son nom \
             exact.",
            llm.model
        ));
    }
    log::info!(
        "test LLM {} : {} ({duration_ms} ms)",
        diagnostic.endpoint,
        if diagnostic.ok { "réussi" } else { "échec" }
    );
    Ok(diagnostic)
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

    #[test]
    fn check_llm_normalizes_and_protects_fields() {
        let mut l = llm("https://api.openai.com/v1/chat/completions/");
        check_llm(&mut l).unwrap();
        assert_eq!(l.base_url, "https://api.openai.com/v1");
        l.extra_body
            .insert("messages".into(), serde_json::json!([]));
        assert!(check_llm(&mut l).is_err());
    }
}
