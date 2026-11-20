//! Moteur de dictée du téléphone : Whisper, Parakeet ou Vosk + pipeline
//! commun (snippets,
//! commandes vocales, nettoyage LLM, traduction, Command Mode). Le texte est
//! rendu à l'application, qui l'insère dans le champ en cours (clavier
//! Android) ; l'historique reste sur le téléphone.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vtt_core::config::{DictationConfig, LlmConfig, Pricing, PromptKind};
use vtt_core::context;
use vtt_core::history::{Entry, History};
use vtt_core::live::{DEFAULT_PREVIEW_CHARS, LiveSession};
use vtt_core::usage::{Ledger, day_from_unix_ms};
use vtt_core::{
    AudioClip, Config, Error, Outcome, Pipeline, PostProcessor, Target, TextSink, Transcriber,
};
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
    /// Compteurs d'activité et coût LLM (toujours actifs, sans aucun texte
    /// dicté, synchronisés avec le PC et les autres appareils).
    ledger: Option<Ledger>,
    pricing: Pricing,
    /// Langue du mode traduction.
    translate_to: String,
    /// Options de dictée (contexte actif opt-in et applications exclues).
    dictation: DictationConfig,
    /// Dictionnaire perso pour éviter les doublons lors de l'extraction de contexte.
    dictionary: Vec<String>,
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

/// Charge le modèle avec son moteur. ONNX Runtime (Parakeet) et libvosk
/// sont dans l'APK : trouvés par leur nom.
fn load(path: &Path, config: &Config) -> Result<Box<dyn Transcriber>, String> {
    let engine = models::known(&config.asr.model)
        .map(|m| m.engine)
        .ok_or_else(|| format!("modèle inconnu : {}", config.asr.model))?;
    let transcriber: Box<dyn Transcriber> = match engine {
        // Processeur seul. Quatre threads (les cœurs rapides ; les cœurs
        // économes ralentiraient l'ensemble), encodeur limité à la durée de
        // la dictée.
        models::Engine::Whisper => Box::new(
            vtt_asr::WhisperTranscriber::load_with(
                path,
                &config.asr.language,
                false,
                &config.dictionary,
                vtt_asr::WhisperOptions {
                    threads: Some(4),
                    short_context: true,
                    flash_attn: true,
                },
            )
            .map_err(|e| e.to_string())?,
        ),
        models::Engine::Parakeet => Box::new(
            vtt_asr::ParakeetTranscriber::load(path, Path::new("libonnxruntime.so"))
                .map_err(|e| e.to_string())?,
        ),
        models::Engine::Vosk => Box::new(
            vtt_asr::VoskTranscriber::load(path, Path::new("libvosk.so"))
                .map_err(|e| e.to_string())?,
        ),
    };
    Ok(transcriber)
}

/// (Re)démarre le moteur avec ces réglages ; le modèle n'est rechargé que
/// s'il a changé. `history_file` : historique des dictées sur le téléphone.
pub fn start(models_dir: &Path, history_file: &Path, toml: &str) -> Result<(), String> {
    let (config, _) = settings::parse(toml)?;
    let history = config
        .history
        .enabled
        .then(|| History::open(history_file.to_owned(), config.history.max_entries));
    let ledger = Some(Ledger::open(history_file.with_file_name("usage.json")));
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
        engine.ledger = ledger;
        engine.pricing = config.llm.pricing.clone();
        engine.translate_to = config.translation.language.clone();
        engine.dictation = config.dictation.clone();
        engine.dictionary = config.dictionary.clone();
        log::info!("réglages appliqués");
        return Ok(());
    }
    // Ancien modèle libéré avant de charger le nouveau : jamais deux en
    // mémoire à la fois.
    *engine = None;
    let transcriber = load(&path, &config)?;
    let pipeline = Pipeline::new(transcriber, cleaner(&config), Box::new(Collect))
        .with_snippets(&config.snippets)
        .with_styles(&config.styles)
        .with_voice_commands(config.dictation.voice_commands);
    *engine = Some(Engine {
        pipeline,
        model: config.asr.model.clone(),
        history,
        ledger,
        pricing: config.llm.pricing.clone(),
        translate_to: config.translation.language.clone(),
        dictation: config.dictation.clone(),
        dictionary: config.dictionary.clone(),
    });
    log::info!("moteur prêt : {}", config.asr.model);
    Ok(())
}

/// Modèle chargé (une transcription de réunion peut avancer).
pub fn loaded() -> bool {
    lock(&ENGINE).is_some()
}

/// Transcription seule d'un bloc de réunion (16 kHz mono), filtrée comme une
/// dictée ; `None` : silence ou hallucination. Le moteur n'est pris que le
/// temps du bloc : les dictées passent entre deux blocs.
pub fn transcribe(samples: Vec<f32>) -> Result<Option<String>, String> {
    let engine = lock(&ENGINE);
    let engine = engine
        .as_ref()
        .ok_or("moteur non démarré : modèle libéré ou pas encore chargé")?;
    engine
        .pipeline
        .transcribe_only(&AudioClip { samples })
        .map_err(|e| e.to_string())
}

/// Client LLM des comptes rendus de réunion (`None` : nettoyage par LLM
/// désactivé), avec les prompts de la configuration.
pub fn summarizer(config: &Config) -> Option<LlmCleaner> {
    if !config.llm.enabled {
        return None;
    }
    let text = |kind, name: &str| config.prompt_text(kind, name).map(str::to_owned);
    let prompts = Prompts {
        summary: text(PromptKind::CompteRendu, &config.meetings.summary_prompt),
        conference: text(PromptKind::Conference, &config.meetings.conference_prompt),
        ..Prompts::default()
    };
    Some(LlmCleaner::new(
        &config.llm,
        &config.dictionary,
        &prompts,
        Box::new(|| lock(&API_KEY).clone()),
    ))
}

/// Libère le modèle (plusieurs centaines de Mo de mémoire vive) : le
/// prochain [`start`] le recharge. Sert à la bulle quand l'écran est éteint,
/// pour que le téléphone ait moins de raisons d'arrêter l'application.
pub fn release() {
    live_cancel();
    *lock(&LIVE) = None;
    if lock(&ENGINE).take().is_some() {
        log::info!("modèle libéré");
    }
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

struct ActiveLive {
    id: u64,
    session: LiveSession,
    app: Option<String>,
    mode: Mode,
    privacy: Privacy,
    /// Termes extraits localement au démarrage (toujours vides hors `Privacy::Normal`).
    context_terms: Vec<String>,
    /// Première erreur ASR rencontrée en direct ; renvoyée à l'arrêt si aucune
    /// tranche n'a produit de texte exploitable.
    asr_error: Option<String>,
}

static LIVE: Mutex<Option<ActiveLive>> = Mutex::new(None);
static NEXT_LIVE_ID: AtomicU64 = AtomicU64::new(0);
static CANCELLED_LIVE_ID: AtomicU64 = AtomicU64::new(0);

/// Résultat d'un apport audio incrémental en mode mains libres.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct LiveFeed {
    /// Vrai si une nouvelle tranche a fait évoluer l'aperçu visible.
    pub changed: bool,
    /// Aperçu glissant UTF-8 des derniers mots transcrits (toujours vide en
    /// `Privacy::Password` ou tant qu'aucune tranche n'a produit de texte).
    pub preview: String,
    /// Nombre de tranches ayant déjà produit du texte exploitable.
    pub segments: usize,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn resolve_context_terms(
    engine: &Engine,
    target: &Target,
    mode: &Mode,
    privacy: Privacy,
    context_text: Option<&str>,
) -> Vec<String> {
    if privacy == Privacy::Normal
        && !matches!(mode, Mode::Command(_))
        && let Some(raw_context) = context_text.filter(|t| !t.trim().is_empty())
        && context::is_context_allowed(&engine.dictation, target, false)
    {
        let terms = context::extract_context_terms(raw_context, &engine.dictionary);
        if !terms.is_empty() {
            log::debug!("contexte actif : {} terme(s)", terms.len());
            return terms;
        }
    }
    Vec::new()
}

/// Entrée d'historique d'une dictée insérée (comme sur PC).
fn entry(
    outcome: &Outcome,
    clip: &AudioClip,
    app: Option<String>,
    pricing: &Pricing,
) -> Option<Entry> {
    entry_with_duration(outcome, clip.duration_ms(), app, pricing)
}

fn entry_with_duration(
    outcome: &Outcome,
    duration_ms: u64,
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
        duration_ms,
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

/// Enregistre les compteurs d'activité (mots, durée, commandes) et le coût LLM
/// dans `usage.json` / `usage-apps.json` (aucun texte dicté). Rien n'est
/// comptabilisé sur un champ de mot de passe ; en navigation privée
/// (`Privacy::NoHistory`), les compteurs globaux sont gardés sans nom
/// d'application.
fn record_stats(
    ledger: Option<&mut Ledger>,
    outcome: &Outcome,
    duration_ms: u64,
    app: Option<&str>,
    pricing: &Pricing,
    privacy: Privacy,
) {
    if privacy == Privacy::Password {
        return;
    }
    let Some(ledger) = ledger else {
        return;
    };
    let Outcome::Inserted {
        raw,
        usage,
        command,
        ..
    } = outcome
    else {
        return;
    };
    let day = day_from_unix_ms(now_ms());
    let app = (privacy == Privacy::Normal).then_some(app).flatten();
    if let Some(usage) = usage
        && let Err(e) = ledger.record(&day, app, usage, pricing)
    {
        log::warn!("statistiques LLM : {e}");
    }
    let words = raw.split_whitespace().count() as u64;
    if let Err(e) = ledger.record_dictation(&day, app, words, duration_ms, *command) {
        log::warn!("statistiques dictée : {e}");
    }
}

/// Dicte `samples` (16 kHz mono) ; `app` : application de destination
/// (styles par application). `context_text` : texte optionnel autour du curseur
/// (strictement ignoré hors `Privacy::Normal`, en Command Mode, si l'option
/// `dictation.screen_context` est désactivée ou si l'application est exclue).
pub fn dictate(
    samples: Vec<f32>,
    app: Option<String>,
    mode: Mode,
    privacy: Privacy,
    context_text: Option<&str>,
) -> Dictation {
    let mut engine = lock(&ENGINE);
    let Some(engine) = engine.as_mut() else {
        return Dictation {
            kind: "error",
            error: Some("moteur non démarré : téléchargez un modèle".into()),
            ..Dictation::default()
        };
    };
    let mut target = Target {
        app: app.clone(),
        ..Target::default()
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
    target.context_terms = resolve_context_terms(engine, &target, &mode, privacy, context_text);
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
    if let Ok(outcome) = &raw {
        record_stats(
            engine.ledger.as_mut(),
            outcome,
            clip.duration_ms(),
            app.as_deref(),
            &engine.pricing,
            privacy,
        );
    }
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

/// Démarre une session de dictée mains libres en continu ; renvoie son
/// identifiant. Le contexte autour du curseur n'est extrait qu'en
/// `Privacy::Normal`, jamais sur mot de passe ni en navigation privée.
pub fn live_start(
    app: Option<String>,
    mode: Mode,
    privacy: Privacy,
    context_text: Option<&str>,
) -> Result<u64, String> {
    if matches!(mode, Mode::Command(_)) {
        return Err("la prévisualisation en continu n'est pas disponible en Command Mode".into());
    }
    let context_terms = {
        let engine = lock(&ENGINE);
        let engine = engine
            .as_ref()
            .ok_or("moteur non démarré : téléchargez un modèle")?;
        let target = Target {
            app: app.clone(),
            ..Target::default()
        };
        resolve_context_terms(engine, &target, &mode, privacy, context_text)
    };
    let id = NEXT_LIVE_ID.fetch_add(1, Ordering::SeqCst) + 1;
    *lock(&LIVE) = Some(ActiveLive {
        id,
        session: LiveSession::new(),
        app,
        mode,
        privacy,
        context_terms,
        asr_error: None,
    });
    Ok(id)
}

/// Ajoute des échantillons 16 kHz mono à la session mains libres en cours,
/// transcrit les tranches prêtes et renvoie l'aperçu glissant (toujours vide
/// en `Privacy::Password`).
pub fn live_feed(samples: &[f32]) -> Result<LiveFeed, String> {
    let (id, privacy, hints, chunks, preview, segments) = {
        let mut guard = lock(&LIVE);
        let Some(active) = guard.as_mut() else {
            return Ok(LiveFeed::default());
        };
        if CANCELLED_LIVE_ID.load(Ordering::SeqCst) >= active.id {
            *guard = None;
            return Ok(LiveFeed::default());
        }
        let chunks = active.session.feed(samples);
        let hints = if active.privacy == Privacy::Password {
            Vec::new()
        } else {
            active.context_terms.clone()
        };
        let preview = if active.privacy == Privacy::Password {
            String::new()
        } else {
            active.session.preview(DEFAULT_PREVIEW_CHARS)
        };
        (
            active.id,
            active.privacy,
            hints,
            chunks,
            preview,
            active.session.segment_count(),
        )
    };

    if chunks.is_empty() {
        return Ok(LiveFeed {
            changed: false,
            preview,
            segments,
        });
    }

    let mut changed = false;
    for chunk in chunks {
        if CANCELLED_LIVE_ID.load(Ordering::SeqCst) >= id {
            let mut guard = lock(&LIVE);
            if guard.as_ref().is_some_and(|a| a.id == id) {
                *guard = None;
            }
            return Ok(LiveFeed::default());
        }
        let transcribed = {
            let engine = lock(&ENGINE);
            let engine = engine
                .as_ref()
                .ok_or("moteur non démarré : modèle libéré ou pas encore chargé")?;
            engine.pipeline.transcribe_chunk(&chunk, &hints)
        };
        if CANCELLED_LIVE_ID.load(Ordering::SeqCst) >= id {
            let mut guard = lock(&LIVE);
            if guard.as_ref().is_some_and(|a| a.id == id) {
                *guard = None;
            }
            return Ok(LiveFeed::default());
        }
        let mut guard = lock(&LIVE);
        let Some(active) = guard.as_mut().filter(|a| a.id == id) else {
            return Ok(LiveFeed::default());
        };
        match transcribed {
            Ok(Some((text, ms))) => {
                if active.session.push_text(&text, ms) && privacy != Privacy::Password {
                    changed = true;
                }
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("transcription d'une tranche en direct : {e}");
                if active.asr_error.is_none() {
                    active.asr_error = Some(e.to_string());
                }
            }
        }
    }

    let guard = lock(&LIVE);
    let Some(active) = guard.as_ref().filter(|a| a.id == id) else {
        return Ok(LiveFeed::default());
    };
    Ok(LiveFeed {
        changed,
        preview: if privacy == Privacy::Password {
            String::new()
        } else {
            active.session.preview(DEFAULT_PREVIEW_CHARS)
        },
        segments: active.session.segment_count(),
    })
}

/// Termine la session mains libres en cours avec le reliquat audio, transcrit
/// la fin éventuelle, puis lance un unique nettoyage LLM (strictement ignoré
/// en `Privacy::Password`) et enregistre dans l'historique uniquement en
/// `Privacy::Normal`.
pub fn live_finish(tail: Vec<f32>) -> Dictation {
    let active = { lock(&LIVE).take() };
    let Some(mut active) = active else {
        return outcome(Ok(Outcome::Skipped));
    };
    if CANCELLED_LIVE_ID.load(Ordering::SeqCst) >= active.id {
        return outcome(Ok(Outcome::Skipped));
    }

    let mut engine = lock(&ENGINE);
    let Some(engine) = engine.as_mut() else {
        return Dictation {
            kind: "error",
            error: Some("moteur non démarré : téléchargez un modèle".into()),
            ..Dictation::default()
        };
    };

    let started = Instant::now();
    let prior_asr_ms = active.session.total_asr_ms();
    let hints: &[String] = if active.privacy == Privacy::Password {
        &[]
    } else {
        &active.context_terms
    };

    for chunk in active.session.feed(&tail) {
        match engine.pipeline.transcribe_chunk(&chunk, hints) {
            Ok(Some((text, ms))) => {
                active.session.push_text(&text, ms);
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("transcription d'une tranche finale en direct : {e}");
                if active.asr_error.is_none() {
                    active.asr_error = Some(e.to_string());
                }
            }
        }
    }
    if let Some(last_chunk) = active.session.finish() {
        match engine.pipeline.transcribe_chunk(&last_chunk, hints) {
            Ok(Some((text, ms))) => {
                active.session.push_text(&text, ms);
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("transcription de la fin de dictée en direct : {e}");
                if active.asr_error.is_none() {
                    active.asr_error = Some(e.to_string());
                }
            }
        }
    }

    let elapsed = |started: Instant| {
        prior_asr_ms
            .saturating_add(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    };

    if active.session.is_empty() {
        if let Some(err) = active.asr_error {
            return Dictation {
                kind: "error",
                error: Some(err),
                ms: elapsed(started),
                ..Dictation::default()
            };
        }
        let mut skipped = outcome(Ok(Outcome::Skipped));
        skipped.ms = elapsed(started);
        return skipped;
    }

    if active.privacy == Privacy::Password {
        // Rien ne quitte le téléphone, rien n'est gardé.
        let text = active.session.full_raw();
        let mut result = if text.is_empty() {
            outcome(Ok(Outcome::Skipped))
        } else {
            Dictation {
                kind: "text",
                raw: text.clone(),
                text,
                ..Dictation::default()
            }
        };
        result.ms = elapsed(started);
        return result;
    }

    let target = Target {
        app: active.app.clone(),
        context_terms: active.context_terms,
        ..Target::default()
    };
    let raw_dictation = active.session.to_raw();
    let translate_to = match active.mode {
        Mode::Translate => Some(engine.translate_to.as_str()),
        _ => None,
    };
    let raw = engine
        .pipeline
        .finalize_raw(&raw_dictation, &target, None, translate_to);
    if let Ok(outcome) = &raw {
        record_stats(
            engine.ledger.as_mut(),
            outcome,
            raw_dictation.duration_ms,
            active.app.as_deref(),
            &engine.pricing,
            active.privacy,
        );
    }
    if active.privacy == Privacy::Normal
        && let (Ok(outcome), Some(history)) = (&raw, engine.history.as_mut())
        && let Some(entry) = entry_with_duration(
            outcome,
            raw_dictation.duration_ms,
            active.app,
            &engine.pricing,
        )
        && let Err(e) = history.append(&entry)
    {
        log::warn!("historique : {e}");
    }
    let mut result = outcome(raw);
    result.ms = elapsed(started);
    log::info!(
        "dictée mains libres traitée en {} ms ({}, {} tranches)",
        result.ms,
        result.kind,
        active.session.segment_count()
    );
    result
}

/// Annule immédiatement la session mains libres en cours sans bloquer sur
/// une transcription ASR éventuelle.
pub fn live_cancel() {
    let latest = NEXT_LIVE_ID.load(Ordering::SeqCst);
    CANCELLED_LIVE_ID.fetch_max(latest, Ordering::SeqCst);
    if let Ok(mut live) = LIVE.try_lock() {
        *live = None;
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
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::thread;

    use vtt_core::config::PromptPreset;
    use vtt_core::usage::Usage;
    use vtt_core::{Cleaned, Directives};

    use super::*;

    /// Sérialise les tests qui manipulent les statiques globales `ENGINE` et `API_KEY`.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct Guard {
        _lock: MutexGuard<'static, ()>,
    }

    impl Guard {
        fn acquire() -> Self {
            let guard = lock(&TEST_LOCK);
            release();
            set_api_key(None);
            Self { _lock: guard }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            release();
            set_api_key(None);
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-android-engine-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn speech_samples() -> Vec<f32> {
        vec![0.1; 16_000]
    }

    struct FixedTranscriber {
        reply: String,
        hints: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl Transcriber for FixedTranscriber {
        fn transcribe(&self, _clip: &AudioClip) -> Result<String, Error> {
            Ok(self.reply.clone())
        }

        fn transcribe_with_hints(
            &self,
            _clip: &AudioClip,
            hints: &[String],
        ) -> Result<String, Error> {
            if let Some(slot) = &self.hints {
                *lock(slot) = hints.to_vec();
            }
            Ok(self.reply.clone())
        }
    }

    struct SpyCleaner {
        calls: Arc<AtomicUsize>,
        reply: String,
        usage: Option<Usage>,
        last_context: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl PostProcessor for SpyCleaner {
        fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(slot) = &self.last_context {
                *lock(slot) = directives.context_terms.to_vec();
            }
            let text = match directives.translate_to {
                Some(lang) => format!("[{lang}] {raw}"),
                None => self.reply.clone(),
            };
            Ok(Cleaned {
                text,
                usage: self.usage,
                fallback: None,
            })
        }

        fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Cleaned {
                text: format!("{instruction} => {}", selection.unwrap_or("<vide>")),
                usage: self.usage,
                fallback: None,
            })
        }
    }

    fn install_test_engine(
        raw: &str,
        cleaner: Option<Box<dyn PostProcessor>>,
        history_file: Option<PathBuf>,
        pricing: Pricing,
        translate_to: &str,
    ) {
        install_test_engine_with_context(
            raw,
            None,
            cleaner,
            history_file,
            pricing,
            translate_to,
            DictationConfig::default(),
            Vec::new(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn install_test_engine_with_context(
        raw: &str,
        asr_hints: Option<Arc<Mutex<Vec<String>>>>,
        cleaner: Option<Box<dyn PostProcessor>>,
        history_file: Option<PathBuf>,
        pricing: Pricing,
        translate_to: &str,
        dictation: DictationConfig,
        dictionary: Vec<String>,
    ) {
        let pipeline = Pipeline::new(
            Box::new(FixedTranscriber {
                reply: raw.to_owned(),
                hints: asr_hints,
            }),
            cleaner,
            Box::new(Collect),
        );
        let ledger = history_file
            .as_ref()
            .map(|file| Ledger::open(file.with_file_name("usage.json")));
        *lock(&ENGINE) = Some(Engine {
            pipeline,
            model: "test-model".into(),
            history: history_file.map(|file| History::open(file, 50)),
            ledger,
            pricing,
            translate_to: translate_to.into(),
            dictation,
            dictionary,
        });
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
    fn outcomes_become_simple_results() {
        let _guard = Guard::acquire();
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
            dictate(
                vec![0.0; 16_000],
                None,
                Mode::Dictate,
                Privacy::Normal,
                None
            )
            .kind,
            "error",
            "pas de moteur"
        );
        set_api_key(Some("  ".into()));
        assert_eq!(*lock(&API_KEY), None);
    }

    #[test]
    fn set_api_key_trims_and_clears() {
        let _guard = Guard::acquire();
        set_api_key(Some("  sk-test-123 \n".into()));
        assert_eq!(lock(&API_KEY).as_deref(), Some("sk-test-123"));
        set_api_key(Some("   ".into()));
        assert_eq!(*lock(&API_KEY), None);
        set_api_key(Some("sk-2".into()));
        set_api_key(None);
        assert_eq!(*lock(&API_KEY), None);
    }

    #[test]
    fn start_validates_toml_and_requires_downloaded_model() {
        let _guard = Guard::acquire();
        let dir = temp_dir("start");
        let models_dir = dir.join("models");
        let history_file = dir.join("history.jsonl");
        std::fs::create_dir_all(&models_dir).unwrap();

        assert!(start(&models_dir, &history_file, "invalide = = =").is_err());

        let err = start(&models_dir, &history_file, "").unwrap_err();
        assert!(err.contains("non téléchargé"), "{err}");
        assert!(!loaded());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loaded_release_and_unloaded_transcribe_reflect_engine_state() {
        let _guard = Guard::acquire();
        assert!(!loaded());
        let err = transcribe(speech_samples()).unwrap_err();
        assert!(err.contains("moteur non démarré"), "{err}");

        install_test_engine(
            "bonjour le monde",
            None,
            None,
            Pricing::default(),
            "anglais",
        );
        assert!(loaded());
        assert_eq!(
            transcribe(speech_samples()).unwrap().as_deref(),
            Some("bonjour le monde")
        );
        assert_eq!(transcribe(vec![0.0; 16_000]).unwrap(), None);

        release();
        assert!(!loaded());
        assert!(transcribe(speech_samples()).is_err());
    }

    #[test]
    fn privacy_password_skips_llm_and_never_writes_history() {
        let _guard = Guard::acquire();
        let dir = temp_dir("password");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let cleaner = Box::new(SpyCleaner {
            calls: Arc::clone(&calls),
            reply: "NE DOIT PAS SERVIR".into(),
            usage: None,
            last_context: None,
        });
        install_test_engine(
            "mon code secret",
            Some(cleaner),
            Some(file.clone()),
            Pricing::default(),
            "anglais",
        );

        let result = dictate(
            speech_samples(),
            Some("com.bank".into()),
            Mode::Dictate,
            Privacy::Password,
            None,
        );
        assert_eq!(result.kind, "text");
        assert_eq!(result.raw, "mon code secret");
        assert_eq!(result.text, "mon code secret");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(history(&file, 10).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn privacy_no_history_runs_llm_without_persisting_history() {
        let _guard = Guard::acquire();
        let dir = temp_dir("no-history");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let cleaner = Box::new(SpyCleaner {
            calls: Arc::clone(&calls),
            reply: "Texte nettoyé.".into(),
            usage: None,
            last_context: None,
        });
        install_test_engine(
            "texte brut",
            Some(cleaner),
            Some(file.clone()),
            Pricing::default(),
            "anglais",
        );

        let result = dictate(
            speech_samples(),
            Some("org.mozilla.firefox".into()),
            Mode::Dictate,
            Privacy::NoHistory,
            None,
        );
        assert_eq!(result.kind, "text");
        assert_eq!(result.text, "Texte nettoyé.");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(history(&file, 10).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn privacy_normal_records_history_with_app_and_cost() {
        let _guard = Guard::acquire();
        let dir = temp_dir("normal");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let cleaner = Box::new(SpyCleaner {
            calls: Arc::clone(&calls),
            reply: "Bonjour à tous.".into(),
            usage: Some(Usage {
                input_tokens: 1_000,
                cached_input_tokens: 0,
                output_tokens: 500,
            }),
            last_context: None,
        });
        let pricing = Pricing {
            input_per_million: 1.0,
            cached_input_per_million: 0.0,
            output_per_million: 2.0,
            currency: "€".into(),
        };
        install_test_engine(
            "bonjour a tous",
            Some(cleaner),
            Some(file.clone()),
            pricing,
            "anglais",
        );

        let result = dictate(
            speech_samples(),
            Some("com.whatsapp".into()),
            Mode::Dictate,
            Privacy::Normal,
            None,
        );
        assert_eq!(result.kind, "text");
        assert_eq!(result.text, "Bonjour à tous.");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let entries = history(&file, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app.as_deref(), Some("com.whatsapp"));
        assert_eq!(entries[0].duration_ms, 1_000);
        assert_eq!(entries[0].currency.as_deref(), Some("€"));
        assert!((entries[0].cost.unwrap() - 0.002).abs() < 1e-9);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn translate_and_command_modes_forward_language_and_selection() {
        let _guard = Guard::acquire();
        let calls = Arc::new(AtomicUsize::new(0));
        let cleaner = Box::new(SpyCleaner {
            calls: Arc::clone(&calls),
            reply: "unused".into(),
            usage: None,
            last_context: None,
        });
        install_test_engine(
            "rends plus court",
            Some(cleaner),
            None,
            Pricing::default(),
            "espagnol",
        );

        let translated = dictate(
            speech_samples(),
            None,
            Mode::Translate,
            Privacy::Normal,
            None,
        );
        assert_eq!(translated.kind, "text");
        assert_eq!(translated.text, "[espagnol] rends plus court");

        let commanded = dictate(
            speech_samples(),
            None,
            Mode::Command(Some("texte initial".into())),
            Privacy::Normal,
            None,
        );
        assert_eq!(commanded.kind, "text");
        assert_eq!(commanded.text, "rends plus court => texte initial");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn screen_context_respects_opt_in_privacy_and_exclusions() {
        let _guard = Guard::acquire();
        let sample_ctx =
            "Bonjour Élodie, le module ContextManager (sk-1234567890abcdefghijklmno) est prêt.";

        // 1. Désactivé par défaut : aucun terme transmis même sur champ normal.
        let asr_hints = Arc::new(Mutex::new(Vec::new()));
        let llm_ctx = Arc::new(Mutex::new(Vec::new()));
        install_test_engine_with_context(
            "bonjour elodie",
            Some(Arc::clone(&asr_hints)),
            Some(Box::new(SpyCleaner {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: "Bonjour Élodie.".into(),
                usage: None,
                last_context: Some(Arc::clone(&llm_ctx)),
            })),
            None,
            Pricing::default(),
            "anglais",
            DictationConfig::default(),
            Vec::new(),
        );
        dictate(
            speech_samples(),
            Some("com.slack".into()),
            Mode::Dictate,
            Privacy::Normal,
            Some(sample_ctx),
        );
        assert!(lock(&asr_hints).is_empty());
        assert!(lock(&llm_ctx).is_empty());

        // 2. Activé : termes extraits sans le secret et transmis à l'ASR et au LLM.
        let enabled_cfg = DictationConfig {
            screen_context: true,
            screen_context_excluded_apps: vec!["com.bank".into()],
            ..DictationConfig::default()
        };
        install_test_engine_with_context(
            "bonjour elodie",
            Some(Arc::clone(&asr_hints)),
            Some(Box::new(SpyCleaner {
                calls: Arc::new(AtomicUsize::new(0)),
                reply: "Bonjour Élodie.".into(),
                usage: None,
                last_context: Some(Arc::clone(&llm_ctx)),
            })),
            None,
            Pricing::default(),
            "anglais",
            enabled_cfg,
            Vec::new(),
        );
        dictate(
            speech_samples(),
            Some("com.slack".into()),
            Mode::Dictate,
            Privacy::Normal,
            Some(sample_ctx),
        );
        let received_asr = lock(&asr_hints).clone();
        let received_llm = lock(&llm_ctx).clone();
        assert!(
            received_asr.iter().any(|t| t == "Élodie"),
            "{received_asr:?}"
        );
        assert!(
            received_asr.iter().any(|t| t == "ContextManager"),
            "{received_asr:?}"
        );
        assert!(
            !received_asr.iter().any(|t| t.contains("sk-")),
            "secret filtré : {received_asr:?}"
        );
        assert_eq!(received_asr, received_llm);

        // 3. Navigation privée / NoHistory -> strictement bloqué.
        lock(&asr_hints).clear();
        lock(&llm_ctx).clear();
        dictate(
            speech_samples(),
            Some("com.slack".into()),
            Mode::Dictate,
            Privacy::NoHistory,
            Some(sample_ctx),
        );
        assert!(lock(&asr_hints).is_empty());
        assert!(lock(&llm_ctx).is_empty());

        // 4. Application exclue -> bloqué.
        dictate(
            speech_samples(),
            Some("com.bank.mobile".into()),
            Mode::Dictate,
            Privacy::Normal,
            Some(sample_ctx),
        );
        assert!(lock(&asr_hints).is_empty());
        assert!(lock(&llm_ctx).is_empty());

        // 5. Gestionnaire de mots de passe -> toujours bloqué.
        dictate(
            speech_samples(),
            Some("com.x8bit.bitwarden".into()),
            Mode::Dictate,
            Privacy::Normal,
            Some(sample_ctx),
        );
        assert!(lock(&asr_hints).is_empty());
        assert!(lock(&llm_ctx).is_empty());

        // 6. Mot de passe -> bloqué (transcription seule sans indices).
        dictate(
            speech_samples(),
            Some("com.slack".into()),
            Mode::Dictate,
            Privacy::Password,
            Some(sample_ctx),
        );
        assert!(lock(&asr_hints).is_empty());
        assert!(lock(&llm_ctx).is_empty());
    }

    #[test]
    fn summarizer_respects_enabled_flag_custom_prompt_and_api_key() {
        let _guard = Guard::acquire();
        let mut config = Config::default();
        config.llm.enabled = false;
        assert!(summarizer(&config).is_none());

        let (url, server) = serve_once(
            200,
            r#"{"choices":[{"message":{"content":"Résumé court."}}]}"#,
        );
        config.llm.enabled = true;
        config.llm.base_url = url;
        config.prompts.push(PromptPreset {
            name: "Synthese".into(),
            kind: PromptKind::CompteRendu,
            text: "Prompt compte rendu personnalisé".into(),
        });
        config.meetings.summary_prompt = "Synthese".into();
        set_api_key(Some("sk-android-key".into()));

        let client = summarizer(&config).unwrap();
        let cleaned = client
            .summarize("Point équipe", "[00:00] Moi : Salut.", None, false)
            .unwrap();
        let req = server.join().unwrap();
        assert_eq!(cleaned.text, "Résumé court.");
        assert!(req.contains("Prompt compte rendu personnalisé"), "{req}");
        assert!(req.contains("Bearer sk-android-key"), "{req}");
    }

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

        let dir = temp_dir("history");
        let file = dir.join("history.jsonl");
        History::open(file.clone(), 10).append(&e).unwrap();
        assert_eq!(history(&file, 5).unwrap().len(), 1);
        clear_history(&file).unwrap();
        assert!(history(&file, 5).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct SequenceTranscriber {
        replies: Mutex<std::collections::VecDeque<Result<String, Error>>>,
        hints: Option<Arc<Mutex<Vec<Vec<String>>>>>,
    }

    impl Transcriber for SequenceTranscriber {
        fn transcribe(&self, clip: &AudioClip) -> Result<String, Error> {
            self.transcribe_with_hints(clip, &[])
        }

        fn transcribe_with_hints(
            &self,
            _clip: &AudioClip,
            hints: &[String],
        ) -> Result<String, Error> {
            if let Some(slot) = &self.hints {
                lock(slot).push(hints.to_vec());
            }
            lock(&self.replies)
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    struct RecordingCleaner {
        calls: Arc<AtomicUsize>,
        last_raw: Arc<Mutex<String>>,
        last_context: Arc<Mutex<Vec<String>>>,
        usage: Option<Usage>,
    }

    impl PostProcessor for RecordingCleaner {
        fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *lock(&self.last_raw) = raw.to_owned();
            *lock(&self.last_context) = directives.context_terms.to_vec();
            let text = match directives.translate_to {
                Some(lang) => format!("[{lang}] {raw}"),
                None => format!("Propre : {raw}"),
            };
            Ok(Cleaned {
                text,
                usage: self.usage,
                fallback: None,
            })
        }
    }

    fn live_speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| if i % 2 == 0 { 0.25 } else { -0.25 })
            .collect()
    }

    fn live_silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * 16_000.0) as usize]
    }

    fn speech_with_pause(seconds: f32) -> Vec<f32> {
        let mut out = live_speech(seconds);
        out.extend(live_silence(0.5));
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn install_sequence_engine(
        replies: Vec<Result<String, Error>>,
        asr_hints: Option<Arc<Mutex<Vec<Vec<String>>>>>,
        cleaner: Option<Box<dyn PostProcessor>>,
        history_file: Option<PathBuf>,
        pricing: Pricing,
        translate_to: &str,
        dictation: DictationConfig,
    ) {
        let pipeline = Pipeline::new(
            Box::new(SequenceTranscriber {
                replies: Mutex::new(replies.into()),
                hints: asr_hints,
            }),
            cleaner,
            Box::new(Collect),
        );
        let ledger = history_file
            .as_ref()
            .map(|file| Ledger::open(file.with_file_name("usage.json")));
        *lock(&ENGINE) = Some(Engine {
            pipeline,
            model: "test-model".into(),
            history: history_file.map(|file| History::open(file, 50)),
            ledger,
            pricing,
            translate_to: translate_to.into(),
            dictation,
            dictionary: Vec::new(),
        });
    }

    #[test]
    fn live_session_streams_preview_runs_single_llm_pass_and_records_history() {
        let _guard = Guard::acquire();
        let dir = temp_dir("live-normal");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let last_raw = Arc::new(Mutex::new(String::new()));
        let last_ctx = Arc::new(Mutex::new(Vec::new()));
        let asr_hints = Arc::new(Mutex::new(Vec::new()));

        let enabled_cfg = DictationConfig {
            screen_context: true,
            ..DictationConfig::default()
        };
        install_sequence_engine(
            vec![
                Ok("Première phrase".into()),
                Ok("deuxième phrase".into()),
                Ok("et fin.".into()),
            ],
            Some(Arc::clone(&asr_hints)),
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw: Arc::clone(&last_raw),
                last_context: Arc::clone(&last_ctx),
                usage: Some(Usage {
                    input_tokens: 1_000,
                    cached_input_tokens: 0,
                    output_tokens: 500,
                }),
            })),
            Some(file.clone()),
            Pricing {
                input_per_million: 1.0,
                output_per_million: 2.0,
                currency: "€".into(),
                ..Pricing::default()
            },
            "anglais",
            enabled_cfg,
        );

        let id = live_start(
            Some("com.whatsapp".into()),
            Mode::Dictate,
            Privacy::Normal,
            Some("Bonjour Élodie, projet ContextManager (sk-1234567890abcdefghijklmno)."),
        )
        .unwrap();
        assert!(id > 0);

        // Sous la borne minimale (3,5 s) : aucune tranche découpée.
        let early = live_feed(&live_speech(2.0)).unwrap();
        assert!(!early.changed);
        assert_eq!(early.preview, "");
        assert_eq!(early.segments, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Complète la première tranche (2 s + 2 s + pause 0,5 s).
        let first = live_feed(&speech_with_pause(2.0)).unwrap();
        assert!(first.changed);
        assert_eq!(first.preview, "Première phrase");
        assert_eq!(first.segments, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Deuxième tranche (4 s + pause 0,5 s).
        let second = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(second.changed);
        assert_eq!(second.preview, "Première phrase deuxième phrase");
        assert_eq!(second.segments, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // Arrêt avec reliquat (1,5 s) : un seul appel LLM sur tout le texte.
        let result = live_finish(live_speech(1.5));
        assert_eq!(result.kind, "text");
        assert_eq!(result.raw, "Première phrase deuxième phrase et fin.");
        assert_eq!(
            result.text,
            "Propre : Première phrase deuxième phrase et fin."
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*lock(&last_raw), "Première phrase deuxième phrase et fin.");

        // Contexte transmis aux tranches ASR et au LLM sans fuite du secret.
        let hints = lock(&asr_hints).clone();
        assert_eq!(hints.len(), 3);
        for chunk_hints in &hints {
            assert!(chunk_hints.iter().any(|t| t == "Élodie"), "{chunk_hints:?}");
            assert!(
                !chunk_hints.iter().any(|t| t.contains("sk-")),
                "{chunk_hints:?}"
            );
        }
        assert_eq!(*lock(&last_ctx), hints[0]);

        // Une seule entrée d'historique avec la durée totale et le coût.
        let entries = history(&file, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app.as_deref(), Some("com.whatsapp"));
        assert_eq!(entries[0].duration_ms, 10_500);
        assert!((entries[0].cost.unwrap() - 0.002).abs() < 1e-9);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn live_session_translate_runs_single_translation_pass_on_finish() {
        let _guard = Guard::acquire();
        let calls = Arc::new(AtomicUsize::new(0));
        let last_raw = Arc::new(Mutex::new(String::new()));
        let last_ctx = Arc::new(Mutex::new(Vec::new()));
        install_sequence_engine(
            vec![Ok("Bonjour tout le monde".into()), Ok("à demain.".into())],
            None,
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw,
                last_context: last_ctx,
                usage: None,
            })),
            None,
            Pricing::default(),
            "espagnol",
            DictationConfig::default(),
        );

        live_start(None, Mode::Translate, Privacy::Normal, None).unwrap();
        let f = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(f.changed);
        assert_eq!(f.preview, "Bonjour tout le monde");
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let result = live_finish(live_speech(1.2));
        assert_eq!(result.kind, "text");
        assert_eq!(result.text, "[espagnol] Bonjour tout le monde à demain.");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn live_session_privacy_password_hides_preview_skips_llm_context_and_history() {
        let _guard = Guard::acquire();
        let dir = temp_dir("live-password");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let asr_hints = Arc::new(Mutex::new(Vec::new()));
        let enabled_cfg = DictationConfig {
            screen_context: true,
            ..DictationConfig::default()
        };
        install_sequence_engine(
            vec![Ok("code".into()), Ok("secret 1234".into())],
            Some(Arc::clone(&asr_hints)),
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw: Arc::new(Mutex::new(String::new())),
                last_context: Arc::new(Mutex::new(Vec::new())),
                usage: None,
            })),
            Some(file.clone()),
            Pricing::default(),
            "anglais",
            enabled_cfg,
        );

        live_start(
            Some("com.bank".into()),
            Mode::Dictate,
            Privacy::Password,
            Some("Élodie ContextManager"),
        )
        .unwrap();
        // Jamais d'aperçu exposé en mot de passe.
        let feed = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(!feed.changed);
        assert_eq!(feed.preview, "");
        assert_eq!(feed.segments, 1);

        let result = live_finish(live_speech(1.5));
        assert_eq!(result.kind, "text");
        assert_eq!(result.raw, "code secret 1234");
        assert_eq!(result.text, "code secret 1234");
        // Ni LLM, ni contexte ASR, ni historique.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(lock(&asr_hints).iter().all(|h| h.is_empty()));
        assert!(history(&file, 10).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn live_session_privacy_no_history_skips_context_and_history_but_runs_llm() {
        let _guard = Guard::acquire();
        let dir = temp_dir("live-no-history");
        let file = dir.join("history.jsonl");
        let calls = Arc::new(AtomicUsize::new(0));
        let last_ctx = Arc::new(Mutex::new(Vec::new()));
        let asr_hints = Arc::new(Mutex::new(Vec::new()));
        let enabled_cfg = DictationConfig {
            screen_context: true,
            ..DictationConfig::default()
        };
        install_sequence_engine(
            vec![Ok("recherche privée".into()), Ok("sur le web".into())],
            Some(Arc::clone(&asr_hints)),
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw: Arc::new(Mutex::new(String::new())),
                last_context: Arc::clone(&last_ctx),
                usage: None,
            })),
            Some(file.clone()),
            Pricing::default(),
            "anglais",
            enabled_cfg,
        );

        live_start(
            Some("org.mozilla.firefox".into()),
            Mode::Dictate,
            Privacy::NoHistory,
            Some("Élodie ContextManager"),
        )
        .unwrap();
        let feed = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(feed.changed);
        assert_eq!(feed.preview, "recherche privée");

        let result = live_finish(live_speech(1.5));
        assert_eq!(result.kind, "text");
        assert_eq!(result.text, "Propre : recherche privée sur le web");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(lock(&asr_hints).iter().all(|h| h.is_empty()));
        assert!(lock(&last_ctx).is_empty());
        assert!(history(&file, 10).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn live_cancel_drops_session_and_prevents_finish_or_llm() {
        let _guard = Guard::acquire();
        let calls = Arc::new(AtomicUsize::new(0));
        install_sequence_engine(
            vec![Ok("texte annulé".into()), Ok("suite ignorée".into())],
            None,
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw: Arc::new(Mutex::new(String::new())),
                last_context: Arc::new(Mutex::new(Vec::new())),
                usage: None,
            })),
            None,
            Pricing::default(),
            "anglais",
            DictationConfig::default(),
        );

        live_start(None, Mode::Dictate, Privacy::Normal, None).unwrap();
        let first = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(first.changed);

        live_cancel();
        let after = live_feed(&speech_with_pause(4.0)).unwrap();
        assert_eq!(after, LiveFeed::default());
        let finished = live_finish(live_speech(1.5));
        assert_eq!(finished.kind, "skip");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn live_start_rejects_command_mode_and_unloaded_engine_and_reports_asr_failure() {
        let _guard = Guard::acquire();
        assert!(live_start(None, Mode::Dictate, Privacy::Normal, None).is_err());

        install_sequence_engine(
            vec![Err(Error::Asr("whisper hs".into()))],
            None,
            None,
            None,
            Pricing::default(),
            "anglais",
            DictationConfig::default(),
        );
        assert!(live_start(None, Mode::Command(None), Privacy::Normal, None).is_err());

        live_start(None, Mode::Dictate, Privacy::Normal, None).unwrap();
        let f = live_feed(&speech_with_pause(4.0)).unwrap();
        assert!(!f.changed);
        let result = live_finish(Vec::new());
        assert_eq!(result.kind, "error");
        assert!(result.error.as_deref().unwrap().contains("whisper hs"));
    }

    #[test]
    fn dictation_records_usage_stats_and_respects_privacy() {
        let _guard = Guard::acquire();
        let dir = temp_dir("usage-dictate");
        let file = dir.join("history.jsonl");
        let usage_file = dir.join("usage.json");
        let calls = Arc::new(AtomicUsize::new(0));
        install_test_engine(
            "bonjour tout le monde",
            Some(Box::new(SpyCleaner {
                calls: Arc::clone(&calls),
                reply: "Bonjour tout le monde.".into(),
                usage: Some(Usage {
                    input_tokens: 200,
                    cached_input_tokens: 50,
                    output_tokens: 40,
                }),
                last_context: None,
            })),
            Some(file),
            Pricing::default(),
            "anglais",
        );

        // 1. Dictée normale : 4 mots, 1000 ms, app enregistrée + jetons LLM.
        let r1 = dictate(
            speech_samples(),
            Some("com.whatsapp".into()),
            Mode::Dictate,
            Privacy::Normal,
            None,
        );
        assert_eq!(r1.kind, "text");

        // 2. Navigation privée (NoHistory) : mots et durée comptés, mais pas l'app.
        let r2 = dictate(
            speech_samples(),
            Some("org.mozilla.firefox".into()),
            Mode::Dictate,
            Privacy::NoHistory,
            None,
        );
        assert_eq!(r2.kind, "text");

        // 3. Champ mot de passe : strictement rien comptabilisé.
        let r3 = dictate(
            speech_samples(),
            Some("com.bank".into()),
            Mode::Dictate,
            Privacy::Password,
            None,
        );
        assert_eq!(r3.kind, "text");

        // 4. Command Mode : compteur `commands` incrémenté.
        let r4 = dictate(
            speech_samples(),
            Some("com.whatsapp".into()),
            Mode::Command(Some("texte".into())),
            Privacy::Normal,
            None,
        );
        assert_eq!(r4.kind, "text");

        let ledger = Ledger::open(usage_file);
        let day = day_from_unix_ms(now_ms());
        let totals = ledger.day(&day);
        assert_eq!(totals.words, 12); // 3 dictées × 4 mots (Password exclu)
        assert_eq!(totals.dictations, 3);
        assert_eq!(totals.commands, 1);
        assert_eq!(totals.audio_ms, 3_000);
        assert_eq!(totals.requests, 3);
        assert_eq!(totals.input_tokens, 600);
        assert_eq!(totals.output_tokens, 120);

        let apps = ledger.apps(None);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].app, "com.whatsapp");
        assert_eq!(apps[0].totals.words, 8);
        assert_eq!(apps[0].totals.commands, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn live_finish_records_usage_stats_except_on_password() {
        let _guard = Guard::acquire();
        let dir = temp_dir("usage-live");
        let file = dir.join("history.jsonl");
        let usage_file = dir.join("usage.json");
        let calls = Arc::new(AtomicUsize::new(0));
        install_sequence_engine(
            vec![
                Ok("un deux".into()),
                Ok("trois quatre cinq".into()),
                Ok("secret mot de passe".into()),
            ],
            None,
            Some(Box::new(RecordingCleaner {
                calls: Arc::clone(&calls),
                last_raw: Arc::new(Mutex::new(String::new())),
                last_context: Arc::new(Mutex::new(Vec::new())),
                usage: Some(Usage {
                    input_tokens: 300,
                    cached_input_tokens: 0,
                    output_tokens: 60,
                }),
            })),
            Some(file),
            Pricing::default(),
            "anglais",
            DictationConfig::default(),
        );

        live_start(
            Some("com.slack".into()),
            Mode::Dictate,
            Privacy::Normal,
            None,
        )
        .unwrap();
        let _ = live_feed(&speech_with_pause(4.0)).unwrap();
        let r1 = live_finish(live_speech(1.0));
        assert_eq!(r1.kind, "text");

        // Session mot de passe : aucun compteur touché.
        live_start(
            Some("com.bank".into()),
            Mode::Dictate,
            Privacy::Password,
            None,
        )
        .unwrap();
        let r2 = live_finish(live_speech(1.0));
        assert_eq!(r2.kind, "text");

        let ledger = Ledger::open(usage_file);
        let day = day_from_unix_ms(now_ms());
        let totals = ledger.day(&day);
        assert_eq!(totals.words, 5);
        assert_eq!(totals.dictations, 1);
        assert!(totals.audio_ms >= 5_000);
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.input_tokens, 300);

        let apps = ledger.apps(None);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].app, "com.slack");
        assert_eq!(apps[0].totals.words, 5);

        let _ = std::fs::remove_dir_all(dir);
    }
}
