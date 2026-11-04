//! Page Prompts : prompts système personnalisés (nettoyage, Command Mode,
//! compte rendu de réunion),
//! choix du prompt actif, test. Appliqués à chaud.

use std::time::Instant;

use serde::Serialize;
use tauri::State;
use vtt_core::config::{self, PromptKind, PromptPreset};
use vtt_core::usage::Usage;
use vtt_core::{Directives, PostProcessor};
use vtt_llm::{
    DEFAULT_CLEANUP_PROMPT, DEFAULT_COMMAND_PROMPT, DEFAULT_SUMMARY_PROMPT, LlmCleaner, Prompts,
};

use crate::controller::ControllerState;
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};

const SAMPLE_DICTATION: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";
const SAMPLE_INSTRUCTION: &str = "rends ce texte plus formel";
const SAMPLE_SELECTION: &str = "salut, la réu c'est demain 14h, ramène le dossier stp";
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
}

#[derive(Serialize)]
pub struct PromptsView {
    prompts: Vec<PromptPreset>,
    defaults: Defaults,
    /// Noms des prompts actifs (vide : intégré).
    cleanup_active: String,
    command_active: String,
    summary_active: String,
}

#[tauri::command]
pub fn prompts_view(config: State<'_, SharedConfig>) -> PromptsView {
    let config = settings::read(&config);
    PromptsView {
        prompts: config.prompts.clone(),
        defaults: Defaults {
            cleanup: DEFAULT_CLEANUP_PROMPT,
            command: DEFAULT_COMMAND_PROMPT,
            summary: DEFAULT_SUMMARY_PROMPT,
        },
        cleanup_active: config.llm.cleanup_prompt.clone(),
        command_active: config.llm.command_prompt.clone(),
        summary_active: config.meetings.summary_prompt.clone(),
    }
}

/// Enregistre les prompts et le choix des prompts actifs, puis les applique.
#[tauri::command]
pub fn save_prompts(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    controller: State<'_, ControllerState>,
    prompts: Vec<PromptPreset>,
    cleanup_active: String,
    command_active: String,
    summary_active: String,
) -> Result<(), String> {
    let (mut updated, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    updated.prompts = config::clean_prompts(&prompts)?;
    updated.llm.cleanup_prompt = cleanup_active.trim().to_owned();
    updated.llm.command_prompt = command_active.trim().to_owned();
    updated.meetings.summary_prompt = summary_active.trim().to_owned();
    updated.check_prompt_refs(&updated.llm)?;
    updated.check_summary_ref()?;
    paths.write_config(&updated)?;
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

/// Essaie `text` (non enregistré) avec le fournisseur LLM actif, sur une
/// dictée ou une commande d'exemple. Facturé et compté comme un test.
#[tauri::command(async)]
pub fn test_prompt(
    config: State<'_, SharedConfig>,
    ledger: State<'_, LedgerStore>,
    kind: PromptKind,
    text: String,
) -> Result<PromptTest, String> {
    if text.trim().is_empty() {
        return Err("prompt vide".into());
    }
    let (llm, dictionary) = {
        let config = settings::read(&config);
        (config.llm.clone(), config.dictionary.clone())
    };
    let prompts = match kind {
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
    };
    let cleaner = LlmCleaner::new(
        &llm,
        &dictionary,
        &prompts,
        settings::api_key_provider(&llm),
    );
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
            cleaner.summarize(SAMPLE_MEETING_TITLE, SAMPLE_MEETING, None),
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
                costs::record(&ledger, None, usage, &llm.pricing);
            }
            test.cost = cleaned.usage.map(|u| llm.pricing.cost(&u));
            test.usage = cleaned.usage;
            test.output = Some(cleaned.text);
        }
        Err(e) => test.error = Some(e.to_string()),
    }
    Ok(test)
}
