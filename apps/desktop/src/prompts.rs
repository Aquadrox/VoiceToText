//! Page Prompts : prompts système personnalisés (nettoyage, Command Mode,
//! compte rendu de réunion),
//! choix du prompt actif, test. Appliqués à chaud.

use std::time::Instant;

use serde::Serialize;
use tauri::State;
use vtt_core::config::{self, LlmConfig, PromptKind, PromptPreset};
use vtt_core::meeting::{MeetingKind, MeetingRecord};
use vtt_core::usage::Usage;
use vtt_core::{Cleaned, Config, Directives, PostProcessor};
use vtt_llm::{
    ApiKeyProvider, DEFAULT_CLEANUP_PROMPT, DEFAULT_COMMAND_PROMPT, DEFAULT_CONFERENCE_PROMPT,
    DEFAULT_REPORT_STYLE_PROMPT, DEFAULT_SUMMARY_PROMPT, LlmCleaner, Prompts,
};

use crate::controller::ControllerState;
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};

const SAMPLE_DICTATION: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";
const SAMPLE_INSTRUCTION: &str = "rends ce texte plus formel";
const SAMPLE_SELECTION: &str = "salut, la réu c'est demain 14h, ramène le dossier stp";
const SAMPLE_CONFERENCE_TITLE: &str = "Le sommeil et la mémoire";
const SAMPLE_CONFERENCE: &str = "\
[00:00] Eux : Bonsoir à tous. Ce soir, je vais vous montrer pourquoi le sommeil est indispensable à la mémoire.
[01:10] Eux : Première partie : pendant le sommeil profond, le cerveau rejoue les souvenirs de la journée.
[04:30] Eux : Une étude de 2019 montre qu'une nuit blanche réduit de 40 % la capacité à mémoriser.
[07:45] Eux : Deuxième partie : les siestes de vingt minutes améliorent l'apprentissage.
[10:20] Eux : En résumé, dormir, c'est aussi apprendre. Place aux questions.";
const SAMPLE_MEETING_TITLE: &str = "Point projet";
const SAMPLE_MEETING: &str = "\
[00:00] Moi : Bonjour à tous, on fait le point sur le site. Le lancement est toujours prévu le 15 ?
[00:06] Eux : Oui, mais il manque encore les traductions. Julie s'en occupe d'ici vendredi.
[00:14] Moi : Très bien. Et le budget de la campagne, on le valide ?
[00:18] Eux : Validé à 5 000 euros. Par contre, on ne sait pas encore qui gère les réseaux sociaux.";

#[derive(Serialize)]
pub struct Defaults {
    cleanup: &'static str,
    command: &'static str,
    summary: &'static str,
    conference: &'static str,
    report: &'static str,
}

#[derive(Serialize)]
pub struct PromptsView {
    prompts: Vec<PromptPreset>,
    defaults: Defaults,
    /// Noms des prompts actifs (vide : intégré).
    cleanup_active: String,
    command_active: String,
    summary_active: String,
    conference_active: String,
    report_active: String,
}

fn build_prompts_view(config: &Config) -> PromptsView {
    PromptsView {
        prompts: config.prompts.clone(),
        defaults: Defaults {
            cleanup: DEFAULT_CLEANUP_PROMPT,
            command: DEFAULT_COMMAND_PROMPT,
            summary: DEFAULT_SUMMARY_PROMPT,
            conference: DEFAULT_CONFERENCE_PROMPT,
            report: DEFAULT_REPORT_STYLE_PROMPT,
        },
        cleanup_active: config.llm.cleanup_prompt.clone(),
        command_active: config.llm.command_prompt.clone(),
        summary_active: config.meetings.summary_prompt.clone(),
        conference_active: config.meetings.conference_prompt.clone(),
        report_active: config.meetings.report_prompt.clone(),
    }
}

#[tauri::command]
pub fn prompts_view(config: State<'_, SharedConfig>) -> PromptsView {
    build_prompts_view(&settings::read(&config))
}

fn save_prompts_to_disk(
    paths: &Paths,
    prompts: &[PromptPreset],
    cleanup_active: &str,
    command_active: &str,
    summary_active: &str,
    conference_active: &str,
    report_active: &str,
) -> Result<Config, String> {
    let (mut updated, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    updated.prompts = config::clean_prompts(prompts)?;
    updated.llm.cleanup_prompt = cleanup_active.trim().to_owned();
    updated.llm.command_prompt = command_active.trim().to_owned();
    updated.meetings.summary_prompt = summary_active.trim().to_owned();
    updated.meetings.conference_prompt = conference_active.trim().to_owned();
    updated.meetings.report_prompt = report_active.trim().to_owned();
    updated.check_prompt_refs(&updated.llm)?;
    updated.check_summary_ref()?;
    paths.write_config(&updated)?;
    Ok(updated)
}

/// Enregistre les prompts et le choix des prompts actifs, puis les applique.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_prompts(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    controller: State<'_, ControllerState>,
    prompts: Vec<PromptPreset>,
    cleanup_active: String,
    command_active: String,
    summary_active: String,
    conference_active: String,
    report_active: Option<String>,
) -> Result<(), String> {
    let updated = save_prompts_to_disk(
        &paths,
        &prompts,
        &cleanup_active,
        &command_active,
        &summary_active,
        &conference_active,
        report_active.as_deref().unwrap_or(""),
    )?;
    controller.apply(&updated);
    log::info!("{} prompt(s) enregistré(s)", updated.prompts.len());
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated;
    Ok(())
}

#[derive(Serialize)]
pub struct PromptTest {
    ok: bool,
    input: String,
    output: Option<String>,
    error: Option<String>,
    duration_ms: u128,
    usage: Option<Usage>,
    cost: Option<f64>,
    currency: String,
}

fn prompts_override(kind: PromptKind, text: String) -> Prompts {
    match kind {
        PromptKind::Nettoyage => Prompts {
            cleanup: Some(text),
            ..Prompts::default()
        },
        PromptKind::Commande => Prompts {
            command: Some(text),
            ..Prompts::default()
        },
        PromptKind::CompteRendu => Prompts {
            summary: Some(text),
            ..Prompts::default()
        },
        PromptKind::Conference => Prompts {
            conference: Some(text),
            ..Prompts::default()
        },
        PromptKind::Rapport => Prompts {
            report_style: Some(text),
            ..Prompts::default()
        },
    }
}

fn run_prompt_test(
    llm: &LlmConfig,
    dictionary: &[String],
    api_key: ApiKeyProvider,
    ledger: &LedgerStore,
    kind: PromptKind,
    text: String,
) -> Result<PromptTest, String> {
    if text.trim().is_empty() {
        return Err("prompt vide".into());
    }
    let prompts = prompts_override(kind, text);
    let cleaner = LlmCleaner::new(llm, dictionary, &prompts, api_key);
    let started = Instant::now();
    let (input, result) = match kind {
        PromptKind::Nettoyage => (
            SAMPLE_DICTATION.to_owned(),
            cleaner.process(SAMPLE_DICTATION, Directives::default()),
        ),
        PromptKind::Commande => (
            format!("Consigne : {SAMPLE_INSTRUCTION}\nSélection : {SAMPLE_SELECTION}"),
            cleaner.command(SAMPLE_INSTRUCTION, Some(SAMPLE_SELECTION)),
        ),
        PromptKind::CompteRendu => (
            format!("Réunion : {SAMPLE_MEETING_TITLE}\n{SAMPLE_MEETING}"),
            cleaner.summarize(SAMPLE_MEETING_TITLE, SAMPLE_MEETING, None, false),
        ),
        PromptKind::Conference => (
            format!("Conférence : {SAMPLE_CONFERENCE_TITLE}\n{SAMPLE_CONFERENCE}"),
            cleaner.summarize(SAMPLE_CONFERENCE_TITLE, SAMPLE_CONFERENCE, None, true),
        ),
        PromptKind::Rapport => (
            format!("Réunion : {SAMPLE_MEETING_TITLE}\n{SAMPLE_MEETING}"),
            cleaner
                .report(
                    SAMPLE_MEETING_TITLE,
                    SAMPLE_MEETING,
                    25_000,
                    None,
                    false,
                    &mut |_, _| {},
                )
                .map(|drafted| {
                    let mut preview = MeetingRecord::new(
                        "exemple".to_owned(),
                        SAMPLE_MEETING_TITLE.to_owned(),
                        0,
                        MeetingKind::Reunion,
                    );
                    preview.duration_ms = 25_000;
                    preview.report = Some(drafted.report);
                    let rendered = preview.report_markdown("Exemple", true).unwrap_or_default();
                    Cleaned {
                        text: rendered,
                        usage: drafted.usage,
                        fallback: None,
                    }
                }),
        ),
    };
    let duration_ms = started.elapsed().as_millis();
    let mut test = PromptTest {
        ok: result.is_ok(),
        input,
        output: None,
        error: None,
        duration_ms,
        usage: None,
        cost: None,
        currency: llm.pricing.currency.clone(),
    };
    match result {
        Ok(cleaned) => {
            if let Some(usage) = &cleaned.usage {
                costs::record(ledger, None, usage, &llm.pricing);
            }
            test.cost = cleaned.usage.map(|u| llm.pricing.cost(&u));
            test.usage = cleaned.usage;
            test.output = Some(cleaned.text);
        }
        Err(e) => test.error = Some(e.to_string()),
    }
    Ok(test)
}

/// Essaie `text` (non enregistré) avec le fournisseur LLM actif, sur une
/// dictée ou une commande d'exemple. Facturé et compté comme un test.
#[tauri::command(async)]
pub fn test_prompt(
    config: State<'_, SharedConfig>,
    ledger: State<'_, LedgerStore>,
    kind: PromptKind,
    text: String,
) -> Result<PromptTest, String> {
    let (llm, dictionary) = {
        let config = settings::read(&config);
        (config.llm.clone(), config.dictionary.clone())
    };
    let api_key = settings::api_key_provider(&llm);
    run_prompt_test(&llm, &dictionary, api_key, &ledger, kind, text)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use vtt_core::usage::Ledger;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vtt-prompts-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn serve_once(status: u16, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
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
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn build_prompts_view_exposes_defaults_and_active_refs() {
        let mut config = Config::default();
        config.llm.cleanup_prompt = "Concis".into();
        config.meetings.summary_prompt = "Court".into();
        config.meetings.report_prompt = "Exécutif".into();
        let view = build_prompts_view(&config);
        assert_eq!(view.defaults.cleanup, DEFAULT_CLEANUP_PROMPT);
        assert_eq!(view.defaults.command, DEFAULT_COMMAND_PROMPT);
        assert_eq!(view.defaults.summary, DEFAULT_SUMMARY_PROMPT);
        assert_eq!(view.defaults.conference, DEFAULT_CONFERENCE_PROMPT);
        assert_eq!(view.defaults.report, DEFAULT_REPORT_STYLE_PROMPT);
        assert_eq!(view.cleanup_active, "Concis");
        assert_eq!(view.summary_active, "Court");
        assert_eq!(view.report_active, "Exécutif");
        assert!(view.command_active.is_empty());
        assert!(view.conference_active.is_empty());
    }

    #[test]
    fn save_prompts_to_disk_persists_valid_and_rejects_broken_refs() {
        let dir = temp_dir("save");
        let paths = Paths::for_tests(&dir);
        let presets = vec![
            PromptPreset {
                name: "Concis".into(),
                kind: PromptKind::Nettoyage,
                text: "Nettoie de façon concise.".into(),
            },
            PromptPreset {
                name: "Synthèse".into(),
                kind: PromptKind::CompteRendu,
                text: "Fais un résumé court.".into(),
            },
            PromptPreset {
                name: "Direction".into(),
                kind: PromptKind::Rapport,
                text: "Style direction exécutive.".into(),
            },
        ];

        let saved = save_prompts_to_disk(
            &paths,
            &presets,
            " Concis ",
            "",
            "Synthèse",
            "",
            "Direction",
        )
        .unwrap();
        assert_eq!(saved.prompts.len(), 3);
        assert_eq!(saved.llm.cleanup_prompt, "Concis");
        assert_eq!(saved.meetings.summary_prompt, "Synthèse");
        assert_eq!(saved.meetings.report_prompt, "Direction");

        let (reloaded, err) = paths.load_config();
        assert!(err.is_none());
        assert_eq!(reloaded.llm.cleanup_prompt, "Concis");
        assert_eq!(reloaded.meetings.summary_prompt, "Synthèse");
        assert_eq!(reloaded.meetings.report_prompt, "Direction");

        // Référence vers un prompt inconnu refusée sans écraser config.toml.
        assert!(save_prompts_to_disk(&paths, &presets, "Inconnu", "", "", "", "").is_err());
        assert!(save_prompts_to_disk(&paths, &presets, "", "", "Inconnu", "", "").is_err());
        assert!(save_prompts_to_disk(&paths, &presets, "", "", "", "", "Inconnu").is_err());

        // config.toml corrompu : refusé avant écriture.
        fs::write(&paths.config_file, "invalide = = =").unwrap();
        assert!(save_prompts_to_disk(&paths, &presets, "Concis", "", "", "", "").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompts_override_targets_only_requested_kind() {
        let cleanup = prompts_override(PromptKind::Nettoyage, "A".into());
        assert_eq!(cleanup.cleanup.as_deref(), Some("A"));
        assert!(cleanup.command.is_none());
        assert!(cleanup.summary.is_none());
        assert!(cleanup.conference.is_none());
        assert!(cleanup.report_style.is_none());

        let conference = prompts_override(PromptKind::Conference, "B".into());
        assert_eq!(conference.conference.as_deref(), Some("B"));
        assert!(conference.cleanup.is_none());

        let report = prompts_override(PromptKind::Rapport, "C".into());
        assert_eq!(report.report_style.as_deref(), Some("C"));
        assert!(report.summary.is_none());
    }

    #[test]
    fn run_prompt_test_rejects_empty_text() {
        let dir = temp_dir("empty");
        let ledger = Arc::new(Mutex::new(Ledger::open(dir.join("usage.json"))));
        let err = run_prompt_test(
            &LlmConfig::default(),
            &[],
            Box::new(|| None),
            &ledger,
            PromptKind::Nettoyage,
            "   ".into(),
        )
        .err()
        .unwrap();
        assert!(err.contains("vide"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_prompt_test_records_usage_and_cost_on_success() {
        let dir = temp_dir("ok");
        let ledger = Arc::new(Mutex::new(Ledger::open(dir.join("usage.json"))));
        let (url, server) = serve_once(
            200,
            r#"{"choices":[{"message":{"content":"Bonjour, la réunion est mercredi à 14h."}}],"usage":{"prompt_tokens":100,"completion_tokens":20}}"#,
        );
        let llm = LlmConfig {
            base_url: url,
            ..LlmConfig::default()
        };

        let result = run_prompt_test(
            &llm,
            &[],
            Box::new(|| None),
            &ledger,
            PromptKind::Nettoyage,
            "Consigne de test".into(),
        )
        .unwrap();
        let req = server.join().unwrap();
        assert!(req.contains("Consigne de test"));
        assert!(result.ok);
        assert_eq!(
            result.output.as_deref(),
            Some("Bonjour, la réunion est mercredi à 14h.")
        );
        assert!(result.error.is_none());
        assert!(result.cost.is_some());
        assert_eq!(costs::summary(&ledger).today.requests, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_prompt_test_reports_llm_error_without_recording_usage() {
        let dir = temp_dir("err");
        let ledger = Arc::new(Mutex::new(Ledger::open(dir.join("usage.json"))));
        let (url, server) = serve_once(500, r#"{"error":{"message":"serveur HS"}}"#);
        let llm = LlmConfig {
            base_url: url,
            ..LlmConfig::default()
        };

        let result = run_prompt_test(
            &llm,
            &[],
            Box::new(|| None),
            &ledger,
            PromptKind::Commande,
            "Consigne commande".into(),
        )
        .unwrap();
        let _ = server.join().unwrap();
        assert!(!result.ok);
        assert!(result.output.is_none());
        assert!(result.error.is_some());
        assert_eq!(costs::summary(&ledger).today.requests, 0);

        let _ = fs::remove_dir_all(&dir);
    }
}
