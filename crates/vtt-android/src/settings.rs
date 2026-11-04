//! Réglages du téléphone : le même config.toml que sur PC (importable tel
//! quel), dont l'application Kotlin ne modifie que quelques champs.

use std::path::Path;

use serde::{Deserialize, Serialize};
use vtt_core::Config;

use crate::models;

/// Configuration du téléphone ; un modèle de PC introuvable ici est remplacé
/// par le modèle par défaut du téléphone.
pub fn parse(toml: &str) -> Result<(Config, Vec<String>), String> {
    let mut config = if toml.trim().is_empty() {
        default_config()
    } else {
        Config::from_toml(toml).map_err(|e| format!("réglages illisibles : {e}"))?
    };
    config.migrate();
    let mut notes = Vec::new();
    if models::known(&config.asr.model).is_none() {
        notes.push(format!(
            "Modèle « {} » non proposé sur le téléphone : {} à la place.",
            config.asr.model,
            models::DEFAULT_MODEL
        ));
        config.asr.model = models::DEFAULT_MODEL.into();
    }
    Ok((config, notes))
}

pub fn default_config() -> Config {
    let mut config = Config::default();
    config.asr.model = models::DEFAULT_MODEL.into();
    config.asr.use_gpu = false;
    config
}

pub fn to_toml(config: &Config) -> Result<String, String> {
    config
        .to_toml()
        .map_err(|e| format!("réglages non enregistrables : {e}"))
}

#[derive(Serialize)]
struct LlmView<'a> {
    enabled: bool,
    base_url: &'a str,
    model: &'a str,
    /// Hôte de l'API : la clé est rangée par hôte, comme sur PC.
    host: &'a str,
}

#[derive(Serialize)]
struct View<'a> {
    llm: LlmView<'a>,
    model: &'a str,
    models: Vec<models::ModelChoice>,
    dictionary: usize,
    snippets: usize,
    voice_commands: bool,
}

/// Réglages affichés par l'application (JSON).
pub fn view(toml: &str, models_dir: &Path) -> Result<String, String> {
    let (config, _) = parse(toml)?;
    let view = View {
        llm: LlmView {
            enabled: config.llm.enabled,
            base_url: &config.llm.base_url,
            model: &config.llm.model,
            host: config.llm.host().unwrap_or_default(),
        },
        model: &config.asr.model,
        models: models::catalog(models_dir),
        dictionary: config.dictionary.len(),
        snippets: config.snippets.len(),
        voice_commands: config.dictation.voice_commands,
    };
    serde_json::to_string(&view).map_err(|e| e.to_string())
}

/// Champs modifiables sur le téléphone.
#[derive(Debug, Deserialize)]
pub struct Update {
    pub llm_enabled: bool,
    pub base_url: String,
    pub llm_model: String,
    pub model: String,
    pub voice_commands: bool,
}

/// Applique `update` à la configuration ; renvoie le nouveau config.toml.
pub fn update(toml: &str, update: &Update) -> Result<String, String> {
    let (mut config, _) = parse(toml)?;
    if models::known(&update.model).is_none() {
        return Err(format!("modèle inconnu : {}", update.model));
    }
    let mut llm = config.llm.clone();
    llm.enabled = update.llm_enabled;
    llm.base_url = update.base_url.trim().to_owned();
    llm.base_url = llm.normalized_base_url();
    llm.model = update.llm_model.trim().to_owned();
    if llm.enabled {
        llm.validate()?;
        if llm.model.is_empty() {
            return Err("nom du modèle LLM vide".into());
        }
    }
    config.llm = llm;
    config.asr.model = update.model.clone();
    config.dictation.voice_commands = update.voice_commands;
    to_toml(&config)
}

#[derive(Serialize)]
pub struct Imported {
    pub toml: String,
    pub notes: Vec<String>,
}

/// Fichier exporté depuis VoiceToText sur PC : vérifié comme sur PC, puis
/// adapté au téléphone.
pub fn import(text: &str) -> Result<Imported, String> {
    let config =
        Config::from_toml(text).map_err(|e| format!("fichier de réglages invalide : {e}"))?;
    config.checked()?;
    let (config, notes) = parse(text)?;
    Ok(Imported {
        toml: to_toml(&config)?,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_with(model: &str, base_url: &str) -> Update {
        Update {
            llm_enabled: true,
            base_url: base_url.into(),
            llm_model: "deepseek-flash".into(),
            model: model.into(),
            voice_commands: true,
        }
    }

    #[test]
    fn empty_settings_use_the_phone_model() {
        let (config, notes) = parse("").unwrap();
        assert_eq!(config.asr.model, models::DEFAULT_MODEL);
        assert!(notes.is_empty());
        let json = view("", Path::new("/nulle-part")).unwrap();
        assert!(json.contains("\"host\":\"api.deepseek.com\""));
    }

    #[test]
    fn pc_settings_are_imported_and_adapted() {
        let mut pc = Config::default();
        pc.dictionary.push("Aquadrox".into());
        pc.asr.model = "parakeet-tdt-0.6b-v3-int8".into();
        let imported = import(&pc.to_toml().unwrap()).unwrap();
        assert_eq!(imported.notes.len(), 1, "modèle de PC remplacé");
        let (config, _) = parse(&imported.toml).unwrap();
        assert_eq!(config.dictionary, ["Aquadrox"]);
        assert_eq!(config.asr.model, models::DEFAULT_MODEL);
        assert!(import("pas du toml = =").is_err());
    }

    #[test]
    fn updates_are_validated() {
        let toml = update(
            "",
            &update_with("ggml-base-q8_0.bin", "https://api.z.ai/api/paas/v4/"),
        )
        .unwrap();
        let (config, _) = parse(&toml).unwrap();
        assert_eq!(config.asr.model, "ggml-base-q8_0.bin");
        assert_eq!(config.llm.base_url, "https://api.z.ai/api/paas/v4");
        assert!(update("", &update_with("inconnu.bin", "https://api.z.ai")).is_err());
        assert!(
            update(
                "",
                &update_with(models::DEFAULT_MODEL, "http://exemple.com")
            )
            .is_err()
        );
    }
}
