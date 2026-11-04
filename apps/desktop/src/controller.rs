//! Chef d'orchestre : charge le modèle, écoute le clavier, pilote le micro
//! et confie chaque enregistrement au pipeline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use vtt_asr::WhisperTranscriber;
use vtt_core::config::{AsrConfig, Pricing, Snippet, Style};
use vtt_core::hotkey::{Action, PushToTalk};
use vtt_core::{AudioClip, Config, Outcome, Pipeline, PostProcessor, Target};
use vtt_platform::audio::Recorder;
use vtt_platform::{ClipboardPaster, MediaPauser, now_ms};

use crate::costs::{self, LedgerStore};
use crate::feedback::{self, Sound};
use crate::history::{self, HistoryStore};
use crate::model;
use crate::paths::Paths;
use crate::raw_paste::{self, LastRaw};
use crate::retry::{ClipboardCopy, Delivery};
use crate::settings;
use crate::status::{Status, StatusReporter};

/// Un enregistrement plus long est coupé et traité (touche restée bloquée…).
const MAX_RECORDING_MS: u64 = 5 * 60 * 1000;
/// Même filet pour le mode mains libres, où les dictées sont plus longues.
const MAX_HANDS_FREE_MS: u64 = 10 * 60 * 1000;
/// Fréquence de vérification de l'état physique du raccourci.
const POLL: Duration = Duration::from_millis(100);
/// Sons et pause de la musique seulement pour un appui confirmé : AltGr sert
/// aussi à taper `@`, `€`… (la dictée, elle, démarre dès l'appui).
const FEEDBACK_MIN_MS: u64 = 300;
/// Rafraîchissement du niveau du micro dans la pastille.
const LEVEL_REFRESH: Duration = Duration::from_millis(50);
/// Délai avant de se fier à l'état physique de la touche : Windows le met à
/// jour après le passage dans le hook.
const KEY_STATE_GRACE_MS: u64 = 300;

/// Ce qui est enregistré après chaque dictée.
pub struct Recorders {
    pub history: HistoryStore,
    pub ledger: LedgerStore,
    pub last_raw: LastRaw,
}

/// Travail confié au thread de traitement.
pub enum Work {
    Dictation(Job),
    /// Dictionnaire, snippets ou styles modifiés dans la fenêtre principale.
    Reconfigure(Reconfiguration),
    /// Retraiter l'audio de la dernière dictée (« Réessayer »).
    Retry(Delivery),
    /// Une dictée commence : recharger le modèle s'il a été libéré, pendant
    /// que l'utilisateur parle.
    Preload,
}

/// Fréquence de vérification de la libération du modèle.
const UNLOAD_CHECK: Duration = Duration::from_secs(15);
/// En plein écran, le modèle reste chargé une minute après une dictée, pour
/// ne pas le recharger à chaque message dicté dans un jeu.
const FULLSCREEN_GRACE: Duration = Duration::from_secs(60);

/// Quand libérer la mémoire graphique occupée par Whisper.
#[derive(Debug, Clone, Copy, Default)]
struct UnloadPolicy {
    /// Après ce délai sans dicter.
    idle: Option<Duration>,
    /// Pendant qu'une application est en plein écran.
    fullscreen: bool,
}

impl UnloadPolicy {
    fn from_config(asr: &AsrConfig) -> Self {
        Self {
            idle: (asr.unload_after_min > 0)
                .then(|| Duration::from_secs(u64::from(asr.unload_after_min) * 60)),
            fullscreen: asr.unload_in_fullscreen,
        }
    }

    /// Raison de libérer le modèle maintenant, `idle` étant le temps écoulé
    /// depuis la dernière dictée. `fullscreen` n'est appelé que si besoin.
    fn reason(&self, idle: Duration, fullscreen: impl FnOnce() -> bool) -> Option<&'static str> {
        if self.idle.is_some_and(|after| idle >= after) {
            Some("aucune dictée récente")
        } else if self.fullscreen && idle >= FULLSCREEN_GRACE && fullscreen() {
            Some("application en plein écran")
        } else {
            None
        }
    }
}

pub struct Reconfiguration {
    pub dictionary: Vec<String>,
    pub snippets: Vec<Snippet>,
    pub styles: Vec<Style>,
    /// Nettoyage reconstruit (nouveau fournisseur ou dictionnaire ; `None` :
    /// désactivé).
    pub post_processor: Option<Box<dyn PostProcessor>>,
    /// Tarifs des fournisseurs, pour le coût des prochaines dictées.
    pub tariffs: Tariffs,
}

/// Tarif du fournisseur principal et de celui du profil de secours.
#[derive(Debug, Clone)]
pub struct Tariffs {
    main: Pricing,
    fallback: Option<Pricing>,
}

impl Tariffs {
    pub fn from_config(config: &Config) -> Self {
        Self {
            main: config.llm.pricing.clone(),
            fallback: config
                .fallback_for(&config.llm)
                .map(|p| p.llm.pricing.clone()),
        }
    }

    /// Tarif du fournisseur qui a répondu.
    fn for_outcome(&self, outcome: &Outcome) -> &Pricing {
        match (outcome, &self.fallback) {
            (
                Outcome::Inserted {
                    fallback: Some(_), ..
                },
                Some(fallback),
            ) => fallback,
            _ => &self.main,
        }
    }
}

/// Applique `config` (dictionnaire, snippets, styles, fournisseur LLM) au
/// thread de traitement, sans recharger le modèle. `false` si le modèle est
/// encore en chargement : la configuration enregistrée sera lue au démarrage
/// suivant.
pub fn reconfigure(worker: &WorkerHandle, config: &Config) -> bool {
    let sender = worker.lock().unwrap_or_else(|e| e.into_inner()).clone();
    sender.is_some_and(|sender| {
        sender
            .send(Work::Reconfigure(Reconfiguration {
                dictionary: config.dictionary.clone(),
                snippets: config.snippets.clone(),
                styles: config.styles.clone(),
                post_processor: settings::build_cleaner(config),
                tariffs: Tariffs::from_config(config),
            }))
            .is_ok()
    })
}

/// Accès au thread de traitement depuis l'interface ; `None` tant que le
/// modèle n'est pas chargé.
pub type WorkerHandle = Arc<Mutex<Option<mpsc::Sender<Work>>>>;

pub fn spawn(
    reporter: Arc<StatusReporter>,
    recorders: Recorders,
    worker: WorkerHandle,
    config: Config,
    paths: Paths,
) {
    thread::Builder::new()
        .name("controller".into())
        .spawn(move || {
            if let Err(e) = run(&reporter, recorders, &worker, &config, &paths) {
                reporter.set(Status::Error(e));
            }
        })
        .expect("création du thread contrôleur");
}

fn run(
    reporter: &Arc<StatusReporter>,
    recorders: Recorders,
    worker: &WorkerHandle,
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
    let cleaner = settings::build_cleaner(config);
    match &cleaner {
        Some(_) => log::info!(
            "nettoyage : {} via {}",
            config.llm.model,
            config.llm.base_url
        ),
        None => log::info!("nettoyage désactivé (llm.enabled = false)"),
    }
    let pipeline = Pipeline::new(Box::new(transcriber), cleaner, Box::new(ClipboardPaster))
        .with_snippets(&config.snippets)
        .with_styles(&config.styles)
        .with_voice_commands(config.dictation.voice_commands);
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
        Tariffs::from_config(config),
        UnloadPolicy::from_config(&config.asr),
        config.hotkey.retry.trim().to_owned(),
        Arc::clone(reporter),
        Arc::clone(&recording),
    );
    *worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(clips.clone());
    let recorder =
        Recorder::spawn(Some(config.audio.device.trim().to_owned()).filter(|d| !d.is_empty()));
    spawn_level_meter(
        recorder.clone(),
        Arc::clone(&recording),
        Arc::clone(reporter),
    );
    let mut controls = Controls {
        recorder,
        sounds: config.dictation.sounds,
        media: config.dictation.pause_media.then(MediaPauser::spawn),
        confirmed: false,
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
        let feedback_after = u64::from(config.hotkey.min_hold_ms).max(FEEDBACK_MIN_MS);
        if ptt
            .recording_ms(now_ms())
            .is_some_and(|elapsed| elapsed >= feedback_after)
        {
            controls.confirm();
        }

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

/// Envoie le niveau du micro à la pastille pendant l'enregistrement.
fn spawn_level_meter(
    recorder: Recorder,
    recording: Arc<AtomicBool>,
    reporter: Arc<StatusReporter>,
) {
    thread::Builder::new()
        .name("level".into())
        .spawn(move || {
            loop {
                thread::sleep(LEVEL_REFRESH);
                if recording.load(Ordering::Relaxed) {
                    reporter.level(recorder.level());
                }
            }
        })
        .expect("création du thread niveau micro");
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
#[derive(Clone)]
pub struct Job {
    clip: AudioClip,
    target: Target,
    /// Command Mode : la dictée est une consigne pour le texte sélectionné.
    command: bool,
}

/// Ce que le thread clavier pilote : micro, état affiché, file des dictées.
struct Controls<'a> {
    recorder: Recorder,
    /// Sons de début et de fin (`dictation.sounds`).
    sounds: bool,
    /// Pause de la musique pendant la dictée (`dictation.pause_media`).
    media: Option<MediaPauser>,
    /// Appui assez long pour être une vraie dictée : son de début joué,
    /// musique en pause.
    confirmed: bool,
    recording: Arc<AtomicBool>,
    reporter: &'a StatusReporter,
    clips: mpsc::Sender<Work>,
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
                    let _ = self.clips.send(Work::Preload);
                    self.confirmed = false;
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
                // Le second appui du double appui est court : confirmé ici.
                self.confirm();
                log::info!("mode mains libres");
                self.reporter
                    .set(Status::HandsFree(self.key_name.to_owned()));
            }
            Some(Action::Cancel) => {
                self.recorder.stop();
                self.recording.store(false, Ordering::SeqCst);
                self.finish(Sound::Cancel);
                self.reporter.set(Status::Idle);
            }
            Some(Action::Stop) => {
                let clip = self.recorder.stop();
                self.recording.store(false, Ordering::SeqCst);
                self.finish(Sound::Stop);
                self.reporter.set(Status::Processing);
                let _ = self.clips.send(Work::Dictation(Job {
                    clip,
                    target: std::mem::take(&mut self.target),
                    command: std::mem::take(&mut self.command),
                }));
            }
            None => {}
        }
    }
}

impl Controls<'_> {
    /// Appui confirmé : son de début, musique en pause (une seule fois).
    fn confirm(&mut self) {
        if self.confirmed || !self.recording.load(Ordering::SeqCst) {
            return;
        }
        self.confirmed = true;
        if self.sounds {
            feedback::play(Sound::Start);
        }
        if let Some(media) = &self.media {
            media.pause();
        }
    }

    /// Fin d'une dictée confirmée : son de fin, musique relancée.
    fn finish(&mut self, sound: Sound) {
        if !std::mem::take(&mut self.confirmed) {
            return;
        }
        if self.sounds {
            feedback::play(sound);
        }
        if let Some(media) = &self.media {
            media.resume();
        }
    }
}

/// Les dictées sont traitées une par une, hors du thread clavier, pour qu'une
/// nouvelle dictée puisse commencer pendant la transcription de la précédente.
///
/// L'audio de la dernière dictée reste en mémoire (jamais sur disque) pour
/// « Réessayer » : après une erreur, rien n'est perdu.
fn spawn_worker(
    mut pipeline: Pipeline,
    recorders: Recorders,
    mut tariffs: Tariffs,
    unload: UnloadPolicy,
    retry_shortcut: String,
    reporter: Arc<StatusReporter>,
    recording: Arc<AtomicBool>,
) -> mpsc::Sender<Work> {
    let (tx, rx) = mpsc::channel::<Work>();
    thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            let mut last: Option<Job> = None;
            let mut last_activity = Instant::now();
            loop {
                let work = match rx.recv_timeout(UNLOAD_CHECK) {
                    Ok(work) => work,
                    Err(RecvTimeoutError::Timeout) => {
                        if !recording.load(Ordering::SeqCst)
                            && let Some(reason) = unload.reason(
                                last_activity.elapsed(),
                                vtt_platform::fullscreen_app_running,
                            )
                            && pipeline.unload_model()
                        {
                            log::info!("modèle déchargé ({reason}) : mémoire graphique libérée");
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if !matches!(work, Work::Reconfigure(_)) {
                    last_activity = Instant::now();
                }
                let (job, delivery) = match work {
                    Work::Dictation(job) => (job, None),
                    Work::Preload => {
                        if let Err(e) = pipeline.preload_model() {
                            reporter.set(Status::Error(e.to_string()));
                        }
                        continue;
                    }
                    Work::Retry(delivery) => match &last {
                        Some(job) => {
                            if !recording.load(Ordering::SeqCst) {
                                reporter.set(Status::Processing);
                            }
                            (job.clone(), Some(delivery))
                        }
                        None => {
                            reporter.set(Status::Error(
                                "aucune dictée à réessayer depuis le démarrage".into(),
                            ));
                            continue;
                        }
                    },
                    Work::Reconfigure(r) => {
                        pipeline.reconfigure(
                            &r.dictionary,
                            &r.snippets,
                            &r.styles,
                            r.post_processor,
                        );
                        tariffs = r.tariffs;
                        log::info!(
                            "configuration appliquée : {} mot(s), {} snippet(s), {} style(s)",
                            r.dictionary.len(),
                            r.snippets.len(),
                            r.styles.len()
                        );
                        continue;
                    }
                };
                // Réessayée par le raccourci : collée là où se trouve le
                // curseur maintenant (le style suit cette fenêtre).
                let target = match delivery {
                    Some(Delivery::Paste) => vtt_platform::foreground_target(),
                    _ => job.target.clone(),
                };
                let result = process(&pipeline, &job, &target, delivery, &retry_shortcut);
                let failed = result.is_err();
                // Un appui accidentel (silence, clip trop court) ne doit pas
                // remplacer la dictée à réessayer.
                // « Annule la dernière dictée » ne se réessaie pas non plus.
                let keep = !matches!(result, Ok(Outcome::Skipped | Outcome::Undone));
                match result {
                    Ok(outcome) => {
                        // Une dictée réessayée n'est comptée qu'une fois dans
                        // les statistiques ; son coût, lui, est bien réel.
                        let stats = delivery.is_none();
                        let pricing = tariffs.for_outcome(&outcome);
                        record(&recorders, &job, &target, &outcome, pricing, stats);
                        if delivery.is_some() && outcome == Outcome::Skipped {
                            reporter.set(Status::Error(
                                "la dernière dictée ne contenait aucune parole".into(),
                            ));
                            continue;
                        }
                        if delivery == Some(Delivery::Clipboard)
                            && matches!(outcome, Outcome::Inserted { .. })
                        {
                            reporter.notify(
                                "VoiceToText : dictée retraitée",
                                "Le texte est dans le presse-papier : Ctrl+V pour le coller.",
                            );
                        }
                    }
                    Err(e) => {
                        let hint = if retry_shortcut.is_empty() {
                            String::new()
                        } else {
                            format!(" ({retry_shortcut} pour réessayer)")
                        };
                        reporter.set(Status::Error(format!("{e}{hint}")));
                    }
                }
                if delivery.is_none() && keep {
                    last = Some(job);
                }
                if !recording.load(Ordering::SeqCst) && !failed {
                    reporter.set(Status::Idle);
                }
            }
        })
        .expect("création du thread pipeline");
    tx
}

/// Traite une dictée. `delivery` : `None` pour une nouvelle dictée, sinon
/// « Réessayer » (collée dans la fenêtre active, ou copiée seulement).
fn process(
    pipeline: &Pipeline,
    job: &Job,
    target: &Target,
    delivery: Option<Delivery>,
    retry_shortcut: &str,
) -> Result<Outcome, vtt_core::Error> {
    if let Some(app) = &target.app {
        log::info!("application cible : {app}");
    }
    if delivery.is_some() {
        log::info!("nouvel essai de la dernière dictée");
    }
    match (job.command, delivery) {
        (true, Some(Delivery::Clipboard)) => {
            Err(vtt_core::Error::Sink(if retry_shortcut.is_empty() {
                "une commande se réessaie avec le raccourci « Réessayer »".into()
            } else {
                format!("une commande se réessaie avec {retry_shortcut}, texte sélectionné")
            }))
        }
        (true, _) => run_command(pipeline, &job.clip, target),
        (false, Some(Delivery::Clipboard)) => pipeline.run_to(&job.clip, target, &ClipboardCopy),
        (false, _) => pipeline.run(&job.clip, target),
    }
}

/// Historique, coût et statistiques d'une dictée traitée.
fn record(
    recorders: &Recorders,
    job: &Job,
    target: &Target,
    outcome: &Outcome,
    pricing: &Pricing,
    stats: bool,
) {
    let Outcome::Inserted {
        raw,
        usage,
        command,
        post_process_error,
        ..
    } = outcome
    else {
        return;
    };
    if let Some(e) = post_process_error {
        log::warn!("nettoyage ignoré : {e}");
    }
    // La version brute d'une commande serait la consigne.
    if !command {
        raw_paste::remember(&recorders.last_raw, raw);
    }
    if let Some(usage) = usage {
        costs::record(&recorders.ledger, usage, pricing);
    }
    if stats {
        let words = raw.split_whitespace().count() as u64;
        costs::record_dictation(&recorders.ledger, words, job.clip.duration_ms(), *command);
    }
    history::record(&recorders.history, &job.clip, target, outcome, pricing);
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

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn policy(idle_min: u32, fullscreen: bool) -> UnloadPolicy {
        UnloadPolicy::from_config(&AsrConfig {
            unload_after_min: idle_min,
            unload_in_fullscreen: fullscreen,
            ..AsrConfig::default()
        })
    }

    #[test]
    fn never_unloads_by_default() {
        let p = UnloadPolicy::from_config(&AsrConfig::default());
        assert_eq!(p.reason(MIN * 600, || true), None);
    }

    #[test]
    fn unloads_after_idle_delay() {
        let p = policy(15, false);
        assert_eq!(p.reason(MIN * 14, || true), None);
        assert!(p.reason(MIN * 15, || false).is_some());
    }

    #[test]
    fn unloads_in_fullscreen_after_grace() {
        let p = policy(0, true);
        assert_eq!(p.reason(MIN * 5, || false), None, "pas de plein écran");
        assert_eq!(
            p.reason(Duration::from_secs(20), || true),
            None,
            "dictée récente"
        );
        assert!(p.reason(MIN * 2, || true).is_some());
    }

    #[test]
    fn fullscreen_is_not_queried_when_disabled() {
        let p = policy(0, false);
        assert_eq!(p.reason(MIN * 5, || panic!("appel inutile")), None);
    }
}
