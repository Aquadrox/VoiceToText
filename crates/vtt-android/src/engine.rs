//! Moteur de dictée du téléphone : Whisper + pipeline commun (snippets,
//! commandes vocales, nettoyage LLM, traduction, Command Mode). Le texte est
//! rendu à l'application, qui l'insère dans le champ en cours (clavier
//! Android) ; l'historique reste sur le téléphone.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vtt_core::config::{LlmConfig, Pricing, PromptKind};
use vtt_core::history::{Entry, History};
use vtt_core::{AudioClip, Config, Error, Outcome, Pipeline, PostProcessor, Target, TextSink};
use vtt_llm::{LlmCleaner, Prompts};

use crate::{models, settings};

/// Clé API du fournisseur actif, gardée en mémoire seulement (l'application
/// la conserve chiffrée par le Keystore Android).
static API_KEY: Mutex<Option<String>> = Mutex::new(None);

struct Engine {
    pipeline: Pipeline,
    model: String,
    /// `None` : historique désactivé dans les réglages.
    history: Option<History>,
    pricing: Pricing,
    /// Langue du mode traduction.
    translate_to: String,
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
/// s'il a changé. `history_file` : historique des dictées sur le téléphone.
pub fn start(models_dir: &Path, history_file: &Path, toml: &str) -> Result<(), String> {
    let (config, _) = settings::parse(toml)?;
    let history = config
        .history
        .enabled
        .then(|| History::open(history_file.to_owned(), config.history.max_entries));
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
        engine.history = history;
        engine.pricing = config.llm.pricing.clone();
        engine.translate_to = config.translation.language.clone();
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
        history,
        pricing: config.llm.pricing.clone(),
        translate_to: config.translation.language.clone(),
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

/// Discrétion demandée par le champ où l'on écrit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    Normal,
    /// Navigation privée, application sensible (« ne pas apprendre ») :
    /// rien dans l'historique.
    NoHistory,
    /// Champ de mot de passe : transcription seule, sans LLM ni historique.
    Password,
}

/// Ce que l'utilisateur dicte.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Dictate,
    /// Mode traduction : texte nettoyé puis traduit.
    Translate,
    /// Command Mode : consigne appliquée au texte sélectionné (ou rédaction
    /// s'il n'y en a pas).
    Command(Option<String>),
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Entrée d'historique d'une dictée insérée (comme sur PC).
fn entry(
    outcome: &Outcome,
    clip: &AudioClip,
    app: Option<String>,
    pricing: &Pricing,
) -> Option<Entry> {
    let Outcome::Inserted {
        raw,
        text,
        post_process_error,
        snippet,
        command,
        usage,
        fallback,
        translated,
    } = outcome
    else {
        return None;
    };
    Some(Entry {
        at_ms: now_ms(),
        duration_ms: clip.duration_ms(),
        raw: raw.clone(),
        text: text.clone(),
        snippet: *snippet,
        cleanup_failed: post_process_error.is_some(),
        command: *command,
        app,
        usage: *usage,
        cost: usage.map(|u| pricing.cost(&u)),
        currency: usage.map(|_| pricing.currency.clone()),
        fallback: fallback.clone(),
        translated: translated.clone(),
    })
}

/// Dicte `samples` (16 kHz mono) ; `app` : application de destination
/// (styles par application).
pub fn dictate(samples: Vec<f32>, app: Option<String>, mode: Mode, privacy: Privacy) -> Dictation {
    let mut engine = lock(&ENGINE);
    let Some(engine) = engine.as_mut() else {
        return Dictation {
            kind: "error",
            error: Some("moteur non démarré : téléchargez un modèle".into()),
            ..Dictation::default()
        };
    };
    let target = Target {
        app: app.clone(),
        title: None,
    };
    let clip = AudioClip { samples };
    let started = Instant::now();
    if privacy == Privacy::Password {
        // Rien ne quitte le téléphone, rien n'est gardé.
        let mut result = match engine.pipeline.transcribe_only(&clip) {
            Ok(Some(text)) => Dictation {
                kind: "text",
                raw: text.clone(),
                text,
                ..Dictation::default()
            },
            Ok(None) => outcome(Ok(Outcome::Skipped)),
            Err(e) => outcome(Err(e)),
        };
        result.ms = started.elapsed().as_millis() as u64;
        return result;
    }
    let raw = match &mode {
        Mode::Dictate => engine.pipeline.run(&clip, &target),
        Mode::Translate => {
            engine
                .pipeline
                .dictate(&clip, &target, None, Some(engine.translate_to.as_str()))
        }
        Mode::Command(selection) => {
            engine
                .pipeline
                .run_command(&clip, &target, selection.as_deref())
        }
    };
    if privacy == Privacy::Normal
        && let (Ok(outcome), Some(history)) = (&raw, engine.history.as_mut())
        && let Some(entry) = entry(outcome, &clip, app, &engine.pricing)
        && let Err(e) = history.append(&entry)
    {
        log::warn!("historique : {e}");
    }
    let mut result = outcome(raw);
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
            dictate(vec![0.0; 16_000], None, Mode::Dictate, Privacy::Normal).kind,
            "error",
            "pas de moteur"
        );
        set_api_key(Some("  ".into()));
        assert_eq!(*lock(&API_KEY), None);
    }
}

/// Dictées récentes (la plus récente d'abord).
pub fn history(file: &Path, limit: usize) -> Result<Vec<Entry>, String> {
    History::open(file.to_owned(), usize::MAX)
        .recent(limit)
        .map_err(|e| format!("historique : {e}"))
}

pub fn clear_history(file: &Path) -> Result<(), String> {
    History::open(file.to_owned(), usize::MAX)
        .clear()
        .map_err(|e| format!("historique : {e}"))
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn inserted_dictations_become_history_entries() {
        let inserted = Outcome::Inserted {
            raw: "envoie le jason".into(),
            text: "Envoie le JSON.".into(),
            post_process_error: None,
            snippet: false,
            usage: Some(vtt_core::usage::Usage {
                input_tokens: 1_000_000,
                cached_input_tokens: 0,
                output_tokens: 0,
            }),
            command: false,
            fallback: None,
            translated: Some("anglais".into()),
        };
        let clip = AudioClip {
            samples: vec![0.0; 32_000],
        };
        let pricing = Pricing {
            input_per_million: 0.3,
            ..Pricing::default()
        };
        let e = entry(&inserted, &clip, Some("com.whatsapp".into()), &pricing).unwrap();
        assert_eq!((e.duration_ms, e.text.as_str()), (2_000, "Envoie le JSON."));
        assert_eq!(e.app.as_deref(), Some("com.whatsapp"));
        assert!((e.cost.unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(e.translated.as_deref(), Some("anglais"));
        assert!(entry(&Outcome::Skipped, &clip, None, &pricing).is_none());

        let dir = std::env::temp_dir().join(format!("vtt-android-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history.jsonl");
        History::open(file.clone(), 10).append(&e).unwrap();
        assert_eq!(history(&file, 5).unwrap().len(), 1);
        clear_history(&file).unwrap();
        assert!(history(&file, 5).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
