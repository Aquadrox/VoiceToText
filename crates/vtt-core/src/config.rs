//! Configuration utilisateur, stockée en TOML.
//!
//! Chaque section a des valeurs par défaut : un fichier vide ou partiel est
//! valide, les clés absentes prennent leur valeur par défaut.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub asr: AsrConfig,
    pub llm: LlmConfig,
    pub history: HistoryConfig,
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
    /// Double appui court → dictée mains libres (nouvel appui pour finir, Échap pour annuler).
    pub hands_free: bool,
    /// Raccourci qui colle la transcription brute (sans nettoyage) de la
    /// dernière dictée, ex. "Alt+Shift+V". Vide : désactivé.
    pub paste_raw: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: "AltGr".into(),
            min_hold_ms: 250,
            hands_free: true,
            paste_raw: "Alt+Shift+V".into(),
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
    /// Tarif du modèle, pour estimer le coût du nettoyage.
    pub pricing: Pricing,
    /// Champs ajoutés tels quels à chaque requête, propres au fournisseur
    /// (ex. désactiver la « réflexion » de DeepSeek ou GLM).
    pub extra_body: Map<String, Value>,
}

/// Délai accepté pour le nettoyage (les modèles locaux sont plus lents).
pub const LLM_TIMEOUT_RANGE_MS: std::ops::RangeInclusive<u32> = 500..=60_000;

impl LlmConfig {
    /// Nom d'hôte de `base_url`, sans port (`api.deepseek.com`, `localhost`…).
    pub fn host(&self) -> Option<&str> {
        let rest = self
            .base_url
            .strip_prefix("https://")
            .or_else(|| self.base_url.strip_prefix("http://"))?;
        let authority = rest.split('/').next()?;
        let host = match authority.strip_prefix('[') {
            // IPv6 : [::1]:8080
            Some(v6) => v6.split(']').next()?,
            None => authority.split(':').next()?,
        };
        (!host.is_empty()).then_some(host)
    }

    /// Modèle tournant sur ce PC (Ollama, LM Studio…) : pas de clé requise,
    /// HTTP simple accepté.
    pub fn is_local(&self) -> bool {
        matches!(self.host(), Some("localhost" | "127.0.0.1" | "::1"))
    }

    /// Vérifie une configuration saisie dans les réglages.
    pub fn validate(&self) -> Result<(), String> {
        if self.host().is_none() {
            return Err("adresse invalide : attendu https://hôte/…".into());
        }
        // La clé API part dans l'en-tête : jamais en clair sur le réseau.
        if !self.base_url.starts_with("https://") && !self.is_local() {
            return Err(
                "adresse en https:// obligatoire (http:// seulement pour un modèle local)".into(),
            );
        }
        if self.model.trim().is_empty() {
            return Err("nom du modèle manquant".into());
        }
        if !LLM_TIMEOUT_RANGE_MS.contains(&self.timeout_ms) {
            return Err(format!(
                "délai entre {} et {} ms",
                LLM_TIMEOUT_RANGE_MS.start(),
                LLM_TIMEOUT_RANGE_MS.end()
            ));
        }
        let p = &self.pricing;
        let prices = [
            p.input_per_million,
            p.cached_input_per_million,
            p.output_per_million,
        ];
        if prices.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err("tarifs : nombres positifs attendus".into());
        }
        if p.currency.len() != 3 || !p.currency.chars().all(|c| c.is_ascii_uppercase()) {
            return Err("devise : code à 3 lettres (USD, EUR…)".into());
        }
        Ok(())
    }
}

/// Prix par million de jetons. À ajuster si le modèle ou le fournisseur change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pricing {
    pub input_per_million: f64,
    /// Jetons d'entrée servis par le cache du fournisseur.
    pub cached_input_per_million: f64,
    pub output_per_million: f64,
    pub currency: String,
}

/// deepseek-flash au tarif plein (heures de pointe) : estimation haute, les
/// heures creuses coûtent moitié moins. Source : api-docs.deepseek.com, 11/2026.
impl Default for Pricing {
    fn default() -> Self {
        Self {
            input_per_million: 0.30,
            cached_input_per_million: 0.006,
            output_per_million: 1.20,
            currency: "USD".into(),
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            timeout_ms: 3000,
            pricing: Pricing::default(),
            extra_body: deepseek_extra_body(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Conserver les dictées sur le PC (jamais envoyées ailleurs).
    pub enabled: bool,
    /// Au-delà, les plus anciennes sont effacées.
    pub max_entries: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 1000,
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

/// DeepSeek active par défaut un mode « réflexion » lent et inutile ici.
fn deepseek_extra_body() -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert("thinking".into(), json!({"type": "disabled"}));
    extra
}

/// Anciens noms de modèles LLM → nom actuel.
const RENAMED_LLM_MODELS: &[(&str, &str)] = &[("deepseek-v4-flash", "deepseek-flash")];

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }

    /// Met à jour les valeurs obsolètes d'un fichier existant.
    /// Renvoie la liste des changements (vide si rien n'a changé).
    pub fn migrate(&mut self) -> Vec<String> {
        let mut changes = Vec::new();
        if let Some((old, new)) = RENAMED_LLM_MODELS
            .iter()
            .find(|(old, _)| *old == self.llm.model)
        {
            changes.push(format!("llm.model : {old} → {new}"));
            self.llm.model = (*new).to_owned();
        }
        changes
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
    fn migrate_renames_retired_deepseek_model() {
        let mut config = Config::from_toml("[llm]\nmodel = \"deepseek-v4-flash\"\n").unwrap();
        assert_eq!(config.migrate().len(), 1);
        assert_eq!(config.llm.model, "deepseek-flash");
        assert!(config.migrate().is_empty());
    }

    #[test]
    fn default_llm_disables_deepseek_thinking() {
        let config = Config::from_toml("[llm]\nmodel = \"deepseek-flash\"\n").unwrap();
        assert_eq!(config.llm.extra_body["thinking"]["type"], "disabled");
        // Un choix explicite (table vide) est respecté.
        let config = Config::from_toml("[llm.extra_body]\n").unwrap();
        assert!(config.llm.extra_body.is_empty());
    }

    #[test]
    fn llm_host_and_locality() {
        let llm = |url: &str| LlmConfig {
            base_url: url.into(),
            ..LlmConfig::default()
        };
        assert_eq!(llm("https://api.z.ai/api/paas/v4").host(), Some("api.z.ai"));
        assert_eq!(llm("http://localhost:11434/v1").host(), Some("localhost"));
        assert_eq!(llm("http://[::1]:8080/v1").host(), Some("::1"));
        assert_eq!(llm("ftp://x").host(), None);
        assert_eq!(llm("https://").host(), None);
        assert!(llm("http://127.0.0.1:1234/v1").is_local());
        assert!(!llm("https://api.openai.com/v1").is_local());
    }

    #[test]
    fn llm_validation() {
        let ok = LlmConfig::default();
        assert!(ok.validate().is_ok());
        let with = |f: &dyn Fn(&mut LlmConfig)| {
            let mut llm = LlmConfig::default();
            f(&mut llm);
            llm.validate()
        };
        assert!(with(&|l| l.base_url = "http://api.openai.com/v1".into()).is_err());
        assert!(with(&|l| l.base_url = "http://localhost:11434/v1".into()).is_ok());
        assert!(with(&|l| l.base_url = "pas une url".into()).is_err());
        assert!(with(&|l| l.model = "  ".into()).is_err());
        assert!(with(&|l| l.timeout_ms = 100).is_err());
        assert!(with(&|l| l.pricing.output_per_million = -1.0).is_err());
        assert!(with(&|l| l.pricing.input_per_million = f64::NAN).is_err());
        assert!(with(&|l| l.pricing.currency = "euro".into()).is_err());
    }

    #[test]
    fn unknown_key_is_ignored() {
        assert!(Config::from_toml("obsolete = 1\n").is_ok());
    }
}
