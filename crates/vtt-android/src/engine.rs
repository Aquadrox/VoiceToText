//! Moteur de dictée du téléphone : Whisper + pipeline commun (snippets,
//! commandes vocales, nettoyage LLM). Le texte est rendu à l'application,
//! qui l'insère dans le champ en cours (clavier Android).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use vtt_core::config::{LlmConfig, PromptKind};
use vtt_core::{AudioClip, Config, Error, Outcome, Pipeline, PostProcessor, Target, TextSink};
use vtt_llm::{LlmCleaner, Prompts};

use crate::{models, settings};

/// Clé API du fournisseur actif, gardée en mémoire seulement (l'application
/// la conserve chiffrée par le Keystore Android).
static API_KEY: Mutex<Option<String>> = Mutex::new(None);

struct Engine {
    pipeline: Pipeline,
    model: String,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_api_key(key: Option<String>) {
    *lock(&API_KEY) = key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
}

/// L'application insère elle-même le texte : la sortie du pipeline est
/// ignorée, l'annulation est signalée dans le résultat.
struct Collect;

impl TextSink for Collect {
    fn insert(&self, _text: &str) -> Result<(), Error> {
        Ok(())
    }

    fn undo(&self) -> Result<(), Error> {
        Ok(())
    }
}

fn cleaner(config: &Config) -> Option<Box<dyn PostProcessor>> {
    if !config.llm.enabled {
        return None;
    }
    let llm: &LlmConfig = &config.llm;
    let text = |kind, name: &str| config.prompt_text(kind, name).map(str::to_owned);
    let prompts = Prompts {
        cleanup: text(PromptKind::Nettoyage, &llm.cleanup_prompt),
        command: text(PromptKind::Commande, &llm.command_prompt),
        ..Prompts::default()
    };
    Some(Box::new(LlmCleaner::new(
        llm,
        &config.dictionary,
        &prompts,
        Box::new(|| lock(&API_KEY).clone()),
    )))
}

/// (Re)démarre le moteur avec ces réglages ; le modèle n'est rechargé que
/// s'il a changé.
pub fn start(models_dir: &Path, toml: &str) -> Result<(), String> {
    let (config, _) = settings::parse(toml)?;
    let path = models::local(models_dir, &config.asr.model)
        .ok_or_else(|| format!("modèle {} non téléchargé", config.asr.model))?;
    let mut engine = lock(&ENGINE);
    if let Some(engine) = engine.as_mut()
        && engine.model == config.asr.model
    {
        engine.pipeline.reconfigure(
            &config.dictionary,
            &config.snippets,
            &config.styles,
            cleaner(&config),
        );
        engine
            .pipeline
            .set_voice_commands(config.dictation.voice_commands);
        log::info!("réglages appliqués");
        return Ok(());
    }
    // Téléphone : processeur seul. Quatre threads (les cœurs rapides ; les
    // cœurs économes ralentiraient l'ensemble), encodeur limité à la durée de
    // la dictée.
    let transcriber = vtt_asr::WhisperTranscriber::load_with(
        &path,
        &config.asr.language,
        false,
        &config.dictionary,
        vtt_asr::WhisperOptions {
            threads: Some(4),
            short_context: true,
            flash_attn: true,
        },
    )
    .map_err(|e| e.to_string())?;
    let pipeline = Pipeline::new(Box::new(transcriber), cleaner(&config), Box::new(Collect))
        .with_snippets(&config.snippets)
        .with_styles(&config.styles)
        .with_voice_commands(config.dictation.voice_commands);
    *engine = Some(Engine {
        pipeline,
        model: config.asr.model.clone(),
    });
    log::info!("moteur prêt : {}", config.asr.model);
    Ok(())
}

/// Résultat d'une dictée pour l'application.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Dictation {
    /// « text » (insérer `text`), « undo » (effacer la dernière dictée),
    /// « skip » (rien entendu) ou « error ».
    pub kind: &'static str,
    pub text: String,
    pub raw: String,
    /// Nettoyage échoué (texte brut inséré) ou erreur.
    pub error: Option<String>,
    /// Durée du traitement (transcription et nettoyage), en millisecondes.
    pub ms: u64,
}

pub fn outcome(result: Result<Outcome, Error>) -> Dictation {
    match result {
        Ok(Outcome::Inserted {
            raw,
            text,
            post_process_error,
            ..
        }) => Dictation {
            kind: "text",
            text,
            raw,
            error: post_process_error,
            ms: 0,
        },
        Ok(Outcome::Undone) => Dictation {
            kind: "undo",
            ..Dictation::default()
        },
        Ok(Outcome::Skipped) => Dictation {
            kind: "skip",
            ..Dictation::default()
        },
        Err(e) => Dictation {
            kind: "error",
            error: Some(e.to_string()),
            ..Dictation::default()
        },
    }
}

/// Dicte `samples` (16 kHz mono) ; `app` : application de destination
/// (styles par application).
pub fn dictate(samples: Vec<f32>, app: Option<String>) -> Dictation {
    let engine = lock(&ENGINE);
    let Some(engine) = engine.as_ref() else {
        return Dictation {
            kind: "error",
            error: Some("moteur non démarré : téléchargez un modèle".into()),
            ..Dictation::default()
        };
    };
    let target = Target { app, title: None };
    let started = Instant::now();
    let mut result = outcome(engine.pipeline.run(&AudioClip { samples }, &target));
    result.ms = started.elapsed().as_millis() as u64;
    log::info!("dictée traitée en {} ms ({})", result.ms, result.kind);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_become_simple_results() {
        let inserted = Outcome::Inserted {
            raw: "euh bonjour".into(),
            text: "Bonjour.".into(),
            post_process_error: None,
            snippet: false,
            usage: None,
            command: false,
            fallback: None,
            translated: None,
        };
        assert_eq!(
            outcome(Ok(inserted)),
            Dictation {
                kind: "text",
                text: "Bonjour.".into(),
                raw: "euh bonjour".into(),
                error: None,
                ms: 0
            }
        );
        assert_eq!(outcome(Ok(Outcome::Undone)).kind, "undo");
        assert_eq!(outcome(Ok(Outcome::Skipped)).kind, "skip");
        assert_eq!(outcome(Err(Error::Asr("x".into()))).kind, "error");
        assert_eq!(
            dictate(vec![0.0; 16_000], None).kind,
            "error",
            "pas de moteur"
        );
        set_api_key(Some("  ".into()));
        assert_eq!(*lock(&API_KEY), None);
    }
}
