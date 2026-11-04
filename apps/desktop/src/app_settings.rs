//! Réglages généraux : raccourcis, micro, modèle Whisper, mises à jour,
//! démarrage avec Windows. Tout s'applique sans redémarrer.

use serde::Serialize;
use tauri::{AppHandle, State};
use vtt_core::config::{
    AsrConfig, AudioConfig, DictationConfig, HOTKEY_NAMES, HotkeyConfig, UNLOAD_AFTER_CHOICES,
    UpdatesConfig,
};

use crate::controller::ControllerState;
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
        keys: HOTKEY_NAMES,
        microphones: vtt_platform::audio::input_devices(),
        models: model::catalog(&paths.models_dir),
        autostart: vtt_platform::autostart_enabled(),
    })
}

#[derive(Serialize)]
pub struct Applied {
    /// Remarques à afficher : raccourci indisponible, modèle en téléchargement.
    notes: Vec<String>,
}

/// Enregistre raccourcis, micro, modèle, sons… dans config.toml et les
/// applique tout de suite : le raccourci de dictée et le micro à la fin de la
/// dictée en cours, un nouveau modèle dès qu'il est téléchargé et chargé.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_app_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    shared: State<'_, SharedConfig>,
    controller: State<'_, ControllerState>,
    shortcuts: State<'_, Shortcuts>,
    update_checks: State<'_, UpdatesState>,
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
    updates: UpdatesConfig,
    dictation: DictationConfig,
) -> Result<Applied, String> {
    hotkey.validate()?;
    retry::validate_shortcut(&hotkey.paste_raw, "coller la version brute", "Alt+Shift+V")?;
    retry::validate_shortcut(&hotkey.retry, "réessayer", "Alt+Shift+R")?;
    if !model::catalog(&paths.models_dir)
        .iter()
        .any(|m| m.file() == asr.model)
    {
        return Err(format!("modèle inconnu : {}", asr.model));
    }
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
    paths.write_config(&config)?;
    *shared.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
    controller.apply(&config);
    let mut notes = shortcuts.apply(&app, &config.hotkey);
    updates::set_auto(&app, &update_checks, config.updates.check);
    if model::local(&paths.models_dir, &config.asr.model).is_none() {
        notes.push(
            "Téléchargement du nouveau modèle (progression dans la pastille) : les dictées \
             continuent avec l'ancien en attendant."
                .into(),
        );
    }
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
    log::info!("démarrage avec Windows : {enabled}");
    Ok(())
}
