//! Moteur de dictée du téléphone : Whisper, Parakeet ou Vosk + pipeline
//! commun (snippets,
//! commandes vocales, nettoyage LLM, traduction, Command Mode). Le texte est
//! rendu à l'application, qui l'insère dans le champ en cours (clavier
//! Android) ; l'historique reste sur le téléphone.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vtt_core::config::{LlmConfig, Pricing, PromptKind};
use vtt_core::history::{Entry, History};
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
        pricing: config.llm.pricing.clone(),
        translate_to: config.translation.language.clone(),
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

    struct FixedTranscriber(String);

    impl Transcriber for FixedTranscriber {
        fn transcribe(&self, _clip: &AudioClip) -> Result<String, Error> {
            Ok(self.0.clone())
        }
    }

    struct SpyCleaner {
        calls: Arc<AtomicUsize>,
        reply: String,
        usage: Option<Usage>,
    }

    impl PostProcessor for SpyCleaner {
        fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
        let pipeline = Pipeline::new(
            Box::new(FixedTranscriber(raw.to_owned())),
            cleaner,
            Box::new(Collect),
        );
        *lock(&ENGINE) = Some(Engine {
            pipeline,
            model: "test-model".into(),
            history: history_file.map(|file| History::open(file, 50)),
            pricing,
            translate_to: translate_to.into(),
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
            dictate(vec![0.0; 16_000], None, Mode::Dictate, Privacy::Normal).kind,
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
        });
        install_test_engine(
            "rends plus court",
            Some(cleaner),
            None,
            Pricing::default(),
            "espagnol",
        );

        let translated = dictate(speech_samples(), None, Mode::Translate, Privacy::Normal);
        assert_eq!(translated.kind, "text");
        assert_eq!(translated.text, "[espagnol] rends plus court");

        let commanded = dictate(
            speech_samples(),
            None,
            Mode::Command(Some("texte initial".into())),
            Privacy::Normal,
        );
        assert_eq!(commanded.kind, "text");
        assert_eq!(commanded.text, "rends plus court => texte initial");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
}
