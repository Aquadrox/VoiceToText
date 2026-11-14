//! Fournisseurs LLM et clés API du téléphone, comme sur PC : préréglages
//! qui remplissent le formulaire, une clé par fournisseur (rangée par hôte
//! de l'adresse), liste des clés enregistrées avec leur usage.
//!
//! Les clés elles-mêmes restent côté Kotlin (chiffrées par le Keystore
//! Android) : ce module ne voit que les noms d'hôte, jamais une clé.

use serde::Serialize;
use serde_json::{Map, Value, json};
use vtt_core::config::{LlmConfig, Pricing};

use crate::settings;

/// Préréglage : il remplit le formulaire, tout reste modifiable. Même liste
/// que sur PC (`apps/desktop/ui/settings.js`), Ollama vu depuis le téléphone.
#[derive(Debug, Serialize)]
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    /// Vide : adresse à saisir (serveur du réseau local).
    pub base_url: &'static str,
    /// Vide : nom exact du modèle à saisir.
    pub model: &'static str,
    /// Exemple d'adresse affiché dans le champ vide.
    pub hint: &'static str,
    pub note: &'static str,
    #[serde(skip)]
    timeout_ms: u32,
    /// Désactive la « réflexion » (DeepSeek, GLM), lente et inutile ici.
    #[serde(skip)]
    no_thinking: bool,
    /// Tarif connu (sinon à renseigner, coût compté à 0).
    #[serde(skip)]
    priced: bool,
}

pub const CUSTOM: &str = "custom";
const OLLAMA_PORT: &str = ":11434";

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com",
        model: "deepseek-flash",
        hint: "",
        note: "Clé sur platform.deepseek.com. Coût estimé au tarif plein (heures creuses à moitié prix).",
        timeout_ms: 3000,
        no_thinking: true,
        priced: true,
    },
    Provider {
        id: "zai",
        label: "GLM (Z.ai)",
        base_url: "https://api.z.ai/api/paas/v4",
        model: "glm-5.2",
        hint: "",
        note: "Clé sur z.ai. La série GLM-5.3 ne permet pas de désactiver la réflexion : préférez GLM-5.2.",
        timeout_ms: 4000,
        no_thinking: true,
        priced: false,
    },
    Provider {
        id: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com/v1",
        model: "",
        hint: "",
        note: "Clé sur platform.openai.com. Indiquez le nom exact du modèle.",
        timeout_ms: 4000,
        no_thinking: false,
        priced: false,
    },
    Provider {
        id: "mistral",
        label: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        model: "mistral-small-latest",
        hint: "",
        note: "Clé sur console.mistral.ai.",
        timeout_ms: 4000,
        no_thinking: false,
        priced: false,
    },
    Provider {
        id: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        model: "",
        hint: "",
        note: "Clé sur openrouter.ai. Modèle au format « fournisseur/modèle ».",
        timeout_ms: 5000,
        no_thinking: false,
        priced: false,
    },
    Provider {
        id: "ollama",
        label: "Ollama (réseau local)",
        base_url: "",
        model: "",
        hint: "http://ADRESSE-IP:11434/v1",
        note: "Pas de clé ni de coût. Adresse du PC qui fait tourner Ollama sur le même Wi-Fi (Ollama lancé avec OLLAMA_HOST=0.0.0.0).",
        timeout_ms: 15000,
        no_thinking: false,
        priced: false,
    },
    Provider {
        id: CUSTOM,
        label: "Autre (compatible OpenAI)",
        base_url: "",
        model: "",
        hint: "https://hôte/v1",
        note: "Adresse et modèle libres. Une clé par fournisseur : celle d'un autre n'est jamais envoyée.",
        timeout_ms: 5000,
        no_thinking: false,
        priced: false,
    },
];

fn llm_for(base_url: &str) -> LlmConfig {
    let mut llm = LlmConfig {
        base_url: base_url.to_owned(),
        ..LlmConfig::default()
    };
    llm.base_url = llm.normalized_base_url();
    llm
}

/// Préréglage d'une adresse : même hôte qu'un fournisseur connu, port
/// d'Ollama, sinon « Autre ».
pub fn provider_of(base_url: &str) -> &'static Provider {
    let llm = llm_for(base_url);
    let host = llm.host();
    PROVIDERS
        .iter()
        .find(|p| !p.base_url.is_empty() && host.is_some() && llm_for(p.base_url).host() == host)
        .or_else(|| {
            let authority = llm.base_url.split("//").nth(1)?.split('/').next()?;
            authority
                .ends_with(OLLAMA_PORT)
                .then(|| PROVIDERS.iter().find(|p| p.id == "ollama"))?
        })
        .unwrap_or_else(|| custom())
}

fn custom() -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|p| p.id == CUSTOM)
        .expect("préréglage « Autre » présent")
}

/// Fournisseur changé : délai, tarif et paramètres propres au fournisseur
/// suivent son préréglage, pour ne pas envoyer à OpenAI le paramètre
/// « thinking » de DeepSeek ni compter le coût au tarif de l'ancien.
pub fn adopt(llm: &mut LlmConfig) {
    let provider = provider_of(&llm.base_url);
    llm.timeout_ms = provider.timeout_ms;
    llm.pricing = if provider.priced {
        Pricing::default()
    } else {
        // Tarif à renseigner : coût compté à 0, devise gardée.
        Pricing {
            input_per_million: 0.0,
            cached_input_per_million: 0.0,
            output_per_million: 0.0,
            currency: std::mem::take(&mut llm.pricing.currency),
        }
    };
    llm.extra_body = if provider.no_thinking {
        no_thinking()
    } else {
        Map::new()
    };
}

fn no_thinking() -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert("thinking".into(), json!({"type": "disabled"}));
    extra
}

/// État de la clé pour l'adresse saisie (pas encore enregistrée).
#[derive(Debug, Serialize)]
pub struct KeyTarget {
    /// Hôte où la clé est rangée ; vide : adresse invalide.
    pub host: String,
    /// Préréglage reconnu.
    pub provider: &'static str,
    /// Serveur du réseau local : clé facultative.
    pub optional: bool,
    /// Adresse HTTP hors du téléphone : une clé ne serait pas envoyée.
    pub cleartext: bool,
}

pub fn key_target(base_url: &str) -> KeyTarget {
    let llm = llm_for(base_url);
    KeyTarget {
        host: llm.host().unwrap_or_default().to_owned(),
        provider: provider_of(&llm.base_url).id,
        optional: llm.is_local(),
        cleartext: llm.host().is_some() && !llm.may_send_key(),
    }
}

/// Clé enregistrée pour un fournisseur : seulement l'hôte et son usage.
#[derive(Debug, Serialize, PartialEq)]
pub struct KeyEntry {
    pub host: String,
    /// Nom du préréglage (« DeepSeek »…) ; vide : adresse libre.
    pub provider: &'static str,
    /// « réglage actif » et profils importés du PC qui utilisent ce fournisseur.
    pub used_by: Vec<String>,
}

/// Liste des clés : `hosts`, les hôtes ayant une clé (lus par Kotlin sans
/// déchiffrer les clés), triés, avec leur usage dans `toml`.
pub fn keys(toml: &str, hosts: &[String]) -> Result<Vec<KeyEntry>, String> {
    let (config, _) = settings::parse(toml)?;
    let mut hosts: Vec<&String> = hosts.iter().filter(|h| !h.is_empty()).collect();
    hosts.sort();
    hosts.dedup();
    Ok(hosts
        .into_iter()
        .map(|host| {
            let host = host.as_str();
            let active = (config.llm.host() == Some(host)).then(|| "réglage actif".to_owned());
            let profiles = config
                .llm_profiles
                .iter()
                .filter(|p| p.llm.host() == Some(host))
                .map(|p| format!("profil « {} »", p.name));
            let provider = provider_of(&format!("https://{host}"));
            KeyEntry {
                host: host.to_owned(),
                provider: if provider.id == CUSTOM {
                    ""
                } else {
                    provider.label
                },
                used_by: active.into_iter().chain(profiles).collect(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vtt_core::Config;
    use vtt_core::config::LlmProfile;

    #[test]
    fn providers_are_recognised_by_host() {
        assert_eq!(provider_of("https://api.deepseek.com/").id, "deepseek");
        assert_eq!(provider_of("https://api.deepseek.com/v1").id, "deepseek");
        assert_eq!(provider_of("https://api.z.ai/api/paas/v4").id, "zai");
        assert_eq!(provider_of("https://openrouter.ai/api/v1").id, "openrouter");
        assert_eq!(provider_of("http://192.168.1.20:11434/v1").id, "ollama");
        assert_eq!(provider_of("https://llm.exemple.fr/v1").id, CUSTOM);
        assert_eq!(provider_of("").id, CUSTOM);
        // Hôte trompeur : jamais pris pour DeepSeek.
        assert_eq!(
            provider_of("https://api.deepseek.com.exemple.fr/v1").id,
            CUSTOM
        );
        // Chaque préréglage avec adresse se reconnaît lui-même.
        for p in PROVIDERS.iter().filter(|p| !p.base_url.is_empty()) {
            assert_eq!(provider_of(p.base_url).id, p.id);
        }
    }

    #[test]
    fn key_target_follows_the_typed_address() {
        let target = key_target(" https://api.mistral.ai/v1/chat/completions ");
        assert_eq!(target.host, "api.mistral.ai");
        assert_eq!(target.provider, "mistral");
        assert!(!target.optional && !target.cleartext);
        let local = key_target("http://192.168.1.20:11434/v1");
        assert!(local.optional);
        // Clé jamais envoyée en clair hors du téléphone.
        assert!(local.cleartext);
        assert!(key_target("http://exemple.com").cleartext);
        assert_eq!(key_target("pas une adresse").host, "");
    }

    #[test]
    fn switching_provider_drops_the_previous_provider_parameters() {
        let mut llm = LlmConfig::default();
        assert!(llm.extra_body.contains_key("thinking"));
        llm.base_url = "https://api.openai.com/v1".into();
        adopt(&mut llm);
        assert!(llm.extra_body.is_empty(), "pas de « thinking » chez OpenAI");
        assert_eq!(llm.pricing.input_per_million, 0.0);
        assert_eq!(llm.pricing.currency, Pricing::default().currency);
        assert_eq!(llm.timeout_ms, 4000);
        llm.base_url = "https://api.z.ai/api/paas/v4".into();
        adopt(&mut llm);
        assert!(llm.extra_body.contains_key("thinking"));
        llm.base_url = "https://api.deepseek.com".into();
        adopt(&mut llm);
        assert_eq!(llm.pricing, Pricing::default());
        assert_eq!(llm.timeout_ms, 3000);
        llm.base_url = "http://192.168.1.20:11434/v1".into();
        adopt(&mut llm);
        assert_eq!(llm.timeout_ms, 15000);
    }

    #[test]
    fn keys_are_listed_with_their_usage() {
        let mut config = Config::default();
        config.llm_profiles.push(LlmProfile {
            name: "Mistral".into(),
            llm: LlmConfig {
                base_url: "https://api.mistral.ai/v1".into(),
                ..LlmConfig::default()
            },
        });
        let toml = config.to_toml().unwrap();
        let hosts = [
            "llm.exemple.fr".to_owned(),
            "api.mistral.ai".to_owned(),
            "api.deepseek.com".to_owned(),
            "api.deepseek.com".to_owned(),
            String::new(),
        ];
        let listed = keys(&toml, &hosts).unwrap();
        assert_eq!(
            listed,
            [
                KeyEntry {
                    host: "api.deepseek.com".into(),
                    provider: "DeepSeek",
                    used_by: vec!["réglage actif".into()],
                },
                KeyEntry {
                    host: "api.mistral.ai".into(),
                    provider: "Mistral",
                    used_by: vec!["profil « Mistral »".into()],
                },
                KeyEntry {
                    host: "llm.exemple.fr".into(),
                    provider: "",
                    used_by: vec![],
                },
            ]
        );
        assert!(keys("pas du toml = =", &hosts).is_err());
    }
}
