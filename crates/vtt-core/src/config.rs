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
    pub audio: AudioConfig,
    pub asr: AsrConfig,
    pub llm: LlmConfig,
    /// Configurations LLM enregistrées, pour passer de l'une à l'autre.
    pub llm_profiles: Vec<LlmProfile>,
    /// Prompts système personnalisés (le prompt actif est choisi dans `llm`).
    pub prompts: Vec<PromptPreset>,
    pub history: HistoryConfig,
    pub updates: UpdatesConfig,
    /// Mots et noms propres à bien orthographier (indices pour Whisper et le LLM).
    pub dictionary: Vec<String>,
    pub snippets: Vec<Snippet>,
    /// Consignes de nettoyage selon l'application de destination.
    pub styles: Vec<Style>,
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
    /// Raccourci qui retraite l'audio de la dernière dictée (après une erreur,
    /// ou pour la coller ailleurs), ex. "Alt+Shift+R". Vide : désactivé.
    pub retry: String,
    /// Touche qui, tenue avec le raccourci, transforme la dictée en consigne
    /// appliquée au texte sélectionné (Command Mode). Vide : désactivé.
    pub command_key: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: "AltGr".into(),
            min_hold_ms: 250,
            hands_free: true,
            paste_raw: "Alt+Shift+V".into(),
            retry: "Alt+Shift+R".into(),
            command_key: "RightCtrl".into(),
        }
    }
}

/// Touches utilisables comme raccourci de dictée ou touche de commande
/// (noms compris par la couche plateforme).
pub const HOTKEY_NAMES: &[&str] = &[
    "AltGr",
    "RightCtrl",
    "RightShift",
    "CapsLock",
    "ScrollLock",
    "Pause",
    "Insert",
    "Menu",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
];

pub const MIN_HOLD_RANGE_MS: std::ops::RangeInclusive<u32> = 50..=1000;

impl HotkeyConfig {
    /// Vérifie une configuration saisie dans les réglages (la syntaxe des
    /// raccourcis globaux est vérifiée par l'application).
    pub fn validate(&self) -> Result<(), String> {
        let known = |name: &str| HOTKEY_NAMES.iter().any(|k| k.eq_ignore_ascii_case(name));
        if !known(&self.key) {
            return Err(format!("raccourci de dictée inconnu : {}", self.key));
        }
        let command = self.command_key.trim();
        if !command.is_empty() {
            if !known(command) {
                return Err(format!("touche de commande inconnue : {command}"));
            }
            if command.eq_ignore_ascii_case(&self.key) {
                return Err("la touche de commande doit différer du raccourci de dictée".into());
            }
        }
        let retry = self.retry.trim();
        if !retry.is_empty() && retry.eq_ignore_ascii_case(self.paste_raw.trim()) {
            return Err("« réessayer » et « coller la version brute » doivent différer".into());
        }
        if !MIN_HOLD_RANGE_MS.contains(&self.min_hold_ms) {
            return Err(format!(
                "appui minimal entre {} et {} ms",
                MIN_HOLD_RANGE_MS.start(),
                MIN_HOLD_RANGE_MS.end()
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Nom du micro ; vide : micro par défaut de Windows.
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    /// Nom du fichier modèle ggml dans le dossier des modèles.
    pub model: String,
    /// Code langue ISO 639-1 imposé à Whisper.
    pub language: String,
    pub use_gpu: bool,
    /// Décharge le modèle (mémoire graphique libérée) après ce nombre de
    /// minutes sans dicter ; 0 : jamais. Il est rechargé pendant la dictée
    /// suivante.
    pub unload_after_min: u32,
    /// Décharge le modèle tant qu'un jeu ou une vidéo est en plein écran.
    pub unload_in_fullscreen: bool,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            model: "ggml-large-v3-turbo-q8_0.bin".into(),
            language: "fr".into(),
            use_gpu: true,
            unload_after_min: 0,
            unload_in_fullscreen: false,
        }
    }
}

/// Délais proposés dans les réglages pour décharger le modèle (minutes).
pub const UNLOAD_AFTER_CHOICES: &[u32] = &[0, 5, 15, 30, 60];

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
    /// Nom du prompt de nettoyage (voir `prompts`) ; vide : prompt intégré.
    pub cleanup_prompt: String,
    /// Nom du prompt du Command Mode ; vide : prompt intégré.
    pub command_prompt: String,
    /// Profil LLM (voir `llm_profiles`) essayé si celui-ci échoue, avant
    /// le repli sur le texte brut ; vide : aucun.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fallback_profile: String,
}

/// Usage d'un prompt système.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptKind {
    /// Nettoyage des dictées.
    Nettoyage,
    /// Command Mode.
    Commande,
}

/// Prompt système enregistré sous un nom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptPreset {
    pub name: String,
    pub kind: PromptKind,
    pub text: String,
}

pub const MAX_PROMPTS: usize = 50;
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// Prompts saisis dans l'interface : nom unique par usage, texte non vide.
pub fn clean_prompts(prompts: &[PromptPreset]) -> Result<Vec<PromptPreset>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut cleaned = Vec::new();
    for prompt in prompts {
        let name = prompt.name.trim();
        if name.is_empty() {
            return Err("chaque prompt a besoin d'un nom".into());
        }
        if !seen.insert((prompt.kind, name.to_lowercase())) {
            return Err(format!("nom de prompt en double : « {name} »"));
        }
        if prompt.text.trim().is_empty() {
            return Err(format!("prompt « {name} » : texte vide"));
        }
        if prompt.text.chars().count() > MAX_PROMPT_CHARS {
            return Err(format!(
                "prompt « {name} » : plus de {MAX_PROMPT_CHARS} caractères"
            ));
        }
        cleaned.push(PromptPreset {
            name: name.to_owned(),
            kind: prompt.kind,
            text: prompt.text.trim().to_owned(),
        });
    }
    if cleaned.len() > MAX_PROMPTS {
        return Err(format!("au plus {MAX_PROMPTS} prompts"));
    }
    Ok(cleaned)
}

impl Config {
    /// Texte du prompt `name` pour cet usage ; `None` : prompt intégré (nom
    /// vide, ou prompt supprimé entre-temps).
    pub fn prompt_text(&self, kind: PromptKind, name: &str) -> Option<&str> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.prompts
            .iter()
            .find(|p| p.kind == kind && p.name == name)
            .map(|p| p.text.as_str())
    }

    /// Les prompts choisis dans `llm` existent bien.
    pub fn check_prompt_refs(&self, llm: &LlmConfig) -> Result<(), String> {
        for (kind, name) in [
            (PromptKind::Nettoyage, &llm.cleanup_prompt),
            (PromptKind::Commande, &llm.command_prompt),
        ] {
            if !name.trim().is_empty() && self.prompt_text(kind, name).is_none() {
                return Err(format!("prompt introuvable : « {name} »"));
            }
        }
        Ok(())
    }
}

impl Config {
    /// Profil de secours de `llm`, s'il existe, est activé et diffère de
    /// `llm` (même adresse et même modèle : inutile de réessayer).
    pub fn fallback_for(&self, llm: &LlmConfig) -> Option<&LlmProfile> {
        let name = llm.fallback_profile.trim();
        if name.is_empty() {
            return None;
        }
        self.llm_profiles.iter().find(|p| {
            p.name == name
                && p.llm.enabled
                && (p.llm.normalized_base_url() != llm.normalized_base_url()
                    || p.llm.model.trim() != llm.model.trim())
        })
    }

    /// Le profil de secours choisi dans `llm` existe et sert à quelque chose.
    pub fn check_fallback_ref(&self, llm: &LlmConfig) -> Result<(), String> {
        let name = llm.fallback_profile.trim();
        if name.is_empty() || self.fallback_for(llm).is_some() {
            return Ok(());
        }
        match self.llm_profiles.iter().find(|p| p.name == name) {
            None => Err(format!("profil de secours introuvable : « {name} »")),
            Some(p) if !p.llm.enabled => Err(format!(
                "profil de secours « {name} » : nettoyage désactivé dans ce profil"
            )),
            Some(_) => Err(format!(
                "profil de secours « {name} » : même adresse et même modèle que les réglages \
                 actuels"
            )),
        }
    }
}

/// Configuration LLM enregistrée sous un nom (« DeepSeek », « Ollama du NAS »…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProfile {
    pub name: String,
    #[serde(flatten)]
    pub llm: LlmConfig,
}

pub const MAX_LLM_PROFILES: usize = 30;

/// Profils saisis dans l'interface : noms uniques, configurations valides,
/// adresses normalisées.
pub fn clean_llm_profiles(profiles: &[LlmProfile]) -> Result<Vec<LlmProfile>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut cleaned = Vec::new();
    for profile in profiles {
        let name = profile.name.trim();
        if name.is_empty() {
            return Err("chaque profil a besoin d'un nom".into());
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("nom de profil en double : « {name} »"));
        }
        let mut llm = profile.llm.clone();
        llm.base_url = llm.normalized_base_url();
        llm.validate()
            .map_err(|e| format!("profil « {name} » : {e}"))?;
        cleaned.push(LlmProfile {
            name: name.to_owned(),
            llm,
        });
    }
    if cleaned.len() > MAX_LLM_PROFILES {
        return Err(format!("au plus {MAX_LLM_PROFILES} profils"));
    }
    Ok(cleaned)
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

    /// Serveur sur ce PC (`localhost`, `127.x.x.x`, `::1`).
    pub fn is_loopback(&self) -> bool {
        match self.host() {
            Some("localhost" | "::1") => true,
            Some(host) => host
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|ip| ip.is_loopback()),
            None => false,
        }
    }

    /// Serveur sur ce PC ou sur le réseau local (Ollama, LM Studio…) : clé
    /// facultative, HTTP simple accepté. La clé, elle, ne circule jamais en
    /// clair hors de ce PC (voir [`LlmConfig::may_send_key`]).
    pub fn is_local(&self) -> bool {
        self.is_loopback()
            || self.host().is_some_and(|host| {
                host.ends_with(".local")
                    || host
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok_and(|ip| ip.is_private() || ip.is_link_local())
            })
    }

    /// La clé API peut être envoyée : HTTPS, ou HTTP vers ce PC seulement.
    pub fn may_send_key(&self) -> bool {
        self.base_url.starts_with("https://") || self.is_loopback()
    }

    /// Adresse nettoyée : espaces et `/` final retirés, et `/chat/completions`
    /// retiré s'il a été recopié (l'application l'ajoute elle-même).
    pub fn normalized_base_url(&self) -> String {
        let mut url = self.base_url.trim().trim_end_matches('/');
        if let Some(stripped) = url.strip_suffix("/chat/completions") {
            url = stripped.trim_end_matches('/');
        }
        url.to_owned()
    }

    /// Vérifie une configuration saisie dans les réglages.
    pub fn validate(&self) -> Result<(), String> {
        if self.host().is_none() {
            return Err("adresse invalide : attendu https://hôte/…".into());
        }
        // La clé API part dans l'en-tête : jamais en clair sur le réseau.
        if !self.base_url.starts_with("https://") && !self.is_local() {
            return Err(
                "adresse en https:// obligatoire (http:// seulement sur ce PC ou le réseau local)"
                    .into(),
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
            cleanup_prompt: String::new(),
            command_prompt: String::new(),
            fallback_profile: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatesConfig {
    /// Vérifier régulièrement si une nouvelle version est publiée (GitHub).
    /// L'installation ne se fait jamais sans l'accord de l'utilisateur.
    pub check: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self { check: true }
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

/// Style de nettoyage pour certaines applications, choisi d'après la fenêtre
/// active au début de la dictée. Le premier style qui correspond s'applique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// Nom de l'exécutable, ex. "slack.exe" (casse ignorée).
    pub apps: Vec<String>,
    /// Morceau du titre de la fenêtre, ex. "Gmail" (casse ignorée) : pour les
    /// sites ouverts dans un navigateur.
    pub titles: Vec<String>,
    /// Consignes ajoutées au nettoyage, ex. « ton décontracté ».
    pub instructions: String,
    /// `false` : insérer la transcription brute dans ces applications.
    pub cleanup: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            apps: Vec::new(),
            titles: Vec::new(),
            instructions: String::new(),
            cleanup: true,
        }
    }
}

impl Style {
    pub fn matches(&self, app: Option<&str>, title: Option<&str>) -> bool {
        let app_matches = app.is_some_and(|app| {
            let app = app.rsplit(['\\', '/']).next().unwrap_or(app);
            self.apps.iter().any(|a| a.trim().eq_ignore_ascii_case(app))
        });
        let title_matches = title.is_some_and(|title| {
            let title = title.to_lowercase();
            self.titles
                .iter()
                .map(|t| t.trim().to_lowercase())
                .any(|t| !t.is_empty() && title.contains(&t))
        });
        app_matches || title_matches
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

/// Limites des saisies de l'interface (garde-fous contre une config géante
/// qui alourdirait chaque requête).
pub const MAX_DICTIONARY_WORDS: usize = 500;
pub const MAX_WORD_CHARS: usize = 80;
pub const MAX_SNIPPETS: usize = 200;
pub const MAX_SNIPPET_CHARS: usize = 20_000;
pub const MAX_STYLES: usize = 100;
pub const MAX_INSTRUCTIONS_CHARS: usize = 2_000;

/// Dictionnaire saisi dans l'interface : mots nettoyés, doublons retirés
/// (casse ignorée), ordre conservé.
pub fn clean_dictionary(words: &[String]) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut cleaned = Vec::new();
    for word in words.iter().map(|w| w.trim()).filter(|w| !w.is_empty()) {
        if word.chars().count() > MAX_WORD_CHARS {
            return Err(format!(
                "mot trop long (plus de {MAX_WORD_CHARS} caractères)"
            ));
        }
        if seen.insert(word.to_lowercase()) {
            cleaned.push(word.to_owned());
        }
    }
    if cleaned.len() > MAX_DICTIONARY_WORDS {
        return Err(format!("au plus {MAX_DICTIONARY_WORDS} mots"));
    }
    Ok(cleaned)
}

/// Snippets saisis dans l'interface : lignes vides retirées, déclencheurs
/// distincts (après normalisation : casse, accents, ponctuation).
pub fn clean_snippets(snippets: &[Snippet]) -> Result<Vec<Snippet>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut cleaned = Vec::new();
    for snippet in snippets {
        let trigger = snippet.trigger.trim();
        if trigger.is_empty() && snippet.text.trim().is_empty() {
            continue;
        }
        let normalized = crate::text::normalize_phrase(trigger);
        if normalized.is_empty() {
            return Err("chaque snippet a besoin d'un déclencheur (des mots)".into());
        }
        if snippet.text.trim().is_empty() {
            return Err(format!("snippet « {trigger} » : texte vide"));
        }
        if snippet.text.chars().count() > MAX_SNIPPET_CHARS {
            return Err(format!(
                "snippet « {trigger} » : plus de {MAX_SNIPPET_CHARS} caractères"
            ));
        }
        if !seen.insert(normalized) {
            return Err(format!("déclencheur en double : « {trigger} »"));
        }
        cleaned.push(Snippet {
            trigger: trigger.to_owned(),
            text: snippet.text.clone(),
        });
    }
    if cleaned.len() > MAX_SNIPPETS {
        return Err(format!("au plus {MAX_SNIPPETS} snippets"));
    }
    Ok(cleaned)
}

/// Styles saisis dans l'interface : listes nettoyées, chaque style vise au
/// moins une application ou un titre.
pub fn clean_styles(styles: &[Style]) -> Result<Vec<Style>, String> {
    let clean_list = |items: &[String]| -> Vec<String> {
        items
            .iter()
            .map(|i| i.trim().to_owned())
            .filter(|i| !i.is_empty())
            .collect()
    };
    let mut cleaned = Vec::new();
    for (index, style) in styles.iter().enumerate() {
        let apps = clean_list(&style.apps);
        let titles = clean_list(&style.titles);
        let instructions = style.instructions.trim().to_owned();
        if apps.is_empty() && titles.is_empty() {
            if instructions.is_empty() {
                continue;
            }
            return Err(format!(
                "style n°{} : indiquez au moins une application ou un titre",
                index + 1
            ));
        }
        if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            return Err(format!(
                "style n°{} : consignes de plus de {MAX_INSTRUCTIONS_CHARS} caractères",
                index + 1
            ));
        }
        cleaned.push(Style {
            apps,
            titles,
            instructions,
            cleanup: style.cleanup,
        });
    }
    if cleaned.len() > MAX_STYLES {
        return Err(format!("au plus {MAX_STYLES} styles"));
    }
    Ok(cleaned)
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
        // Réseau local : clé facultative, mais jamais envoyée en clair.
        let lan = llm("http://192.168.1.20:11434/v1");
        assert!(lan.is_local() && !lan.is_loopback() && !lan.may_send_key());
        assert!(llm("http://nas.local:11434/v1").is_local());
        assert!(llm("http://localhost:11434/v1").may_send_key());
        assert!(llm("https://api.z.ai/api/paas/v4").may_send_key());
        assert!(!llm("http://8.8.8.8/v1").is_local());
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
        assert!(with(&|l| l.base_url = "http://192.168.1.20:11434/v1".into()).is_ok());
        assert!(with(&|l| l.base_url = "http://10.0.0.5:8080/v1".into()).is_ok());
        assert!(with(&|l| l.base_url = "http://203.0.113.7/v1".into()).is_err());
        assert!(with(&|l| l.base_url = "pas une url".into()).is_err());
        assert!(with(&|l| l.model = "  ".into()).is_err());
        assert!(with(&|l| l.timeout_ms = 100).is_err());
        assert!(with(&|l| l.pricing.output_per_million = -1.0).is_err());
        assert!(with(&|l| l.pricing.input_per_million = f64::NAN).is_err());
        assert!(with(&|l| l.pricing.currency = "euro".into()).is_err());
    }

    #[test]
    fn style_matches_app_or_title() {
        let style = Style {
            apps: vec!["Slack.exe".into()],
            titles: vec!["Gmail".into(), " ".into()],
            ..Style::default()
        };
        assert!(style.matches(Some("slack.exe"), None));
        assert!(style.matches(Some(r"C:\Program Files\Slack\slack.exe"), None));
        assert!(style.matches(Some("chrome.exe"), Some("Boîte de réception - GMAIL")));
        assert!(!style.matches(Some("chrome.exe"), Some("Recherche Google")));
        assert!(!style.matches(None, None));
        // Un titre vide dans la liste ne correspond pas à tout.
        assert!(!style.matches(None, Some("n'importe quoi")));
    }

    #[test]
    fn styles_parse_from_toml() {
        let config = Config::from_toml(
            "[[styles]]\napps = [\"code.exe\"]\ncleanup = false\n\n[[styles]]\ntitles = [\"Gmail\"]\ninstructions = \"Ton soigné.\"\n",
        )
        .unwrap();
        assert_eq!(config.styles.len(), 2);
        assert!(!config.styles[0].cleanup);
        assert!(config.styles[1].cleanup, "cleanup vaut true par défaut");
    }

    #[test]
    fn unknown_key_is_ignored() {
        assert!(Config::from_toml("obsolete = 1\n").is_ok());
    }

    #[test]
    fn dictionary_is_trimmed_and_deduplicated() {
        let words = ["  Tauri ", "", "tauri", "Aquadrox"].map(String::from);
        assert_eq!(clean_dictionary(&words).unwrap(), ["Tauri", "Aquadrox"]);
        assert!(clean_dictionary(&["x".repeat(81)]).is_err());
        assert!(clean_dictionary(&vec!["a".into(); 1]).is_ok());
        let many: Vec<String> = (0..501).map(|i| format!("mot{i}")).collect();
        assert!(clean_dictionary(&many).is_err());
    }

    #[test]
    fn snippets_are_validated() {
        let snippet = |t: &str, x: &str| Snippet {
            trigger: t.into(),
            text: x.into(),
        };
        let cleaned =
            clean_snippets(&[snippet(" ma signature ", "Florian"), snippet("", "  ")]).unwrap();
        assert_eq!(cleaned, [snippet("ma signature", "Florian")]);
        assert!(
            clean_snippets(&[snippet("?!", "x")]).is_err(),
            "déclencheur sans mots"
        );
        assert!(
            clean_snippets(&[snippet("adresse", " ")]).is_err(),
            "texte vide"
        );
        assert!(
            clean_snippets(&[snippet("Mon adresse", "a"), snippet("mon adresse !", "b")]).is_err(),
            "doublon après normalisation"
        );
    }

    #[test]
    fn styles_are_validated() {
        let style = |apps: &[&str], instructions: &str| Style {
            apps: apps.iter().map(|a| a.to_string()).collect(),
            instructions: instructions.into(),
            ..Style::default()
        };
        let cleaned =
            clean_styles(&[style(&[" slack.exe ", ""], " court "), style(&[], "")]).unwrap();
        assert_eq!(cleaned, [style(&["slack.exe"], "court")]);
        assert!(
            clean_styles(&[style(&[], "ton soigné")]).is_err(),
            "aucune cible"
        );
        assert!(clean_styles(&[style(&["a.exe"], &"x".repeat(2001))]).is_err());
    }

    #[test]
    fn fallback_profile_lookup() {
        let ollama = LlmConfig {
            base_url: "http://192.168.1.20:11434/v1".into(),
            model: "qwen3:8b".into(),
            ..LlmConfig::default()
        };
        let mut config = Config {
            llm_profiles: vec![
                LlmProfile {
                    name: "Ollama".into(),
                    llm: ollama.clone(),
                },
                LlmProfile {
                    name: "DeepSeek".into(),
                    llm: LlmConfig::default(),
                },
            ],
            ..Config::default()
        };
        let mut llm = LlmConfig::default();
        assert!(config.fallback_for(&llm).is_none(), "aucun secours choisi");
        config.check_fallback_ref(&llm).unwrap();

        llm.fallback_profile = "Ollama".into();
        assert_eq!(config.fallback_for(&llm).unwrap().llm, ollama);
        config.check_fallback_ref(&llm).unwrap();

        llm.fallback_profile = "DeepSeek".into();
        assert!(config.fallback_for(&llm).is_none(), "même fournisseur");
        assert!(config.check_fallback_ref(&llm).is_err());

        llm.fallback_profile = "Inconnu".into();
        assert!(config.check_fallback_ref(&llm).is_err());

        config.llm_profiles[0].llm.enabled = false;
        llm.fallback_profile = "Ollama".into();
        assert!(config.fallback_for(&llm).is_none(), "secours désactivé");
        assert!(config.check_fallback_ref(&llm).is_err());
    }

    #[test]
    fn empty_fallback_is_not_written() {
        let text = Config::default().to_toml().unwrap();
        assert!(!text.contains("fallback_profile"), "{text}");
    }

    #[test]
    fn hotkey_validation() {
        assert!(HotkeyConfig::default().validate().is_ok());
        let with = |f: &dyn Fn(&mut HotkeyConfig)| {
            let mut hotkey = HotkeyConfig::default();
            f(&mut hotkey);
            hotkey.validate()
        };
        assert!(with(&|h| h.key = "F13".into()).is_ok());
        assert!(with(&|h| h.key = "Espace".into()).is_err());
        assert!(
            with(&|h| h.command_key = String::new()).is_ok(),
            "commande désactivée"
        );
        assert!(
            with(&|h| h.command_key = "altgr".into()).is_err(),
            "même touche"
        );
        assert!(with(&|h| h.min_hold_ms = 10).is_err());
        assert!(
            with(&|h| h.retry = "alt+shift+v".into()).is_err(),
            "même raccourci que la version brute"
        );
        assert!(
            with(&|h| {
                h.retry = String::new();
                h.paste_raw = String::new();
            })
            .is_ok(),
            "deux raccourcis désactivés"
        );
    }

    /// L'exemple complet du manuel (README) reste une configuration valide.
    #[test]
    fn readme_example_is_valid() {
        // Fins de ligne \r\n possibles (Git sous Windows) : on cherche la fin de
        // la ligne d'ouverture plutôt qu'un « ```toml\n » exact.
        let readme = include_str!("../../../README.md");
        let fence = readme.find("```toml").expect("bloc toml dans le README");
        let start = fence + readme[fence..].find('\n').unwrap() + 1;
        let end = start + readme[start..].find("```").unwrap();
        let config = Config::from_toml(&readme[start..end]).expect("TOML du README");
        config.hotkey.validate().unwrap();
        config.llm.validate().unwrap();
        config.check_fallback_ref(&config.llm).unwrap();
        clean_dictionary(&config.dictionary).unwrap();
        assert_eq!(clean_snippets(&config.snippets).unwrap(), config.snippets);
        assert_eq!(clean_styles(&config.styles).unwrap(), config.styles);
        assert!(!config.styles[1].cleanup);
        assert_eq!(
            clean_llm_profiles(&config.llm_profiles).unwrap(),
            config.llm_profiles
        );
        assert!(config.llm_profiles[0].llm.is_local());
        assert_eq!(clean_prompts(&config.prompts).unwrap(), config.prompts);
        config.check_prompt_refs(&config.llm).unwrap();
    }

    #[test]
    fn base_url_is_normalized() {
        let llm = |url: &str| LlmConfig {
            base_url: url.into(),
            ..LlmConfig::default()
        };
        assert_eq!(
            llm(" http://192.168.1.20:11434/v1/ ").normalized_base_url(),
            "http://192.168.1.20:11434/v1"
        );
        assert_eq!(
            llm("https://api.openai.com/v1/chat/completions").normalized_base_url(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            llm("https://api.deepseek.com").normalized_base_url(),
            "https://api.deepseek.com"
        );
    }

    #[test]
    fn llm_profiles_round_trip_and_validation() {
        let mut config = Config::default();
        let mut ollama = LlmConfig {
            base_url: "http://192.168.1.20:11434/v1/".into(),
            model: "qwen3:8b".into(),
            timeout_ms: 15_000,
            ..LlmConfig::default()
        };
        ollama.extra_body.clear();
        config.llm_profiles = vec![
            LlmProfile {
                name: " Ollama du NAS ".into(),
                llm: ollama.clone(),
            },
            LlmProfile {
                name: "DeepSeek".into(),
                llm: LlmConfig::default(),
            },
        ];
        let parsed = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(parsed, config);

        let cleaned = clean_llm_profiles(&config.llm_profiles).unwrap();
        assert_eq!(cleaned[0].name, "Ollama du NAS");
        assert_eq!(cleaned[0].llm.base_url, "http://192.168.1.20:11434/v1");
        let duplicate = [cleaned[1].clone(), cleaned[1].clone()];
        assert!(clean_llm_profiles(&duplicate).is_err());
        let mut invalid = cleaned[0].clone();
        invalid.llm.model = String::new();
        assert!(clean_llm_profiles(&[invalid]).is_err());
    }

    #[test]
    fn prompts_are_validated_and_resolved() {
        let prompt = |name: &str, kind, text: &str| PromptPreset {
            name: name.into(),
            kind,
            text: text.into(),
        };
        let prompts = clean_prompts(&[
            prompt(" Concis ", PromptKind::Nettoyage, " Corrige.\n"),
            prompt("Concis", PromptKind::Commande, "Applique."),
        ])
        .unwrap();
        assert_eq!(prompts[0].name, "Concis");
        assert_eq!(prompts[0].text, "Corrige.");
        assert!(
            clean_prompts(&[
                prompt("A", PromptKind::Nettoyage, "x"),
                prompt("a", PromptKind::Nettoyage, "y")
            ])
            .is_err(),
            "même nom, même usage"
        );
        assert!(clean_prompts(&[prompt("A", PromptKind::Nettoyage, " ")]).is_err());

        let config = Config {
            prompts,
            ..Config::default()
        };
        assert_eq!(
            config.prompt_text(PromptKind::Nettoyage, "Concis"),
            Some("Corrige.")
        );
        assert_eq!(config.prompt_text(PromptKind::Nettoyage, ""), None);
        let mut llm = LlmConfig {
            cleanup_prompt: "Concis".into(),
            ..LlmConfig::default()
        };
        assert!(config.check_prompt_refs(&llm).is_ok());
        llm.command_prompt = "Inconnu".into();
        assert!(config.check_prompt_refs(&llm).is_err());

        let toml = config.to_toml().unwrap();
        assert!(toml.contains("kind = \"nettoyage\""));
        assert_eq!(Config::from_toml(&toml).unwrap(), config);
    }
}
