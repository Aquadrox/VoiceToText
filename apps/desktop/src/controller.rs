//! Chef d'orchestre : charge le modèle, écoute le clavier, pilote le micro
//! et confie chaque enregistrement au pipeline.
//!
//! Les réglages modifiés dans l'interface sont appliqués sans redémarrer :
//! le thread clavier les prend entre deux dictées, le thread de traitement
//! les prend aussitôt (et change de modèle si besoin).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use vtt_core::config::{AsrConfig, HotkeyConfig, Pricing, Snippet, Style};
use vtt_core::hotkey::{Action, PushToTalk};
use vtt_core::{AudioClip, Config, Outcome, Pipeline, PostProcessor, Target, TextSink, Word};
use vtt_platform::audio::Recorder;
use vtt_platform::{ClipboardPaster, MediaPauser, now_ms};

use crate::costs::{self, LedgerStore};
use crate::dictation_report;
use crate::feedback::{self, Sound};
use crate::history::{self, HistoryStore};
use crate::meetings::MeetingQueue;
use crate::model;
use crate::paths::Paths;
use crate::raw_paste::{self, LastRaw};
use crate::retry::{ClipboardCopy, Delivery};
use crate::settings;
use crate::status::{Status, StatusReporter};
use crate::translation::TranslationState;

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

/// Ce qui est enregistré après chaque dictée, et la file des blocs de
/// réunion à transcrire entre deux dictées.
#[derive(Clone)]
pub struct Recorders {
    pub history: HistoryStore,
    pub ledger: LedgerStore,
    pub last_raw: LastRaw,
    pub meeting_jobs: MeetingQueue,
}

/// Travail confié au thread de traitement.
pub enum Work {
    Dictation(Job),
    /// Réglages modifiés dans la fenêtre principale.
    Reconfigure(Box<Reconfiguration>),
    /// Retraiter l'audio de la dernière dictée (« Réessayer »).
    Retry(Delivery),
    /// Une dictée commence : recharger le modèle s'il a été libéré, pendant
    /// que l'utilisateur parle.
    Preload,
    /// Le modèle choisi dans les réglages vient d'être téléchargé.
    ModelReady,
    /// Des blocs de réunion attendent dans `Recorders::meeting_jobs`.
    MeetingWake,
    /// Mots horodatés d'un clip (capture commentée), renvoyés par `reply`.
    Transcribe(AudioClip, Sender<Result<Vec<Word>, String>>),
}

/// Fréquence de vérification de la libération du modèle.
const UNLOAD_CHECK: Duration = Duration::from_secs(15);
/// Blocs de réunion en attente pendant une dictée : revérifié souvent.
const MEETING_CHECK: Duration = Duration::from_millis(200);
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

/// Réglages du thread de traitement, appliqués sans redémarrer.
pub struct Reconfiguration {
    dictionary: Vec<String>,
    snippets: Vec<Snippet>,
    styles: Vec<Style>,
    /// Nettoyage reconstruit (nouveau fournisseur ou dictionnaire ; `None` :
    /// désactivé).
    post_processor: Option<Box<dyn PostProcessor>>,
    /// Tarifs des fournisseurs, pour le coût des prochaines dictées.
    tariffs: Tariffs,
    voice_commands: bool,
    unload: UnloadPolicy,
    retry_shortcut: String,
    /// Modèle de transcription demandé (rechargé s'il change).
    asr: AsrConfig,
}

impl Reconfiguration {
    fn from_config(config: &Config) -> Self {
        Self {
            dictionary: config.dictionary.clone(),
            snippets: config.snippets.clone(),
            styles: config.styles.clone(),
            post_processor: settings::build_cleaner(config),
            tariffs: Tariffs::from_config(config),
            voice_commands: config.dictation.voice_commands,
            unload: UnloadPolicy::from_config(&config.asr),
            retry_shortcut: config.hotkey.retry.trim().to_owned(),
            asr: config.asr.clone(),
        }
    }
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

/// Accès au thread de traitement depuis l'interface et les raccourcis ;
/// `None` avant le démarrage.
pub type WorkerHandle = Arc<Mutex<Option<Sender<Work>>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Threads clavier et traitement, et de quoi les relancer. Partagé avec
/// l'interface, qui y applique les réglages modifiés.
pub struct Controller {
    reporter: Arc<StatusReporter>,
    recorders: Recorders,
    worker: WorkerHandle,
    /// Réglages à appliquer par le thread clavier ; `None` avant le démarrage.
    keyboard: Mutex<Option<Sender<Config>>>,
    paths: Paths,
    translation: TranslationState,
}

pub type ControllerState = Arc<Controller>;

impl Controller {
    pub fn new(
        reporter: Arc<StatusReporter>,
        recorders: Recorders,
        worker: WorkerHandle,
        paths: Paths,
        translation: TranslationState,
    ) -> ControllerState {
        Arc::new(Self {
            reporter,
            recorders,
            worker,
            keyboard: Mutex::new(None),
            paths,
            translation,
        })
    }

    /// Démarre les threads clavier et traitement (au lancement, ou après un
    /// arrêt sur erreur).
    pub fn start(&self, config: Config) {
        // Créés tout de suite : les réglages envoyés pendant le chargement du
        // modèle sont appliqués dès qu'il est prêt.
        let (work_tx, work_rx) = mpsc::channel();
        let (settings_tx, settings_rx) = mpsc::channel();
        *lock(&self.worker) = Some(work_tx.clone());
        *lock(&self.keyboard) = Some(settings_tx);
        let reporter = Arc::clone(&self.reporter);
        let recorders = self.recorders.clone();
        let worker = Arc::clone(&self.worker);
        let paths = self.paths.clone();
        let translation = Arc::clone(&self.translation);
        thread::Builder::new()
            .name("controller".into())
            .spawn(move || {
                let channels = Channels {
                    work_tx,
                    work_rx,
                    settings: settings_rx,
                    translation,
                };
                if let Err(e) = run(&reporter, recorders, &worker, channels, &config, &paths) {
                    reporter.set(Status::Error(e));
                }
            })
            .expect("création du thread contrôleur");
    }

    /// Applique `config` sans redémarrer. Si les threads se sont arrêtés sur
    /// une erreur (modèle introuvable, touche inconnue…), ils sont relancés
    /// avec cette configuration.
    pub fn apply(&self, config: &Config) {
        self.translation.set_language(&config.translation.language);
        if let Some(worker) = lock(&self.worker).clone() {
            let reconfiguration = Reconfiguration::from_config(config);
            let _ = worker.send(Work::Reconfigure(Box::new(reconfiguration)));
        }
        // Le thread clavier vit tant que tout va bien : c'est lui qui compte.
        let keyboard = lock(&self.keyboard).clone();
        if !keyboard.is_some_and(|k| k.send(config.clone()).is_ok()) {
            log::warn!("dictée arrêtée : relance avec la nouvelle configuration");
            self.start(config.clone());
        }
    }
}

/// Canaux du thread clavier : file des dictées et réglages à appliquer.
struct Channels {
    work_tx: Sender<Work>,
    work_rx: Receiver<Work>,
    settings: Receiver<Config>,
    translation: TranslationState,
}

/// Machine à états du raccourci, d'après la configuration.
fn push_to_talk(hotkey: &HotkeyConfig) -> Result<PushToTalk, String> {
    let trigger = vtt_platform::key_code(&hotkey.key)
        .ok_or_else(|| format!("touche inconnue dans config.toml : {:?}", hotkey.key))?;
    Ok(PushToTalk::new(trigger, hotkey.min_hold_ms.into())
        .with_ignored(vtt_platform::companion_keys(trigger))
        .with_hands_free(hotkey.hands_free, vtt_platform::key_code("Escape"))
        .with_command_key(command_key(hotkey)))
}

/// Micro choisi (`None` : micro par défaut de Windows).
fn device(config: &Config) -> Option<String> {
    Some(config.audio.device.trim().to_owned()).filter(|d| !d.is_empty())
}

fn run(
    reporter: &Arc<StatusReporter>,
    recorders: Recorders,
    worker: &WorkerHandle,
    channels: Channels,
    config: &Config,
    paths: &Paths,
) -> Result<(), String> {
    let mut ptt = push_to_talk(&config.hotkey)?;

    let installed = model::ensure(&paths.models_dir, &config.asr.model, |percent| {
        reporter.set(Status::Loading(format!(
            "Téléchargement du modèle : {percent} %"
        )));
    })?;
    reporter.set(Status::Loading("Chargement du modèle…".into()));
    let transcriber = model::load(&installed, &config.asr, &config.dictionary)?;
    let cleaner = settings::build_cleaner(config);
    match &cleaner {
        Some(_) => log::info!(
            "nettoyage : {} via {}",
            config.llm.model,
            config.llm.base_url
        ),
        None => log::info!("nettoyage désactivé (llm.enabled = false)"),
    }
    let pipeline = Pipeline::new(transcriber, cleaner, Box::new(ClipboardPaster))
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
    let worker_thread = Worker {
        pipeline,
        recorders,
        tariffs: Tariffs::from_config(config),
        unload: UnloadPolicy::from_config(&config.asr),
        retry_shortcut: config.hotkey.retry.trim().to_owned(),
        reporter: Arc::clone(reporter),
        recording: Arc::clone(&recording),
        model: ModelSlot {
            models_dir: paths.models_dir.clone(),
            loaded: config.asr.clone(),
            wanted: config.asr.clone(),
            dictionary: config.dictionary.clone(),
            downloading: Arc::default(),
            worker: Arc::clone(worker),
        },
    };
    worker_thread.spawn(channels.work_rx);
    let recorder = Recorder::spawn(device(config));
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
        clips: channels.work_tx,
        trigger: ptt.trigger(),
        key_name: config.hotkey.key.clone(),
        target: Target::default(),
        command: false,
        translation: channels.translation,
        translate_to: None,
    };

    let (keys_tx, keys) = mpsc::channel();
    vtt_platform::spawn_keyboard_hook(keys_tx)?;
    log::info!("prêt, raccourci : {}", config.hotkey.key);
    reporter.set(Status::Idle);

    let mut hotkey = config.hotkey.clone();
    // Réglages reçus pendant une dictée : appliqués à la fin de celle-ci.
    let mut pending: Option<Config> = None;
    loop {
        let action = match keys.recv_timeout(POLL) {
            Ok(event) => ptt.on_key(event.key, event.down, event.at_ms),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Err("hook clavier arrêté".into()),
        };
        controls.handle(action, ptt.is_command());
        let feedback_after = u64::from(hotkey.min_hold_ms).max(FEEDBACK_MIN_MS);
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
                let released =
                    elapsed > KEY_STATE_GRACE_MS && !vtt_platform::key_is_down(ptt.trigger());
                (released, MAX_RECORDING_MS)
            };
            if released || elapsed > max_ms {
                log::info!("relâchement forcé après {elapsed} ms");
                controls.handle(ptt.force_release(now), false);
            }
        }

        if let Some(latest) = channels.settings.try_iter().last() {
            pending = Some(latest);
        }
        if !ptt.is_recording()
            && let Some(latest) = pending.take()
        {
            if latest.hotkey != hotkey {
                match push_to_talk(&latest.hotkey) {
                    Ok(new) => {
                        ptt = new;
                        hotkey = latest.hotkey.clone();
                        log::info!("raccourci : {}", hotkey.key);
                    }
                    Err(e) => log::error!("{e}"),
                }
            }
            controls.reconfigure(&latest, ptt.trigger(), &hotkey.key);
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
fn command_key(hotkey: &HotkeyConfig) -> Option<u32> {
    let name = hotkey.command_key.trim();
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
    /// Mode traduction actif au début de la dictée : langue d'arrivée.
    translate_to: Option<String>,
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
    clips: Sender<Work>,
    trigger: u32,
    key_name: String,
    /// Fenêtre de destination, relevée au début de la dictée (le focus peut
    /// changer pendant la transcription).
    target: Target,
    /// La dictée en cours est une commande.
    command: bool,
    translation: TranslationState,
    /// Langue d'arrivée de la dictée en cours (mode traduction).
    translate_to: Option<String>,
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
                    self.translate_to = self.translation.current();
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
                self.reporter.set(Status::HandsFree(self.key_name.clone()));
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
                    translate_to: self.translate_to.take(),
                }));
            }
            None => {}
        }
    }

    /// Réglages modifiés : raccourci, micro, sons, pause de la musique.
    fn reconfigure(&mut self, config: &Config, trigger: u32, key_name: &str) {
        self.trigger = trigger;
        self.key_name = key_name.to_owned();
        self.recorder.set_device(device(config));
        self.sounds = config.dictation.sounds;
        if config.dictation.pause_media != self.media.is_some() {
            self.media = config.dictation.pause_media.then(MediaPauser::spawn);
        }
    }

    /// Appui confirmé : son de début, musique en pause (une seule fois).
    fn confirm(&mut self) {
        if self.confirmed || !self.recording.load(Ordering::SeqCst) {
            return;
        }
        self.confirmed = true;
        if self.sounds {
            feedback::play(Sound::Start);
        }
        // Pendant une réunion, la pause couperait l'appel (Meet, Teams web…).
        if let Some(media) = &self.media
            && !crate::meetings::recording()
        {
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

/// Modèle de transcription en place, et celui demandé dans les réglages.
struct ModelSlot {
    models_dir: PathBuf,
    /// Réglages du modèle chargé.
    loaded: AsrConfig,
    /// Réglages demandés (le modèle peut être en cours de téléchargement).
    wanted: AsrConfig,
    /// Dictionnaire, donné au nouveau modèle.
    dictionary: Vec<String>,
    downloading: Arc<AtomicBool>,
    /// Pour signaler la fin d'un téléchargement au thread de traitement.
    worker: WorkerHandle,
}

/// Même modèle, même langue, même choix de carte graphique.
fn same_model(a: &AsrConfig, b: &AsrConfig) -> bool {
    a.model == b.model && a.use_gpu == b.use_gpu && a.language == b.language
}

impl ModelSlot {
    /// Charge le modèle demandé s'il diffère du modèle en place. S'il est
    /// absent, il est d'abord téléchargé en arrière-plan : les dictées
    /// continuent avec l'ancien modèle en attendant.
    fn sync(
        &mut self,
        pipeline: &mut Pipeline,
        reporter: &Arc<StatusReporter>,
        recording: &Arc<AtomicBool>,
    ) {
        if same_model(&self.loaded, &self.wanted) {
            return;
        }
        let Some(installed) = model::local(&self.models_dir, &self.wanted.model) else {
            self.download(reporter, recording);
            return;
        };
        let asr = &self.wanted;
        reporter.set(Status::Loading("Chargement du modèle…".into()));
        // Libérer la mémoire graphique avant de charger le nouveau modèle.
        pipeline.unload_model();
        match model::load(&installed, asr, &self.dictionary) {
            Ok(transcriber) => {
                pipeline.set_transcriber(transcriber);
                let device = match model::engine(&asr.model) {
                    model::Engine::Whisper if asr.use_gpu => "carte graphique",
                    _ => "processeur",
                };
                log::info!("modèle changé : {} ({device})", asr.model);
                self.loaded = asr.clone();
                if !recording.load(Ordering::SeqCst) {
                    reporter.set(Status::Idle);
                }
            }
            // L'ancien modèle reste en place (rechargé à la dictée suivante).
            Err(e) => reporter.set(Status::Error(format!(
                "chargement du modèle {} : {e}",
                asr.model
            ))),
        }
    }

    /// Télécharge le modèle demandé, un seul à la fois : à la fin, le modèle
    /// demandé est revérifié (il a pu changer entre-temps).
    fn download(&self, reporter: &Arc<StatusReporter>, recording: &Arc<AtomicBool>) {
        if self.downloading.swap(true, Ordering::SeqCst) {
            return;
        }
        let dir = self.models_dir.clone();
        let file = self.wanted.model.clone();
        let downloading = Arc::clone(&self.downloading);
        let worker = Arc::clone(&self.worker);
        let reporter = Arc::clone(reporter);
        let recording = Arc::clone(recording);
        thread::Builder::new()
            .name("model-download".into())
            .spawn(move || {
                let result = model::ensure(&dir, &file, |percent| {
                    // Ne pas masquer « Écoute… » pendant une dictée.
                    if !recording.load(Ordering::SeqCst) {
                        reporter.set(Status::Loading(format!(
                            "Téléchargement du modèle : {percent} %"
                        )));
                    }
                });
                downloading.store(false, Ordering::SeqCst);
                match result {
                    Ok(_) => {
                        if let Some(worker) = lock(&worker).clone() {
                            let _ = worker.send(Work::ModelReady);
                        }
                    }
                    Err(e) => reporter.set(Status::Error(e)),
                }
            })
            .expect("création du thread de téléchargement");
    }
}

/// Thread de traitement : les dictées sont traitées une par une, hors du
/// thread clavier, pour qu'une nouvelle dictée puisse commencer pendant la
/// transcription de la précédente.
///
/// L'audio de la dernière dictée reste en mémoire (jamais sur disque) pour
/// « Réessayer » : après une erreur, rien n'est perdu.
struct Worker {
    pipeline: Pipeline,
    recorders: Recorders,
    tariffs: Tariffs,
    unload: UnloadPolicy,
    retry_shortcut: String,
    reporter: Arc<StatusReporter>,
    recording: Arc<AtomicBool>,
    model: ModelSlot,
}

impl Worker {
    fn spawn(self, rx: Receiver<Work>) {
        thread::Builder::new()
            .name("pipeline".into())
            .spawn(move || self.run(rx))
            .expect("création du thread pipeline");
    }

    fn run(mut self, rx: Receiver<Work>) {
        let mut last: Option<Job> = None;
        let mut last_activity = Instant::now();
        loop {
            // Les dictées passent avant les blocs de réunion : un bloc n'est
            // transcrit que si aucune dictée n'attend ni n'est en cours.
            let waiting = match rx.try_recv() {
                Ok(work) => Some(work),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => break,
            };
            if waiting.is_none() && !self.recording.load(Ordering::SeqCst) {
                let job = lock(&self.recorders.meeting_jobs).pop_front();
                if let Some(job) = job {
                    last_activity = Instant::now();
                    job.run(&self.pipeline);
                    continue;
                }
            }
            let pending_meeting = !lock(&self.recorders.meeting_jobs).is_empty();
            let timeout = if pending_meeting {
                MEETING_CHECK
            } else {
                UNLOAD_CHECK
            };
            let work = match waiting.map_or_else(|| rx.recv_timeout(timeout), Ok) {
                Ok(work) => work,
                Err(RecvTimeoutError::Timeout) if pending_meeting => continue,
                Err(RecvTimeoutError::Timeout) => {
                    if !self.recording.load(Ordering::SeqCst)
                        && let Some(reason) = self.unload.reason(
                            last_activity.elapsed(),
                            vtt_platform::fullscreen_app_running,
                        )
                        && self.pipeline.unload_model()
                    {
                        log::info!("modèle déchargé ({reason}) : mémoire graphique libérée");
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if matches!(work, Work::Dictation(_) | Work::Retry(_) | Work::Preload) {
                last_activity = Instant::now();
            }
            let (job, delivery) = match work {
                Work::Dictation(job) => (job, None),
                Work::Preload => {
                    if let Err(e) = self.pipeline.preload_model() {
                        self.reporter.set(Status::Error(e.to_string()));
                    }
                    continue;
                }
                Work::Retry(delivery) => match &last {
                    Some(job) => {
                        if !self.recording.load(Ordering::SeqCst) {
                            self.reporter.set(Status::Processing);
                        }
                        (job.clone(), Some(delivery))
                    }
                    None => {
                        self.reporter.set(Status::Error(
                            "aucune dictée à réessayer depuis le démarrage".into(),
                        ));
                        continue;
                    }
                },
                Work::Reconfigure(r) => {
                    self.reconfigure(*r);
                    continue;
                }
                Work::ModelReady => {
                    self.model
                        .sync(&mut self.pipeline, &self.reporter, &self.recording);
                    continue;
                }
                Work::MeetingWake => continue,
                Work::Transcribe(clip, reply) => {
                    last_activity = Instant::now();
                    let words = self.pipeline.transcribe_words(&clip);
                    let _ = reply.send(words.map_err(|e| e.to_string()));
                    continue;
                }
            };
            // Réessayée par le raccourci : collée là où se trouve le
            // curseur maintenant (le style suit cette fenêtre).
            let target = match delivery {
                Some(Delivery::Paste) => vtt_platform::foreground_target(),
                _ => job.target.clone(),
            };
            let started = Instant::now();
            let result = process(
                &self.pipeline,
                &job,
                &target,
                delivery,
                &self.retry_shortcut,
            );
            self.report(&result, &job, &target, delivery.is_some(), started);
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
                    let pricing = self.tariffs.for_outcome(&outcome);
                    record(&self.recorders, &job, &target, &outcome, pricing, stats);
                    if delivery.is_some() && outcome == Outcome::Skipped {
                        self.reporter.set(Status::Error(
                            "la dernière dictée ne contenait aucune parole".into(),
                        ));
                        continue;
                    }
                    if delivery == Some(Delivery::Clipboard)
                        && matches!(outcome, Outcome::Inserted { .. })
                    {
                        self.reporter.notify(
                            "VoiceToText : dictée retraitée",
                            "Le texte est dans le presse-papier : Ctrl+V pour le coller.",
                        );
                    }
                }
                Err(e) => {
                    let hint = if self.retry_shortcut.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} pour réessayer)", self.retry_shortcut)
                    };
                    self.reporter.set(Status::Error(format!("{e}{hint}")));
                }
            }
            if delivery.is_none() && keep {
                last = Some(job);
            }
            if !self.recording.load(Ordering::SeqCst) && !failed {
                self.reporter.set(Status::Idle);
            }
        }
    }

    /// Zone de test des réglages : résultat et durées de la dictée.
    fn report(
        &self,
        result: &Result<Outcome, vtt_core::Error>,
        job: &Job,
        target: &Target,
        retry: bool,
        started: Instant,
    ) {
        let pricing = match result {
            Ok(outcome) => self.tariffs.for_outcome(outcome),
            Err(_) => &self.tariffs.main,
        };
        let measures = dictation_report::Measures {
            audio_ms: job.clip.duration_ms(),
            asr_ms: self.pipeline.last_asr_ms(),
            total_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            model: &self.model.loaded.model,
            command: job.command,
            retry,
        };
        self.reporter
            .dictation(&dictation_report::build(result, target, pricing, &measures));
    }

    fn reconfigure(&mut self, r: Reconfiguration) {
        self.pipeline
            .reconfigure(&r.dictionary, &r.snippets, &r.styles, r.post_processor);
        self.pipeline.set_voice_commands(r.voice_commands);
        self.tariffs = r.tariffs;
        self.unload = r.unload;
        self.retry_shortcut = r.retry_shortcut;
        log::info!(
            "configuration appliquée : {} mot(s), {} snippet(s), {} style(s)",
            r.dictionary.len(),
            r.snippets.len(),
            r.styles.len()
        );
        self.model.wanted = r.asr;
        self.model.dictionary = r.dictionary;
        self.model
            .sync(&mut self.pipeline, &self.reporter, &self.recording);
    }
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
        (false, delivery) => {
            let sink: Option<&dyn TextSink> = match delivery {
                Some(Delivery::Clipboard) => Some(&ClipboardCopy),
                _ => None,
            };
            if let Some(language) = &job.translate_to {
                log::info!("mode traduction : {language}");
            }
            pipeline.dictate(&job.clip, target, sink, job.translate_to.as_deref())
        }
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
        costs::record(&recorders.ledger, target.app.as_deref(), usage, pricing);
    }
    if stats {
        let words = raw.split_whitespace().count() as u64;
        costs::record_dictation(
            &recorders.ledger,
            target.app.as_deref(),
            words,
            job.clip.duration_ms(),
            *command,
        );
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
    fn model_changes_with_file_gpu_or_language() {
        let a = AsrConfig::default();
        let unload = AsrConfig {
            unload_after_min: 15,
            ..a.clone()
        };
        assert!(same_model(&a, &unload), "libération : pas de rechargement");
        let gpu = AsrConfig {
            use_gpu: !a.use_gpu,
            ..a.clone()
        };
        assert!(!same_model(&a, &gpu));
        let other = AsrConfig {
            model: "ggml-small-q8_0.bin".into(),
            ..a.clone()
        };
        assert!(!same_model(&a, &other));
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
