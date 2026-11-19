//! Export et import des réglages : configuration, prompts, profils LLM,
//! dictionnaire, snippets et styles, dans un fichier TOML. Jamais les clés
//! API : elles restent dans le Gestionnaire d'identifiants de ce PC.

use std::fs;
use std::path::Path;

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

/// Sérialise les réglages actuels prêts à être exportés (avec en-tête et sans
/// champs ressemblant à un secret).
fn export_toml(paths: &Paths) -> Result<(String, Vec<String>), String> {
    let (config, removed) = current(paths)?.for_export();
    let toml = config
        .to_toml()
        .map_err(|e| format!("réglages non exportables : {e}"))?;
    Ok((format!("{}{toml}", header()), removed))
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
    let (content, removed) = export_toml(&paths)?;
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
    fs::write(&path, content).map_err(|e| format!("écriture de {} : {e}", path.display()))?;
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

fn read_and_parse_import(path: &Path, paths: &Paths) -> Result<Config, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("lecture de {} : {e}", path.display()))?
        .len();
    if size > MAX_IMPORT_BYTES {
        return Err("fichier trop gros pour des réglages VoiceToText".into());
    }
    let source =
        fs::read_to_string(path).map_err(|e| format!("lecture de {} : {e}", path.display()))?;
    parse(&source, paths)
}

fn backup_existing_config(paths: &Paths) -> Result<Option<String>, String> {
    let backup = paths.config_file.with_file_name(BACKUP_FILE);
    match fs::copy(&paths.config_file, &backup) {
        Ok(_) => Ok(Some(backup.display().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("sauvegarde des réglages actuels : {e}")),
    }
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
    let config = read_and_parse_import(&path, &paths)?;
    let backup = backup_existing_config(&paths)?;
    paths.write_config(&config)?;
    let notes = app_settings::apply(&app, &paths, &config);
    log::info!("réglages importés : {}", path.display());
    Ok(Some(Imported { backup, notes }))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;
    use vtt_core::config::LlmProfile;

    use super::*;

    fn temp_paths(name: &str) -> (PathBuf, Paths) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-export-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let paths = Paths::for_tests(&dir);
        (dir, paths)
    }

    #[test]
    fn exported_file_imports_back() {
        let (dir, paths) = temp_paths("roundtrip");
        let mut config = Config::default();
        config.dictionary.push("Aquadrox".into());
        let toml = config.for_export().0.to_toml().unwrap();
        let imported = parse(&format!("{}{toml}", header()), &paths).unwrap();
        assert_eq!(imported, config);
        assert!(parse("[hotkey]\nretry = \"pas un raccourci\"\n", &paths).is_err());
        assert!(parse("[asr]\nmodel = \"inconnu.bin\"\n", &paths).is_err());
        assert!(parse("pas du toml = = =", &paths).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_toml_strips_secret_like_extra_body_and_rejects_invalid_config() {
        let (dir, paths) = temp_paths("strip");
        let mut config = Config::default();
        config
            .llm
            .extra_body
            .insert("api_key".into(), json!("sk-top-secret"));
        config
            .llm
            .extra_body
            .insert("temperature".into(), json!(0.2));
        paths.write_config(&config).unwrap();

        let (exported, removed) = export_toml(&paths).unwrap();
        assert!(exported.starts_with("# Réglages VoiceToText"));
        assert!(!exported.contains("sk-top-secret"));
        assert_eq!(removed, vec!["llm.extra_body.api_key"]);

        let reimported = parse(&exported, &paths).unwrap();
        assert!(!reimported.llm.extra_body.contains_key("api_key"));
        assert_eq!(
            reimported.llm.extra_body.get("temperature"),
            Some(&json!(0.2))
        );

        // Un config.toml invalide bloque l'export au lieu d'exporter les valeurs par défaut.
        fs::write(&paths.config_file, "toml invalide = = =").unwrap();
        let err = export_toml(&paths).unwrap_err();
        assert!(err.contains("corrigez-le d'abord"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_and_parse_import_checks_size_limit_and_parses_valid_file() {
        let (dir, paths) = temp_paths("import");
        let valid_file = dir.join("valid.toml");
        let mut config = Config::default();
        config.translation.language = "  anglais  ".into();
        fs::write(&valid_file, config.to_toml().unwrap()).unwrap();

        let imported = read_and_parse_import(&valid_file, &paths).unwrap();
        assert_eq!(imported.translation.language, "anglais");

        let huge_file = dir.join("huge.toml");
        fs::write(&huge_file, vec![b'a'; (MAX_IMPORT_BYTES + 1) as usize]).unwrap();
        let err = read_and_parse_import(&huge_file, &paths).unwrap_err();
        assert!(err.contains("trop gros"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_rejects_reserved_extra_body_in_llm_and_profiles() {
        let (dir, paths) = temp_paths("reserved");

        let mut bad_main = Config::default();
        bad_main.llm.extra_body.insert("messages".into(), json!([]));
        let err = parse(&bad_main.to_toml().unwrap(), &paths).unwrap_err();
        assert!(err.starts_with("nettoyage :"));

        let mut bad_profile = Config::default();
        let mut profile_llm = Config::default().llm;
        profile_llm.extra_body.insert("stream".into(), json!(true));
        bad_profile.llm_profiles.push(LlmProfile {
            name: "Secours".into(),
            llm: profile_llm,
        });
        let err = parse(&bad_profile.to_toml().unwrap(), &paths).unwrap_err();
        assert!(err.starts_with("profil « Secours » :"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backup_existing_config_copies_when_present_and_returns_none_when_missing() {
        let (dir, paths) = temp_paths("backup");
        assert_eq!(backup_existing_config(&paths).unwrap(), None);

        fs::write(&paths.config_file, "[audio]\ndevice = \"Micro\"\n").unwrap();
        let backup_path = backup_existing_config(&paths).unwrap().unwrap();
        assert!(backup_path.ends_with(BACKUP_FILE));
        assert_eq!(
            fs::read_to_string(&backup_path).unwrap(),
            "[audio]\ndevice = \"Micro\"\n"
        );

        let _ = fs::remove_dir_all(dir);
    }
}
