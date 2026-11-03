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
use vtt_core::{AudioClip, Config, Outcome, Pipeline, PostProcessor, Target};
use vtt_llm::LlmCleaner;
use vtt_platform::audio::Recorder;
use vtt_platform::{ClipboardPaster, now_ms};

use crate::costs::{self, LedgerStore};
use crate::history::{self, HistoryStore};
use crate::model;
use crate::paths::Paths;
use crate::raw_paste::{self, LastRaw};
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

/// Ce qui est enregistré après chaque dictée.
pub struct Recorders {
    pub history: HistoryStore,
    pub ledger: LedgerStore,
    pub last_raw: LastRaw,
}

pub fn spawn(reporter: Arc<StatusReporter>, recorders: Recorders, config: Config, paths: Paths) {
    thread::Builder::new()
        .name("controller".into())
        .spawn(move || {
            if let Err(e) = run(&reporter, recorders, &config, &paths) {
                reporter.set(Status::Error(e));
            }
        })
        .expect("création du thread contrôleur");
}

fn run(
    reporter: &Arc<StatusReporter>,
    recorders: Recorders,
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
        .with_snippets(&config.snippets)
        .with_styles(&config.styles);
    if !config.snippets.is_empty() {
        log::info!("{} snippet(s) chargé(s)", config.snippets.len());
    }
    if !config.styles.is_empty() {
        log::info!("{} style(s) par application", config.styles.len());
    }

    let recording = Arc::new(AtomicBool::new(false));
    let clips = spawn_worker(
        pipeline,
        recorders,
        config.llm.pricing.clone(),
        Arc::clone(reporter),
        Arc::clone(&recording),
    );
    let mut controls = Controls {
        recorder: Recorder::spawn(),
        recording,
        reporter,
        clips,
        trigger,
        key_name: &config.hotkey.key,
        target: Target::default(),
        command: false,
    };

    let (keys_tx, keys) = mpsc::channel();
    vtt_platform::spawn_keyboard_hook(keys_tx)?;
    let mut ptt = PushToTalk::new(trigger, config.hotkey.min_hold_ms.into())
        .with_ignored(vtt_platform::companion_keys(trigger))
        .with_hands_free(config.hotkey.hands_free, vtt_platform::key_code("Escape"))
        .with_command_key(command_key(config));
    log::info!("prêt, raccourci : {}", config.hotkey.key);
    reporter.set(Status::Idle);

    loop {
        let action = match keys.recv_timeout(POLL) {
            Ok(event) => ptt.on_key(event.key, event.down, event.at_ms),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Err("hook clavier arrêté".into()),
        };
        controls.handle(action, ptt.is_command());

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
                controls.handle(ptt.force_release(now), false);
            }
        }
    }
}

/// Touche du Command Mode (`hotkey.command_key`, vide : désactivé).
fn command_key(config: &Config) -> Option<u32> {
    let name = config.hotkey.command_key.trim();
    if name.is_empty() {
        return None;
    }
    let key = vtt_platform::key_code(name);
    if key.is_none() {
        log::warn!("touche de commande inconnue : {name:?}, Command Mode désactivé");
    }
    key
}

/// Une dictée à traiter.
struct Job {
    clip: AudioClip,
    target: Target,
    /// Command Mode : la dictée est une consigne pour le texte sélectionné.
    command: bool,
}

/// Ce que le thread clavier pilote : micro, état affiché, file des dictées.
struct Controls<'a> {
    recorder: Recorder,
    recording: Arc<AtomicBool>,
    reporter: &'a StatusReporter,
    clips: mpsc::Sender<Job>,
    trigger: u32,
    key_name: &'a str,
    /// Fenêtre de destination, relevée au début de la dictée (le focus peut
    /// changer pendant la transcription).
    target: Target,
    /// La dictée en cours est une commande.
    command: bool,
}

impl Controls<'_> {
    /// `command` : la dictée en cours est en Command Mode (touche de commande
    /// tenue au moment du démarrage).
    fn handle(&mut self, action: Option<Action>, command: bool) {
        match action {
            Some(Action::Start) => match self.recorder.start() {
                Ok(()) => {
                    self.target = vtt_platform::foreground_target();
                    self.command = command;
                    self.recording.store(true, Ordering::SeqCst);
                    vtt_platform::suppress_menu_activation(self.trigger);
                    self.reporter.set(if command {
                        Status::Command
                    } else {
                        Status::Listening
                    });
                }
                Err(e) => self.reporter.set(Status::Error(e)),
            },
            Some(Action::Command) => {
                self.command = true;
                self.reporter.set(Status::Command);
            }
            Some(Action::HandsFree) => {
                log::info!("mode mains libres");
                self.reporter
                    .set(Status::HandsFree(self.key_name.to_owned()));
            }
            Some(Action::Cancel) => {
                self.recorder.stop();
                self.recording.store(false, Ordering::SeqCst);
                self.reporter.set(Status::Idle);
            }
            Some(Action::Stop) => {
                let clip = self.recorder.stop();
                self.recording.store(false, Ordering::SeqCst);
                self.reporter.set(Status::Processing);
                let _ = self.clips.send(Job {
                    clip,
                    target: std::mem::take(&mut self.target),
                    command: std::mem::take(&mut self.command),
                });
            }
            None => {}
        }
    }
}

/// Les dictées sont traitées une par une, hors du thread clavier, pour qu'une
/// nouvelle dictée puisse commencer pendant la transcription de la précédente.
fn spawn_worker(
    pipeline: Pipeline,
    recorders: Recorders,
    pricing: Pricing,
    reporter: Arc<StatusReporter>,
    recording: Arc<AtomicBool>,
) -> mpsc::Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            for Job {
                clip,
                target,
                command,
            } in rx
            {
                if let Some(app) = &target.app {
                    log::info!("application cible : {app}");
                }
                let result = if command {
                    run_command(&pipeline, &clip, &target)
                } else {
                    pipeline.run(&clip, &target)
                };
                match result {
                    Ok(outcome) => {
                        if let Outcome::Inserted {
                            post_process_error: Some(e),
                            ..
                        } = &outcome
                        {
                            log::warn!("nettoyage ignoré : {e}");
                        }
                        if let Outcome::Inserted {
                            raw,
                            usage,
                            command,
                            ..
                        } = &outcome
                        {
                            // La version brute d'une commande serait la consigne.
                            if !command {
                                raw_paste::remember(&recorders.last_raw, raw);
                            }
                            if let Some(usage) = usage {
                                costs::record(&recorders.ledger, usage, &pricing);
                            }
                        }
                        history::record(&recorders.history, &clip, &target, &outcome);
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

/// Command Mode : récupère la sélection (Ctrl+C simulé, une fois les touches
/// du raccourci relâchées) puis applique la consigne dictée.
fn run_command(
    pipeline: &Pipeline,
    clip: &AudioClip,
    target: &Target,
) -> Result<Outcome, vtt_core::Error> {
    if target.is_terminal() {
        // Refus avant le Ctrl+C simulé, qui interromprait le programme en
        // cours dans une console.
        return pipeline.run_command(clip, target, None);
    }
    if !raw_paste::wait_modifiers_released() {
        return Err(vtt_core::Error::Sink(
            "Command Mode : touches du raccourci toujours enfoncées".into(),
        ));
    }
    let selection = vtt_platform::copy_selection();
    pipeline.run_command(clip, target, selection.as_deref())
}
