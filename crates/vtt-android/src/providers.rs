//! Fournisseurs LLM et clés API du téléphone, comme sur PC : préréglages
//! qui remplissent le formulaire, une clé par fournisseur (rangée par hôte
//! de l'adresse), liste des clés enregistrées avec leur usage.
//!
//! Les clés elles-mêmes restent côté Kotlin (chiffrées par le Keystore
//! Android) : ce module ne voit que les noms d'hôte, jamais une clé.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vtt_core::config::{LlmConfig, Pricing, PromptKind};
use vtt_core::usage::Usage;
use vtt_core::{Directives, PostProcessor};
use vtt_llm::{LlmCleaner, Prompts};

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
    /// Adresse et modèle essayés par « Tester » : réglage actif, sinon
    /// profil, sinon préréglage (modèle vide : liste des modèles seulement).
    pub base_url: String,
    pub model: String,
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
            let (base_url, model) = std::iter::once(&config.llm)
                .chain(config.llm_profiles.iter().map(|p| &p.llm))
                .find(|llm| llm.host() == Some(host))
                .map(|llm| (llm.normalized_base_url(), llm.model.trim().to_owned()))
                .unwrap_or_else(|| {
                    if provider.base_url.is_empty() {
                        (format!("https://{host}"), String::new())
                    } else {
                        (provider.base_url.to_owned(), provider.model.to_owned())
                    }
                });
            KeyEntry {
                base_url,
                model,
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

/// Phrase d'exemple nettoyée par le test (la même que sur PC).
const SAMPLE: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";

/// Réglages à tester, saisis mais pas encore enregistrés.
#[derive(Debug, Deserialize)]
pub struct TestRequest {
    pub base_url: String,
    /// Vide : seule la liste des modèles est demandée (vérifie la clé).
    pub model: String,
    /// Clé saisie, sinon celle enregistrée pour cette adresse (vide : aucune).
    pub key: String,
}

/// Compte rendu du test, comme sur PC. Jamais la clé elle-même.
#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub ok: bool,
    /// Adresse réellement appelée.
    pub endpoint: String,
    pub host: String,
    pub model: String,
    /// « envoyée », « aucune », « non envoyée (…) ».
    pub key: &'static str,
    pub duration_ms: u64,
    pub input: &'static str,
    pub output: Option<String>,
    pub error: Option<String>,
    pub usage: Option<Usage>,
    pub cost: Option<f64>,
    pub currency: String,
    /// Modèles annoncés par le serveur (`GET …/models`).
    pub models: Vec<String>,
    pub models_error: Option<String>,
    pub hints: Vec<String>,
}

/// Teste des réglages sans les enregistrer : nettoyage de la phrase
/// d'exemple (si un modèle est donné) et liste des modèles du serveur, avec
/// des pistes en cas d'erreur. Le fournisseur testé reçoit les paramètres
/// qu'il aurait une fois enregistré. Bloquant (réseau) : hors du fil
/// principal.
pub fn test(toml: &str, request: &TestRequest) -> Result<Diagnostic, String> {
    let (config, _) = settings::parse(toml)?;
    let mut llm = config.llm.clone();
    llm.enabled = true;
    llm.base_url = request.base_url.trim().to_owned();
    llm.base_url = llm.normalized_base_url();
    llm.model = request.model.trim().to_owned();
    if llm.host() != config.llm.host() {
        adopt(&mut llm);
    }
    let check_only_key = llm.model.is_empty();
    if check_only_key {
        // Modèle factice pour la validation : seule la liste est demandée.
        llm.model = "-".into();
        llm.validate()?;
        llm.model.clear();
    } else {
        llm.validate()?;
    }
    let key = Some(request.key.trim().to_owned()).filter(|k| !k.is_empty());
    let key_status = match (&key, llm.may_send_key()) {
        (None, _) => "aucune",
        (Some(_), true) => "envoyée",
        (Some(_), false) => "non envoyée (http:// : jamais en clair hors du téléphone)",
    };
    let text = |kind, name: &str| config.prompt_text(kind, name).map(str::to_owned);
    let prompts = Prompts {
        cleanup: text(PromptKind::Nettoyage, &llm.cleanup_prompt),
        command: text(PromptKind::Commande, &llm.command_prompt),
        ..Prompts::default()
    };
    let provided = key.clone();
    let cleaner = LlmCleaner::new(
        &llm,
        &config.dictionary,
        &prompts,
        Box::new(move || provided.clone()),
    );
    let started = Instant::now();
    let result = (!check_only_key).then(|| cleaner.process(SAMPLE, Directives::default()));
    let duration_ms = started.elapsed().as_millis() as u64;
    let (models, models_error) = match vtt_llm::list_models(&llm, key) {
        Ok(models) => (models, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let mut diagnostic = Diagnostic {
        ok: false,
        endpoint: cleaner.endpoint().to_owned(),
        host: llm.host().unwrap_or_default().to_owned(),
        model: llm.model.clone(),
        key: key_status,
        duration_ms,
        input: SAMPLE,
        output: None,
        error: None,
        usage: None,
        cost: None,
        currency: llm.pricing.currency.clone(),
        models,
        models_error,
        hints: Vec::new(),
    };
    match result {
        Some(Ok(cleaned)) => {
            diagnostic.ok = true;
            diagnostic.cost = cleaned.usage.map(|u| llm.pricing.cost(&u));
            diagnostic.usage = cleaned.usage;
            diagnostic.output = Some(cleaned.text);
        }
        Some(Err(e)) => {
            let error = e.to_string();
            diagnostic.hints = vtt_llm::hints(&llm, &error);
            diagnostic.error = Some(error);
        }
        // Clé seule : la liste des modèles fait foi.
        None => {
            diagnostic.ok = diagnostic.models_error.is_none();
            if let Some(error) = &diagnostic.models_error {
                diagnostic.hints = vtt_llm::hints(&llm, error);
            }
        }
    }
    if !check_only_key
        && !diagnostic.models.is_empty()
        && !diagnostic.models.iter().any(|m| m == &llm.model)
    {
        diagnostic.hints.push(format!(
            "Le modèle « {} » n'est pas dans la liste annoncée par le serveur : vérifiez son nom \
             exact.",
            llm.model
        ));
    }
    Ok(diagnostic)
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
                    base_url: "https://api.deepseek.com".into(),
                    model: "deepseek-flash".into(),
                },
                KeyEntry {
                    host: "api.mistral.ai".into(),
                    provider: "Mistral",
                    used_by: vec!["profil « Mistral »".into()],
                    base_url: "https://api.mistral.ai/v1".into(),
                    model: "deepseek-flash".into(),
                },
                KeyEntry {
                    host: "llm.exemple.fr".into(),
                    provider: "",
                    used_by: vec![],
                    base_url: "https://llm.exemple.fr".into(),
                    model: String::new(),
                },
            ]
        );
        assert!(keys("pas du toml = =", &hosts).is_err());
    }

    /// Serveur compatible OpenAI jetable (127.0.0.1) : répond à
    /// `requests` requêtes, renvoie son adresse et les requêtes reçues.
    fn fake_server(requests: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let (status, body) = if !text.contains("Bearer bonne-cle") {
                    (401, r#"{"error":{"message":"invalid api key"}}"#)
                } else if text.starts_with("GET") {
                    (200, r#"{"data":[{"id":"petit"},{"id":"grand"}]}"#)
                } else {
                    (
                        200,
                        r#"{"choices":[{"message":{"content":"Bonjour, la réunion est mercredi à 14 h."}}],"usage":{"prompt_tokens":100,"completion_tokens":20}}"#,
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                seen.push(text);
            }
            seen
        });
        (url, handle)
    }

    fn request(base_url: &str, model: &str, key: &str) -> TestRequest {
        TestRequest {
            base_url: base_url.into(),
            model: model.into(),
            key: key.into(),
        }
    }

    #[test]
    fn a_working_key_is_reported_with_the_server_models() {
        let (url, server) = fake_server(2);
        let report = test("", &request(&url, "petit", "bonne-cle")).unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(report.key, "envoyée");
        assert_eq!(report.host, "127.0.0.1");
        assert_eq!(
            report.output.as_deref(),
            Some("Bonjour, la réunion est mercredi à 14 h.")
        );
        assert_eq!(report.usage.map(|u| u.output_tokens), Some(20));
        assert_eq!(report.models, ["grand", "petit"]);
        assert!(report.hints.is_empty(), "{:?}", report.hints);
        let seen = server.join().unwrap();
        // Adresse changée : pas de « thinking » de DeepSeek envoyé.
        assert!(!seen[0].contains("thinking"), "{}", seen[0]);
    }

    #[test]
    fn a_refused_key_gives_a_hint() {
        let (url, server) = fake_server(2);
        let report = test("", &request(&url, "inconnu", "mauvaise")).unwrap();
        assert!(!report.ok);
        assert!(
            report.error.as_deref().unwrap_or("").contains("401"),
            "{report:?}"
        );
        assert!(
            report.hints.iter().any(|h| h.contains("Clé refusée")),
            "{:?}",
            report.hints
        );
        server.join().unwrap();
    }

    #[test]
    fn without_a_model_only_the_key_is_checked() {
        let (url, server) = fake_server(1);
        let report = test("", &request(&url, "", "bonne-cle")).unwrap();
        assert!(report.ok, "{report:?}");
        assert!(report.output.is_none());
        assert_eq!(report.models.len(), 2);
        assert_eq!(server.join().unwrap().len(), 1, "seulement GET /models");
        // Rien n'écoute : échec expliqué.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", closed.local_addr().unwrap());
        drop(closed);
        let report = test("", &request(&url, "", "")).unwrap();
        assert!(!report.ok);
        assert_eq!(report.key, "aucune");
        assert!(!report.hints.is_empty(), "{report:?}");
    }

    #[test]
    fn invalid_settings_are_refused_before_any_request() {
        assert!(test("", &request("http://exemple.com", "m", "k")).is_err());
        assert!(test("", &request("pas une adresse", "m", "k")).is_err());
    }
}
