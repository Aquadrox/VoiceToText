//! Export et import des réglages : configuration, prompts, profils LLM,
//! dictionnaire, snippets et styles, dans un fichier TOML. Jamais les clés
//! API : elles restent dans le Gestionnaire d'identifiants de ce PC.

use std::fs;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use vtt_core::Config;

use crate::app_settings;
use crate::paths::Paths;
use crate::settings;

/// Un fichier de réglages fait quelques Ko : au-delà, ce n'en est pas un.
const MAX_IMPORT_BYTES: u64 = 1024 * 1024;
/// Copie des réglages remplacés par un import, à côté de config.toml.
const BACKUP_FILE: &str = "config.avant-import.toml";
const FILTER: &str = "Réglages VoiceToText";

fn header() -> String {
    format!(
        "# Réglages VoiceToText {} exportés le {}.\n\
         # Sans les clés API : à saisir de nouveau après l'import (Réglages → Nettoyage par LLM → Clé API).\n\n",
        env!("CARGO_PKG_VERSION"),
        crate::costs::today()
    )
}

/// Réglages actuels tels qu'écrits dans config.toml ; refuse s'il est
/// invalide (l'export contiendrait les valeurs par défaut).
fn current(paths: &Paths) -> Result<Config, String> {
    match paths.load_config() {
        (_, Some(error)) => Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        )),
        (config, None) => Ok(config),
    }
}

#[derive(Serialize)]
pub struct Exported {
    path: String,
    /// Champs de `extra_body` retirés parce qu'ils ressemblent à un secret.
    removed: Vec<String>,
}

/// Enregistre les réglages dans le fichier choisi (`None` : annulé).
#[tauri::command(async)]
pub fn export_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
) -> Result<Option<Exported>, String> {
    let (config, removed) = current(&paths)?.for_export();
    let toml = config
        .to_toml()
        .map_err(|e| format!("réglages non exportables : {e}"))?;
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Exporter les réglages")
        .add_filter(FILTER, &["toml"])
        .set_file_name(format!(
            "VoiceToText-reglages-{}.toml",
            crate::costs::today()
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    let Some(path) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, format!("{}{toml}", header()))
        .map_err(|e| format!("écriture de {} : {e}", path.display()))?;
    log::info!("réglages exportés : {}", path.display());
    if !removed.is_empty() {
        log::warn!("champs retirés de l'export : {}", removed.join(", "));
    }
    Ok(Some(Exported {
        path: path.display().to_string(),
        removed,
    }))
}

/// Vérifie un fichier importé comme si chaque réglage avait été saisi dans
/// l'interface.
fn parse(source: &str, paths: &Paths) -> Result<Config, String> {
    let mut config =
        Config::from_toml(source).map_err(|e| format!("fichier de réglages invalide : {e}"))?;
    config.migrate();
    let mut config = config.checked()?;
    app_settings::validate_shortcuts(&config.hotkey)?;
    app_settings::validate_captures(&config.captures)?;
    app_settings::validate_model(paths, &config.asr.model)?;
    config.translation.language = config.translation.language.trim().to_owned();
    settings::check_llm(&mut config.llm).map_err(|e| format!("nettoyage : {e}"))?;
    for profile in &mut config.llm_profiles {
        settings::check_llm(&mut profile.llm)
            .map_err(|e| format!("profil « {} » : {e}", profile.name))?;
    }
    Ok(config)
}

#[derive(Serialize)]
pub struct Imported {
    backup: Option<String>,
    notes: Vec<String>,
}

/// Remplace les réglages par ceux du fichier choisi (`None` : annulé) et
/// les applique sans redémarrer. Les réglages remplacés sont gardés dans
/// `config.avant-import.toml`.
#[tauri::command(async)]
pub fn import_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
) -> Result<Option<Imported>, String> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Importer des réglages")
        .add_filter(FILTER, &["toml"]);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    let Some(path) = dialog.blocking_pick_file() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let size = fs::metadata(&path)
        .map_err(|e| format!("lecture de {} : {e}", path.display()))?
        .len();
    if size > MAX_IMPORT_BYTES {
        return Err("fichier trop gros pour des réglages VoiceToText".into());
    }
    let source =
        fs::read_to_string(&path).map_err(|e| format!("lecture de {} : {e}", path.display()))?;
    let config = parse(&source, &paths)?;

    let backup = paths.config_file.with_file_name(BACKUP_FILE);
    let backup = match fs::copy(&paths.config_file, &backup) {
        Ok(_) => Some(backup.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("sauvegarde des réglages actuels : {e}")),
    };
    paths.write_config(&config)?;
    let notes = app_settings::apply(&app, &paths, &config);
    log::info!("réglages importés : {}", path.display());
    Ok(Some(Imported { backup, notes }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_file_imports_back() {
        let dir = std::env::temp_dir().join(format!("vtt-export-{}", std::process::id()));
        let paths = Paths {
            config_file: dir.join("config.toml"),
            models_dir: dir.join("models"),
            logs_dir: dir.join("logs"),
            history_file: dir.join("history.jsonl"),
            usage_file: dir.join("usage.json"),
            version_file: dir.join("version.json"),
            meetings_dir: dir.join("meetings"),
            captures_dir: dir.join("captures"),
        };
        let mut config = Config::default();
        config.dictionary.push("Aquadrox".into());
        let toml = config.for_export().0.to_toml().unwrap();
        let imported = parse(&format!("{}{toml}", header()), &paths).unwrap();
        assert_eq!(imported, config);
        assert!(parse("[hotkey]\nretry = \"pas un raccourci\"\n", &paths).is_err());
        assert!(parse("[asr]\nmodel = \"inconnu.bin\"\n", &paths).is_err());
        assert!(parse("pas du toml = = =", &paths).is_err());
    }
}
