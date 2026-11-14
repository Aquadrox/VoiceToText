//! Réglages généraux : raccourcis, micro, modèle Whisper, mises à jour,
//! démarrage avec Windows. Tout s'applique sans redémarrer.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use vtt_core::Config;
use vtt_core::config::{
    AsrConfig, AudioConfig, CAPTURE_AREAS, CAPTURE_KEEP_DAYS, CAPTURE_RAW_TEMPLATE, CapturesConfig,
    DictationConfig, HOTKEY_NAMES, HotkeyConfig, TRANSLATION_LANGUAGES, TranslationConfig,
    UNLOAD_AFTER_CHOICES, UpdatesConfig,
};

use crate::controller::ControllerState;
use crate::history::{self, HistoryStore};
use crate::model::{self, ModelChoice};
use crate::paths::Paths;
use crate::retry;
use crate::settings::SharedConfig;
use crate::shortcuts::Shortcuts;
use crate::updates::{self, UpdatesState};

#[derive(Serialize)]
pub struct AppSettingsView {
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
    updates: UpdatesConfig,
    dictation: DictationConfig,
    translation: TranslationConfig,
    captures: CapturesConfig,
    /// Modèles de texte de la capture commentée : (identifiant, libellé).
    capture_templates: Vec<(&'static str, &'static str)>,
    capture_keep_days: &'static [u32],
    /// Profils LLM, pour la description des images.
    llm_profiles: Vec<String>,
    languages: &'static [&'static str],
    keys: &'static [&'static str],
    unload_choices: Vec<u32>,
    microphones: Vec<String>,
    models: Vec<ModelChoice>,
    autostart: bool,
}

#[tauri::command(async)]
pub fn app_settings_view(paths: State<'_, Paths>) -> Result<AppSettingsView, String> {
    let (config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(error);
    }
    Ok(AppSettingsView {
        // Une valeur saisie à la main dans config.toml reste proposée.
        unload_choices: unload_choices(config.asr.unload_after_min),
        hotkey: config.hotkey,
        audio: config.audio,
        asr: config.asr,
        updates: config.updates,
        dictation: config.dictation,
        translation: config.translation,
        captures: config.captures,
        capture_templates: vtt_llm::CAPTURE_TEMPLATES
            .iter()
            .map(|t| (t.id, t.label))
            .chain([(CAPTURE_RAW_TEMPLATE, "Narration seule (sans LLM)")])
            .collect(),
        capture_keep_days: CAPTURE_KEEP_DAYS,
        llm_profiles: config.llm_profiles.iter().map(|p| p.name.clone()).collect(),
        languages: TRANSLATION_LANGUAGES,
        keys: HOTKEY_NAMES,
        microphones: vtt_platform::audio::input_devices(),
        models: model::catalog(&paths.models_dir),
        autostart: vtt_platform::autostart_enabled(),
    })
}

#[derive(Serialize)]
pub struct Applied {
    /// Remarques à afficher : raccourci indisponible, modèle en téléchargement.
    pub notes: Vec<String>,
}

/// Applique `config`, déjà écrite dans config.toml, sans redémarrer ; renvoie
/// les remarques à afficher.
pub fn apply(app: &AppHandle, paths: &Paths, config: &Config) -> Vec<String> {
    let shared = app.state::<SharedConfig>();
    let previous = std::mem::replace(
        &mut *shared.write().unwrap_or_else(|e| e.into_inner()),
        config.clone(),
    );
    history::reconfigure(
        &app.state::<HistoryStore>(),
        &paths.history_file,
        &previous.history,
        &config.history,
    );
    app.state::<ControllerState>().apply(config);
    let mut notes = app.state::<Shortcuts>().apply(app, &config.hotkey);
    updates::set_auto(app, &app.state::<UpdatesState>(), config.updates.check);
    if model::local(&paths.models_dir, &config.asr.model).is_none() {
        notes.push(
            "Téléchargement du nouveau modèle (progression dans la pastille) : les dictées \
             continuent avec l'ancien en attendant."
                .into(),
        );
    }
    notes
}

/// Raccourcis secondaires valides (la touche de dictée est vérifiée par
/// `HotkeyConfig::validate`).
pub fn validate_shortcuts(hotkey: &HotkeyConfig) -> Result<(), String> {
    retry::validate_shortcut(&hotkey.paste_raw, "coller la version brute", "Alt+Shift+V")?;
    retry::validate_shortcut(&hotkey.retry, "réessayer", "Alt+Shift+R")?;
    retry::validate_shortcut(&hotkey.translate, "mode traduction", "Alt+Shift+T")?;
    retry::validate_shortcut(&hotkey.meeting, "réunion", "Alt+Shift+M")?;
    retry::validate_shortcut(
        &hotkey.meeting_copy,
        "copier les nouvelles répliques",
        "Alt+Shift+N",
    )?;
    retry::validate_shortcut(&hotkey.annotate, "capture commentée", "Alt+Shift+A")
}

/// Modèle de texte connu.
pub fn validate_captures(captures: &CapturesConfig) -> Result<(), String> {
    let known = captures.template == CAPTURE_RAW_TEMPLATE
        || vtt_llm::CAPTURE_TEMPLATES
            .iter()
            .any(|t| t.id == captures.template);
    if !known {
        return Err(format!("modèle de capture inconnu : {}", captures.template));
    }
    if !CAPTURE_AREAS.contains(&captures.area.as_str()) {
        return Err(format!("zone de capture inconnue : {}", captures.area));
    }
    Ok(())
}

/// Modèle Whisper proposé dans les réglages (téléchargé au besoin).
pub fn validate_model(paths: &Paths, model: &str) -> Result<(), String> {
    if model::catalog(&paths.models_dir)
        .iter()
        .any(|m| m.file() == model)
    {
        Ok(())
    } else {
        Err(format!("modèle inconnu : {model}"))
    }
}

/// Enregistre raccourcis, micro, modèle, sons… dans config.toml et les
/// applique tout de suite : le raccourci de dictée et le micro à la fin de la
/// dictée en cours, un nouveau modèle dès qu'il est téléchargé et chargé.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_app_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
    updates: UpdatesConfig,
    dictation: DictationConfig,
    translation: TranslationConfig,
    captures: CapturesConfig,
) -> Result<Applied, String> {
    hotkey.validate()?;
    validate_shortcuts(&hotkey)?;
    translation.validate()?;
    validate_captures(&captures)?;
    validate_model(&paths, &asr.model)?;
    let (mut config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    config.hotkey = hotkey;
    config.audio = audio;
    // La langue n'est pas encore réglable.
    config.asr.model = asr.model;
    config.asr.use_gpu = asr.use_gpu;
    config.asr.unload_after_min = asr.unload_after_min;
    config.asr.unload_in_fullscreen = asr.unload_in_fullscreen;
    config.updates = updates;
    config.dictation = dictation;
    config.translation = TranslationConfig {
        language: translation.language.trim().to_owned(),
    };
    let profile = captures.vision_profile.trim();
    if !profile.is_empty() && !config.llm_profiles.iter().any(|p| p.name == profile) {
        return Err(format!("profil LLM introuvable : « {profile} »"));
    }
    config.captures = captures;
    paths.write_config(&config)?;
    let notes = apply(&app, &paths, &config);
    log::info!("réglages généraux appliqués");
    Ok(Applied { notes })
}

fn unload_choices(current: u32) -> Vec<u32> {
    let mut choices = UNLOAD_AFTER_CHOICES.to_vec();
    if !choices.contains(&current) {
        choices.push(current);
        choices.sort_unstable();
    }
    choices
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    vtt_platform::set_autostart(enabled)?;
    log::info!("démarrage avec la session : {enabled}");
    Ok(())
}
