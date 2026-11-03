//! Réglages généraux : raccourcis, micro, modèle Whisper, démarrage avec
//! Windows.

use std::str::FromStr;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_global_shortcut::Shortcut;
use vtt_core::config::{AsrConfig, AudioConfig, HOTKEY_NAMES, HotkeyConfig};

use crate::model::{self, ModelChoice};
use crate::paths::Paths;

#[derive(Serialize)]
pub struct AppSettingsView {
    hotkey: HotkeyConfig,
    audio: AudioConfig,
    asr: AsrConfig,
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
) -> Result<(), String> {
    hotkey.validate()?;
    let paste_raw = hotkey.paste_raw.trim();
    if !paste_raw.is_empty() && Shortcut::from_str(paste_raw).is_err() {
        return Err(format!(
            "raccourci « coller la version brute » invalide : {paste_raw} (ex. Alt+Shift+V)"
        ));
    }
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
