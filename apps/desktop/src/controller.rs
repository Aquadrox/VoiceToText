//! Chef d'orchestre : charge le modèle, écoute le clavier, pilote le micro
//! et confie chaque enregistrement au pipeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use vtt_asr::WhisperTranscriber;
use vtt_core::config::Pricing;
use vtt_core::hotkey::{Action, PushToTalk};
use vtt_core::{AudioClip, Config, Outcome, Pipeline, PostProcessor};
use vtt_llm::LlmCleaner;
use vtt_platform::audio::Recorder;
use vtt_platform::{ClipboardPaster, now_ms};

use crate::costs::{self, LedgerStore};
use crate::history::{self, HistoryStore};
use crate::model;
use crate::paths::Paths;
use crate::settings;
use crate::status::{Status, StatusReporter};

/// Un enregistrement plus long est coupé et traité (touche restée bloquée…).
const MAX_RECORDING_MS: u64 = 5 * 60 * 1000;
/// Même filet pour le mode mains libres, où les dictées sont plus longues.
const MAX_HANDS_FREE_MS: u64 = 10 * 60 * 1000;
/// Fréquence de vérification de l'état physique du raccourci.
const POLL: Duration = Duration::from_millis(100);
/// Délai avant de se fier à l'état physique de la touche : Windows le met à
/// jour après le passage dans le hook.
const KEY_STATE_GRACE_MS: u64 = 300;

pub fn spawn(
    reporter: Arc<StatusReporter>,
    history: HistoryStore,
    ledger: LedgerStore,
    config: Config,
    paths: Paths,
) {
    thread::Builder::new()
        .name("controller".into())
        .spawn(move || {
            if let Err(e) = run(&reporter, history, ledger, &config, &paths) {
                reporter.set(Status::Error(e));
            }
        })
        .expect("création du thread contrôleur");
}

fn run(
    reporter: &Arc<StatusReporter>,
    history: HistoryStore,
    ledger: LedgerStore,
    config: &Config,
    paths: &Paths,
) -> Result<(), String> {
    let trigger = vtt_platform::key_code(&config.hotkey.key)
        .ok_or_else(|| format!("touche inconnue dans config.toml : {:?}", config.hotkey.key))?;

    let model_path = model::ensure(&paths.models_dir, &config.asr.model, |percent| {
        reporter.set(Status::Loading(format!(
            "Téléchargement du modèle : {percent} %"
        )));
    })?;
    reporter.set(Status::Loading("Chargement du modèle…".into()));
    let transcriber = WhisperTranscriber::load(
        &model_path,
        &config.asr.language,
        config.asr.use_gpu,
        &config.dictionary,
    )
    .map_err(|e| e.to_string())?;
    let cleaner: Option<Box<dyn PostProcessor>> = if config.llm.enabled {
        log::info!(
            "nettoyage : {} via {}",
            config.llm.model,
            config.llm.base_url
        );
        Some(Box::new(LlmCleaner::new(
            &config.llm,
            &config.dictionary,
            settings::api_key_provider(&config.llm),
        )))
    } else {
        log::info!("nettoyage désactivé (llm.enabled = false)");
        None
    };
    let pipeline = Pipeline::new(Box::new(transcriber), cleaner, Box::new(ClipboardPaster))
        .with_snippets(&config.snippets);
    if !config.snippets.is_empty() {
        log::info!("{} snippet(s) chargé(s)", config.snippets.len());
    }

    let recording = Arc::new(AtomicBool::new(false));
    let recorders = Recorders {
        history,
        ledger,
        pricing: config.llm.pricing.clone(),
    };
    let clips = spawn_worker(
        pipeline,
        recorders,
        Arc::clone(reporter),
        Arc::clone(&recording),
    );
    let recorder = Recorder::spawn();

    let (keys_tx, keys) = mpsc::channel();
    vtt_platform::spawn_keyboard_hook(keys_tx)?;
    let mut ptt = PushToTalk::new(trigger, config.hotkey.min_hold_ms.into())
        .with_ignored(vtt_platform::companion_keys(trigger))
        .with_hands_free(config.hotkey.hands_free, vtt_platform::key_code("Escape"));
    log::info!("prêt, raccourci : {}", config.hotkey.key);
    reporter.set(Status::Idle);

    loop {
        let action = match keys.recv_timeout(POLL) {
            Ok(event) => ptt.on_key(event.key, event.down, event.at_ms),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Err("hook clavier arrêté".into()),
        };
        handle(
            action,
            &recorder,
            &recording,
            reporter,
            &clips,
            trigger,
            &config.hotkey.key,
        );

        // Filet de sécurité : relâchement non reçu par le hook, ou durée maximale.
        // En mains libres, la touche est relâchée exprès : seule la durée compte.
        let now = now_ms();
        if let Some(elapsed) = ptt.recording_ms(now) {
            let (released, max_ms) = if ptt.is_hands_free() {
                (false, MAX_HANDS_FREE_MS)
            } else {
                let released = elapsed > KEY_STATE_GRACE_MS && !vtt_platform::key_is_down(trigger);
                (released, MAX_RECORDING_MS)
            };
            if released || elapsed > max_ms {
                log::info!("relâchement forcé après {elapsed} ms");
                let action = ptt.force_release(now);
                handle(
                    action,
                    &recorder,
                    &recording,
                    reporter,
                    &clips,
                    trigger,
                    &config.hotkey.key,
                );
            }
        }
    }
}

fn handle(
    action: Option<Action>,
    recorder: &Recorder,
    recording: &AtomicBool,
    reporter: &StatusReporter,
    clips: &mpsc::Sender<AudioClip>,
    trigger: u32,
    key_name: &str,
) {
    match action {
        Some(Action::Start) => match recorder.start() {
            Ok(()) => {
                recording.store(true, Ordering::SeqCst);
                vtt_platform::suppress_menu_activation(trigger);
                reporter.set(Status::Listening);
            }
            Err(e) => reporter.set(Status::Error(e)),
        },
        Some(Action::HandsFree) => {
            log::info!("mode mains libres");
            reporter.set(Status::HandsFree(key_name.to_owned()));
        }
        Some(Action::Cancel) => {
            recorder.stop();
            recording.store(false, Ordering::SeqCst);
            reporter.set(Status::Idle);
        }
        Some(Action::Stop) => {
            let clip = recorder.stop();
            recording.store(false, Ordering::SeqCst);
            reporter.set(Status::Processing);
            let _ = clips.send(clip);
        }
        None => {}
    }
}

/// Les dictées sont traitées une par une, hors du thread clavier, pour qu'une
/// nouvelle dictée puisse commencer pendant la transcription de la précédente.
/// Ce qui est enregistré après chaque dictée.
struct Recorders {
    history: HistoryStore,
    ledger: LedgerStore,
    pricing: Pricing,
}

fn spawn_worker(
    pipeline: Pipeline,
    recorders: Recorders,
    reporter: Arc<StatusReporter>,
    recording: Arc<AtomicBool>,
) -> mpsc::Sender<AudioClip> {
    let (tx, rx) = mpsc::channel::<AudioClip>();
    thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            for clip in rx {
                match pipeline.run(&clip) {
                    Ok(outcome) => {
                        if let Outcome::Inserted {
                            post_process_error: Some(e),
                            ..
                        } = &outcome
                        {
                            log::warn!("nettoyage ignoré : {e}");
                        }
                        if let Outcome::Inserted {
                            usage: Some(usage), ..
                        } = &outcome
                        {
                            costs::record(&recorders.ledger, usage, &recorders.pricing);
                        }
                        history::record(&recorders.history, &clip, &outcome);
                    }
                    Err(e) => {
                        reporter.set(Status::Error(e.to_string()));
                        continue;
                    }
                }
                if !recording.load(Ordering::SeqCst) {
                    reporter.set(Status::Idle);
                }
            }
        })
        .expect("création du thread pipeline");
    tx
}
