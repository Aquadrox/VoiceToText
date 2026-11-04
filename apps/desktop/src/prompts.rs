//! Page Prompts : prompts système personnalisés (nettoyage et Command Mode),
//! choix du prompt actif, test. Appliqués à chaud.

use std::time::Instant;

use serde::Serialize;
use tauri::State;
use vtt_core::PostProcessor;
use vtt_core::config::{self, PromptKind, PromptPreset};
use vtt_core::usage::Usage;
use vtt_llm::{DEFAULT_CLEANUP_PROMPT, DEFAULT_COMMAND_PROMPT, LlmCleaner, Prompts};

use crate::controller::ControllerState;
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};

const SAMPLE_DICTATION: &str =
    "euh bonjour, euh je voulais te dire que la réunion c'est mardi, non pardon mercredi à 14h";
const SAMPLE_INSTRUCTION: &str = "rends ce texte plus formel";
const SAMPLE_SELECTION: &str = "salut, la réu c'est demain 14h, ramène le dossier stp";

#[derive(Serialize)]
pub struct Defaults {
    cleanup: &'static str,
    command: &'static str,
}

#[derive(Serialize)]
pub struct PromptsView {
    prompts: Vec<PromptPreset>,
    defaults: Defaults,
    /// Noms des prompts actifs (vide : intégré).
    cleanup_active: String,
    command_active: String,
}

#[tauri::command]
pub fn prompts_view(config: State<'_, SharedConfig>) -> PromptsView {
    let config = settings::read(&config);
    PromptsView {
        prompts: config.prompts.clone(),
        defaults: Defaults {
            cleanup: DEFAULT_CLEANUP_PROMPT,
            command: DEFAULT_COMMAND_PROMPT,
        },
        cleanup_active: config.llm.cleanup_prompt.clone(),
        command_active: config.llm.command_prompt.clone(),
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
    updated.check_prompt_refs(&updated.llm)?;
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
            command: None,
        },
        PromptKind::Commande => Prompts {
            cleanup: None,
            command: Some(text),
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
            cleaner.process(SAMPLE_DICTATION, None),
        ),
        PromptKind::Commande => (
            format!("Consigne : {SAMPLE_INSTRUCTION}\nSélection : {SAMPLE_SELECTION}"),
            cleaner.command(SAMPLE_INSTRUCTION, Some(SAMPLE_SELECTION)),
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
                costs::record(&ledger, usage, &llm.pricing);
            }
            test.cost = cleaned.usage.map(|u| llm.pricing.cost(&u));
            test.usage = cleaned.usage;
            test.output = Some(cleaned.text);
        }
        Err(e) => test.error = Some(e.to_string()),
    }
    Ok(test)
}
