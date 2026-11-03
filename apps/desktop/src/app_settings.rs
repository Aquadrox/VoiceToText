//! Réglages généraux : raccourcis, micro, modèle Whisper, mises à jour,
//! démarrage avec Windows.

use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use vtt_core::config::{AsrConfig, AudioConfig, HOTKEY_NAMES, HotkeyConfig, UpdatesConfig};

use crate::model::{self, ModelChoice};
use crate::paths::Paths;
use crate::retry;

#[derive(Serialize)]
pub struct AppSettingsView {
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
    updates: UpdatesConfig,
    keys: &'static [&'static str],
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
        hotkey: config.hotkey,
        audio: config.audio,
        asr: config.asr,
        updates: config.updates,
        keys: HOTKEY_NAMES,
        microphones: vtt_platform::audio::input_devices(),
        models: model::catalog(&paths.models_dir),
        autostart: vtt_platform::autostart_enabled(),
    })
}

/// Enregistre raccourcis, micro et modèle dans config.toml puis redémarre
/// l'application (le hook clavier, le micro et le modèle sont pris au
/// démarrage). Un nouveau modèle est téléchargé au redémarrage.
#[tauri::command]
pub fn save_app_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
    updates: UpdatesConfig,
) -> Result<(), String> {
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
    // La langue n'est pas encore réglable : seuls le modèle et le GPU changent.
    config.asr.model = asr.model;
    config.asr.use_gpu = asr.use_gpu;
    config.updates = updates;
    paths.write_config(&config)?;
    log::info!("réglages généraux enregistrés, redémarrage");
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        app.restart();
    });
    Ok(())
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    vtt_platform::set_autostart(enabled)?;
    log::info!("démarrage avec Windows : {enabled}");
    Ok(())
}
