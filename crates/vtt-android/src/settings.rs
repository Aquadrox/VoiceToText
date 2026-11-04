//! Réglages du téléphone : le même config.toml que sur PC (importable tel
//! quel), dont l'application Kotlin ne modifie que quelques champs.

use std::path::Path;

use serde::{Deserialize, Serialize};
use vtt_core::Config;
use vtt_core::config::{self, Snippet, TRANSLATION_LANGUAGES, TranslationConfig};

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
    translation: &'a str,
    languages: &'static [&'static str],
    history: bool,
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
        translation: &config.translation.language,
        languages: TRANSLATION_LANGUAGES,
        history: config.history.enabled,
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
    pub translation: String,
    pub history: bool,
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
    let translation = TranslationConfig {
        language: update.translation.trim().to_owned(),
    };
    translation.validate()?;
    config.asr.model = update.model.clone();
    config.dictation.voice_commands = update.voice_commands;
    config.translation = translation;
    config.history.enabled = update.history;
    to_toml(&config)
}

/// Dictionnaire et snippets, modifiables sur le téléphone.
#[derive(Debug, Serialize, Deserialize)]
pub struct Vocabulary {
    pub dictionary: Vec<String>,
    pub snippets: Vec<Snippet>,
}

pub fn vocabulary(toml: &str) -> Result<Vocabulary, String> {
    let (config, _) = parse(toml)?;
    Ok(Vocabulary {
        dictionary: config.dictionary,
        snippets: config.snippets,
    })
}

/// Enregistre dictionnaire et snippets (vérifiés comme sur PC).
pub fn save_vocabulary(toml: &str, vocabulary: &Vocabulary) -> Result<String, String> {
    let (mut config, _) = parse(toml)?;
    config.dictionary = config::clean_dictionary(&vocabulary.dictionary)?;
    config.snippets = config::clean_snippets(&vocabulary.snippets)?;
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
            translation: "anglais".into(),
            history: true,
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

    #[test]
    fn vocabulary_and_translation_are_validated() {
        let saved = save_vocabulary(
            "",
            &Vocabulary {
                dictionary: vec![" Aquadrox ".into(), "Kubernetes".into()],
                snippets: vec![Snippet {
                    trigger: "ma signature".into(),
                    text: "Florian".into(),
                }],
            },
        )
        .unwrap();
        let back = vocabulary(&saved).unwrap();
        assert_eq!(back.dictionary, ["Aquadrox", "Kubernetes"]);
        assert_eq!(back.snippets.len(), 1);
        let empty_trigger = Vocabulary {
            dictionary: Vec::new(),
            snippets: vec![Snippet {
                trigger: " ".into(),
                text: "x".into(),
            }],
        };
        assert!(save_vocabulary("", &empty_trigger).is_err());
        let mut no_language = update_with(models::DEFAULT_MODEL, "https://api.z.ai");
        no_language.translation = " ".into();
        assert!(update("", &no_language).is_err());
    }
}
