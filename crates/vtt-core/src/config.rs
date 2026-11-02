//! Configuration utilisateur, stockée en TOML.
//!
//! Chaque section a des valeurs par défaut : un fichier vide ou partiel est
//! valide, les clés absentes prennent leur valeur par défaut.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub asr: AsrConfig,
    pub llm: LlmConfig,
    /// Mots et noms propres à bien orthographier (indices pour Whisper et le LLM).
    pub dictionary: Vec<String>,
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Touche de push-to-talk, ex. "AltGr", "RightCtrl", "F13".
    pub key: String,
    /// Appui plus court que ce seuil : ignoré (évite les déclenchements accidentels).
    pub min_hold_ms: u32,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: "AltGr".into(),
            min_hold_ms: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    /// Nom du fichier modèle ggml dans le dossier des modèles.
    pub model: String,
    /// Code langue ISO 639-1 imposé à Whisper.
    pub language: String,
    pub use_gpu: bool,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            model: "ggml-large-v3-turbo-q8_0.bin".into(),
            language: "fr".into(),
            use_gpu: true,
        }
    }
}

/// Nettoyage du texte par un LLM exposant une API compatible OpenAI.
///
/// La clé API n'est pas stockée ici mais dans le gestionnaire d'identifiants de l'OS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    /// Au-delà de ce délai, le texte brut est inséré.
    pub timeout_ms: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            timeout_ms: 3000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    /// Phrase prononcée, ex. "ma signature".
    pub trigger: String,
    /// Texte inséré à la place.
    pub text: String,
}

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let config = Config::from_toml("[hotkey]\nkey = \"F13\"\n").unwrap();
        assert_eq!(config.hotkey.key, "F13");
        assert_eq!(config.hotkey.min_hold_ms, 250);
        assert_eq!(config.llm, LlmConfig::default());
    }

    #[test]
    fn round_trip() {
        let mut config = Config::default();
        config.dictionary.push("Aquadrox".into());
        config.snippets.push(Snippet {
            trigger: "ma signature".into(),
            text: "Florian\nVoiceToText".into(),
        });
        let parsed = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn unknown_key_is_ignored() {
        assert!(Config::from_toml("obsolete = 1\n").is_ok());
    }
}
