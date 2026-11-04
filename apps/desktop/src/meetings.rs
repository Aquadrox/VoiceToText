//! Réunions : enregistrement d'un appel (micro « Moi » et son du PC « Eux »),
//! transcription au fil de l'eau par le thread de traitement (entre deux
//! dictées), compte rendu par le LLM, page Réunions.
//!
//! Une réunion par dossier : `meeting.json` (transcription en clair, comme
//! l'historique) et, selon le réglage, l'audio chiffré par Windows.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vtt_core::calls::{self, Decision, Detector, Policy, Provider};
use vtt_core::config::{MEETING_KEEP_DAYS, MeetingAudio, MeetingsConfig, TRANSLATION_LANGUAGES};
use vtt_core::meeting::{self, Chunk, Chunker, Segment, Track};
use vtt_core::usage::Usage;
use vtt_core::{AudioClip, Pipeline, TextSink};
use vtt_platform::ClipboardPaster;
use vtt_platform::capture::{Capture, Tracks};

use crate::controller::{Work, WorkerHandle};
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;
use crate::raw_paste;
use crate::settings::{self, SharedConfig};

const RECORD_FILE: &str = "meeting.json";
/// L'audio capté est découpé et transcrit chaque seconde.
const TICK: Duration = Duration::from_secs(1);
/// Durée enregistrée dans meeting.json pendant l'enregistrement.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Relevé des programmes qui utilisent le micro (détection d'appel).
const DETECT_EVERY: Duration = Duration::from_secs(1);
/// Proposition sans réponse : refermée (jamais d'enregistrement par défaut).
const OFFER_TIMEOUT: Duration = Duration::from_secs(20);
const OFFER_LABEL: &str = "offer";
const OFFER_SIZE: (f64, f64) = (380.0, 132.0);
const OFFER_MARGIN: f64 = 16.0;

/// Une réunion est en cours d'enregistrement (lu par la dictée : pas de
/// pause des médias, qui couperait un appel dans le navigateur).
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    Enregistrement,
    Transcription,
    Terminee,
    /// L'application s'est arrêtée pendant l'enregistrement, sans audio
    /// pour reprendre.
    Interrompue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub text: String,
    pub at_ms: u64,
    pub model: String,
    pub cost: Option<f64>,
    pub currency: String,
}

/// Contenu de `meeting.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub id: String,
    pub title: String,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub status: MeetingStatus,
    pub audio: MeetingAudio,
    /// Le son du PC a été capté (piste « Eux »).
    pub system: bool,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub summary: Option<MeetingSummary>,
    #[serde(default)]
    pub summary_error: Option<String>,
    /// Blocs dont la transcription a échoué.
    #[serde(default)]
    pub failed_chunks: u32,
    /// Remarques (son du PC non capté, audio non chiffrable…).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Écho écarté de la piste « Moi » (appel sur haut-parleurs).
    #[serde(default = "enabled")]
    pub echo_filter: bool,
    /// Blocs du micro écartés comme écho.
    #[serde(default)]
    pub echo_chunks: u32,
}

fn enabled() -> bool {
    true
}

impl MeetingRecord {
    fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(RECORD_FILE);
        let json = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn save(&self, dir: &Path) {
        let result = serde_json::to_string_pretty(self)
            .map_err(|e| e.to_string())
            .and_then(|json| {
                let tmp = dir.join("meeting.json.tmp");
                fs::write(&tmp, json)
                    .and_then(|()| fs::rename(&tmp, dir.join(RECORD_FILE)))
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::error!("enregistrement de la réunion {} : {e}", self.id);
        }
    }

    /// Segments affichés : sans l'écho si le filtre est actif.
    fn segments(&self) -> Vec<Segment> {
        if self.echo_filter {
            meeting::without_echo(&self.segments)
        } else {
            self.segments.clone()
        }
    }

    /// Fin de la transcription déjà faite, par piste (reprise).
    fn transcribed_until(&self, track: Track) -> u64 {
        self.segments
            .iter()
            .filter(|s| s.track == track)
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0)
    }
}

/// Bloc de réunion à transcrire par le thread de traitement.
pub struct MeetingJob {
    session: Arc<Session>,
    track: Track,
    chunk: Chunk,
}

/// File partagée avec le thread de traitement, qui la vide entre deux
/// dictées.
pub type MeetingQueue = Arc<Mutex<VecDeque<MeetingJob>>>;

impl MeetingJob {
    /// Appelé par le thread de traitement.
    pub fn run(self, pipeline: &Pipeline) {
        let clip = AudioClip {
            samples: self.chunk.samples,
        };
        let end_ms = self.chunk.start_ms + clip.duration_ms();
        let result = pipeline.transcribe_only(&clip);
        self.session
            .transcribed(self.track, self.chunk.start_ms, end_ms, result);
    }
}

/// Réunion en cours de transcription (enregistrement, reprise ou nouvelle
/// transcription).
struct Session {
    dir: PathBuf,
    record: Mutex<MeetingRecord>,
    /// Blocs envoyés au thread de traitement, pas encore transcrits.
    pending: AtomicUsize,
    /// L'audio arrive encore (enregistrement en cours).
    capturing: AtomicBool,
    app: AppHandle,
}

impl Session {
    fn id(&self) -> String {
        lock(&self.record).id.clone()
    }

    fn transcribed(
        &self,
        track: Track,
        start_ms: u64,
        end_ms: u64,
        result: Result<Option<String>, vtt_core::Error>,
    ) {
        {
            let mut record = lock(&self.record);
            match result {
                Ok(Some(text)) => {
                    record.segments.push(Segment {
                        track,
                        start_ms,
                        end_ms,
                        text,
                        copied: false,
                    });
                    meeting::sort_segments(&mut record.segments);
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("réunion {} : bloc non transcrit : {e}", record.id);
                    record.failed_chunks += 1;
                }
            }
            record.save(&self.dir);
        }
        let _ = self.app.emit("meetings", ());
        self.pending.fetch_sub(1, Ordering::SeqCst);
        self.finish_if_done();
    }

    /// Plus rien à capter ni à transcrire : réunion terminée.
    fn finish_if_done(&self) {
        if self.capturing.load(Ordering::SeqCst) || self.pending.load(Ordering::SeqCst) > 0 {
            return;
        }
        let (id, audio, title) = {
            let mut record = lock(&self.record);
            if record.status == MeetingStatus::Terminee {
                return;
            }
            record.status = MeetingStatus::Terminee;
            record.save(&self.dir);
            (record.id.clone(), record.audio, record.title.clone())
        };
        if audio == MeetingAudio::Temporaire {
            meeting::delete_audio(&self.dir);
        }
        log::info!("réunion {id} transcrite");
        if let Some(meetings) = self.app.try_state::<MeetingsState>() {
            meetings.finished(&id, &title);
        }
        let _ = self.app.emit("meetings", ());
    }
}

/// Enregistrement en cours.
struct Active {
    session: Arc<Session>,
    stop: mpsc::Sender<()>,
    started: Instant,
}

pub struct Meetings {
    app: AppHandle,
    dir: PathBuf,
    queue: MeetingQueue,
    worker: WorkerHandle,
    tray_item: MenuItem<Wry>,
    active: Mutex<Option<Active>>,
    /// Réunions en cours de transcription, par identifiant.
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Appel détecté, proposé à l'enregistrement (et quand).
    offer: Mutex<Option<(&'static Provider, Instant)>>,
}

pub type MeetingsState = Arc<Meetings>;

/// État affiché par la pastille.
#[derive(Clone, Serialize)]
pub struct MeetingState {
    recording: bool,
    elapsed_ms: u64,
    /// Rappeler de prévenir les participants (au démarrage).
    consent: bool,
}

impl Meetings {
    pub fn new(
        app: AppHandle,
        paths: &Paths,
        queue: MeetingQueue,
        worker: WorkerHandle,
        tray_item: MenuItem<Wry>,
    ) -> MeetingsState {
        Arc::new(Self {
            app,
            dir: paths.meetings_dir.clone(),
            queue,
            worker,
            tray_item,
            active: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            offer: Mutex::new(None),
        })
    }

    fn config(&self) -> vtt_core::Config {
        settings::read(&self.app.state::<SharedConfig>()).clone()
    }

    fn enqueue(&self, session: &Arc<Session>, track: Track, chunk: Chunk) {
        if chunk.is_silent() {
            return;
        }
        session.pending.fetch_add(1, Ordering::SeqCst);
        lock(&self.queue).push_back(MeetingJob {
            session: Arc::clone(session),
            track,
            chunk,
        });
        if let Some(worker) = lock(&self.worker).clone() {
            let _ = worker.send(Work::MeetingWake);
        }
    }

    fn emit_state(&self, consent: bool) {
        let state = self.state();
        let _ = self.app.emit("meeting", MeetingState { consent, ..state });
        let _ = self.tray_item.set_text(if state.recording {
            "Arrêter la réunion"
        } else {
            "Démarrer une réunion"
        });
    }

    pub fn state(&self) -> MeetingState {
        let active = lock(&self.active);
        MeetingState {
            recording: active.is_some(),
            elapsed_ms: active
                .as_ref()
                .map_or(0, |a| a.started.elapsed().as_millis() as u64),
            consent: false,
        }
    }

    /// Démarre l'enregistrement ; renvoie l'identifiant de la réunion.
    /// `call` : service de l'appel détecté, pour le titre.
    pub fn start(self: &Arc<Self>, call: Option<&str>) -> Result<String, String> {
        let mut active = lock(&self.active);
        if active.is_some() {
            return Err("une réunion est déjà en cours d'enregistrement".into());
        }
        let config = self.config();
        let settings = config.meetings.clone();
        let device = Some(config.audio.device.trim()).filter(|d| !d.is_empty());
        let capture = Capture::start(device, settings.system_audio)?;
        let started_at_ms = now_ms();
        let id = started_at_ms.to_string();
        let dir = self.dir.join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut notes = Vec::new();
        if settings.system_audio && capture.outputs.is_empty() {
            notes.push("Son du PC non capté : seul le micro est transcrit.".to_owned());
        }
        let record = MeetingRecord {
            id: id.clone(),
            title: default_title(call),
            started_at_ms,
            duration_ms: 0,
            status: MeetingStatus::Enregistrement,
            audio: settings.audio,
            system: !capture.outputs.is_empty(),
            segments: Vec::new(),
            summary: None,
            summary_error: None,
            failed_chunks: 0,
            notes,
            echo_filter: settings.echo_filter,
            echo_chunks: 0,
        };
        record.save(&dir);
        log::info!(
            "réunion {id} : enregistrement (audio : {:?}, son du PC : {})",
            settings.audio,
            if capture.outputs.is_empty() {
                "non".to_owned()
            } else {
                capture.outputs.join(", ")
            }
        );
        let session = Arc::new(Session {
            dir,
            record: Mutex::new(record),
            pending: AtomicUsize::new(0),
            capturing: AtomicBool::new(true),
            app: self.app.clone(),
        });
        lock(&self.sessions).insert(id.clone(), Arc::clone(&session));
        let (stop_tx, stop_rx) = mpsc::channel();
        let meetings = Arc::clone(self);
        let recorder = Arc::clone(&session);
        thread::Builder::new()
            .name("meeting".into())
            .spawn(move || meetings.record(recorder, capture, stop_rx))
            .map_err(|e| format!("thread de réunion : {e}"))?;
        *active = Some(Active {
            session,
            stop: stop_tx,
            started: Instant::now(),
        });
        drop(active);
        RECORDING.store(true, Ordering::SeqCst);
        self.emit_state(true);
        let _ = self.app.emit("meetings", ());
        Ok(id)
    }

    /// Arrête l'enregistrement ; la transcription des derniers blocs suit.
    pub fn stop(&self) -> Result<(), String> {
        let active = lock(&self.active)
            .take()
            .ok_or("aucune réunion en cours d'enregistrement")?;
        let _ = active.stop.send(());
        RECORDING.store(false, Ordering::SeqCst);
        self.emit_state(false);
        Ok(())
    }

    pub fn toggle(self: &Arc<Self>) {
        let result = if lock(&self.active).is_some() {
            self.stop()
        } else {
            self.start(None).map(|_| ())
        };
        if let Err(e) = result {
            log::warn!("réunion : {e}");
            let _ = self.app.emit("meeting-error", e);
        }
    }

    /// Détection d'appel : chaque seconde, les programmes qui utilisent le
    /// micro ; un appel qui dure → proposition (ou enregistrement, selon le
    /// choix pour ce service).
    pub fn spawn_detection(self: &Arc<Self>) {
        let meetings = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("call-detection".into())
            .spawn(move || {
                let mut detector = Detector::default();
                loop {
                    thread::sleep(DETECT_EVERY);
                    let settings = meetings.config().meetings;
                    if !settings.detect {
                        detector = Detector::default();
                        continue;
                    }
                    let apps = vtt_platform::microphone_apps();
                    let decision =
                        detector.observe(now_ms(), &apps, recording(), |key| settings.policy(key));
                    match decision {
                        Some(Decision::Offer(provider)) => {
                            log::info!("appel détecté : {}", provider.label);
                            meetings.show_offer(provider);
                        }
                        Some(Decision::Record(provider)) => {
                            log::info!(
                                "appel détecté : {} (enregistrement automatique)",
                                provider.label
                            );
                            if let Err(e) = meetings.start(Some(provider.label)) {
                                log::warn!("enregistrement automatique : {e}");
                            }
                        }
                        None => {}
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("détection d'appel : {e}");
        }
    }

    /// Petite fenêtre « Appel détecté : Enregistrer / Pas maintenant », en
    /// bas à droite, sans prendre le focus de l'appel.
    fn show_offer(self: &Arc<Self>, provider: &'static Provider) {
        *lock(&self.offer) = Some((provider, Instant::now()));
        if let Some(window) = self.app.get_webview_window(OFFER_LABEL) {
            let _ = self.app.emit_to(OFFER_LABEL, "offer", ());
            let _ = window.show();
        } else {
            let mut builder = WebviewWindowBuilder::new(
                &self.app,
                OFFER_LABEL,
                WebviewUrl::App("offer.html".into()),
            )
            .title("VoiceToText : appel détecté")
            .inner_size(OFFER_SIZE.0, OFFER_SIZE.1)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false);
            if let Ok(Some(monitor)) = self.app.primary_monitor() {
                let scale = monitor.scale_factor();
                let area = monitor.work_area();
                let right = f64::from(area.position.x) / scale + f64::from(area.size.width) / scale;
                let bottom =
                    f64::from(area.position.y) / scale + f64::from(area.size.height) / scale;
                builder = builder.position(
                    right - OFFER_SIZE.0 - OFFER_MARGIN,
                    bottom - OFFER_SIZE.1 - OFFER_MARGIN,
                );
            }
            if let Err(e) = builder.build() {
                log::warn!("fenêtre de proposition : {e}");
                return;
            }
        }
        // Sans réponse : refermée, rien n'est enregistré.
        let meetings = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(OFFER_TIMEOUT);
            let expired =
                lock(&meetings.offer).is_some_and(|(_, shown)| shown.elapsed() >= OFFER_TIMEOUT);
            if expired {
                meetings.close_offer();
            }
        });
    }

    fn close_offer(&self) {
        *lock(&self.offer) = None;
        if let Some(window) = self.app.get_webview_window(OFFER_LABEL) {
            let _ = window.close();
        }
    }

    /// Réponse à la proposition.
    fn answer_offer(self: &Arc<Self>, choice: &str) -> Result<(), String> {
        let provider = lock(&self.offer).map(|(p, _)| p);
        self.close_offer();
        let Some(provider) = provider else {
            return Ok(());
        };
        let policy = match choice {
            "toujours" => Some(Policy::Toujours),
            "jamais" => Some(Policy::Jamais),
            _ => None,
        };
        if let Some(policy) = policy {
            self.save_policy(provider.key, policy)?;
        }
        if matches!(choice, "enregistrer" | "toujours") {
            self.start(Some(provider.label))?;
        }
        Ok(())
    }

    fn save_policy(&self, key: &str, policy: Policy) -> Result<(), String> {
        let paths = self.app.state::<Paths>();
        let (mut config, error) = paths.load_config();
        if let Some(error) = error {
            return Err(error);
        }
        config.meetings.set_policy(key, policy);
        paths.write_config(&config)?;
        *self
            .app
            .state::<SharedConfig>()
            .write()
            .unwrap_or_else(|e| e.into_inner()) = config;
        log::info!("appels {key} : {policy:?}");
        Ok(())
    }

    /// Thread d'enregistrement : chaque seconde, l'audio capté est écrit
    /// (selon le réglage) et découpé en blocs pour la transcription.
    fn record(&self, session: Arc<Session>, capture: Capture, stop: mpsc::Receiver<()>) {
        let (dir, mut audio, system, echo_filter) = {
            let record = lock(&session.record);
            (
                session.dir.clone(),
                record.audio,
                record.system,
                record.echo_filter,
            )
        };
        let mut echo = meeting::EchoGate::default();
        let mut chunkers = [Chunker::default(), Chunker::default()];
        // `Capture::stop` consomme la capture : gardée jusqu'à l'arrêt.
        let mut capture = Some(capture);
        let started = Instant::now();
        let mut last_save = Instant::now();
        loop {
            let stopped = match stop.recv_timeout(TICK) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
                Err(RecvTimeoutError::Timeout) => false,
            };
            let tracks = if stopped {
                capture.take().map(Capture::stop).unwrap_or_default()
            } else {
                capture.as_ref().map(Capture::take).unwrap_or_default()
            };
            let Tracks { mic, system: other } = tracks;
            // Son du PC d'abord : il sert à reconnaître l'écho dans le micro.
            let mut feeds = Vec::new();
            if system {
                echo.feed_system(&other);
                feeds.push((Track::Eux, other));
            }
            feeds.push((Track::Moi, mic));
            for (track, samples) in feeds {
                if audio != MeetingAudio::Memoire
                    && let Err(e) = meeting::append_audio(
                        &dir.join(track.file()),
                        &samples,
                        vtt_platform::protect,
                    )
                {
                    // Pas de chiffrement : rien sur le disque, jamais en clair.
                    log::warn!("audio de la réunion non écrit : {e}");
                    audio = MeetingAudio::Memoire;
                    meeting::delete_audio(&dir);
                    let mut record = lock(&session.record);
                    record.audio = MeetingAudio::Memoire;
                    record.notes.push(format!(
                        "Audio non enregistré sur le disque ({e}) : gardé en mémoire seulement."
                    ));
                }
                let chunker = &mut chunkers[track as usize];
                let mut chunks = chunker.feed(&samples);
                if stopped {
                    chunks.extend(chunker.finish());
                }
                for chunk in chunks {
                    if track == Track::Moi && system && echo_filter && echo.is_echo(&chunk) {
                        lock(&session.record).echo_chunks += 1;
                        continue;
                    }
                    self.enqueue(&session, track, chunk);
                }
            }
            if stopped || last_save.elapsed() >= SAVE_EVERY {
                let mut record = lock(&session.record);
                record.duration_ms = started.elapsed().as_millis() as u64;
                if stopped {
                    record.status = MeetingStatus::Transcription;
                }
                record.save(&dir);
                last_save = Instant::now();
            }
            if stopped {
                break;
            }
        }
        log::info!("réunion {} : enregistrement arrêté", session.id());
        session.capturing.store(false, Ordering::SeqCst);
        let _ = self.app.emit("meetings", ());
        session.finish_if_done();
    }

    /// Fin de la transcription : compte rendu automatique si demandé.
    fn finished(self: &Arc<Self>, id: &str, title: &str) {
        lock(&self.sessions).remove(id);
        let config = self.config();
        if config.meetings.auto_summary && config.llm.enabled {
            let meetings = Arc::clone(self);
            let id = id.to_owned();
            thread::spawn(move || {
                if let Err(e) = meetings.summarize(&id, None) {
                    log::warn!("compte rendu de la réunion {id} : {e}");
                }
            });
        }
        let reporter = self.app.state::<Arc<crate::status::StatusReporter>>();
        reporter.notify(
            "VoiceToText : réunion transcrite",
            &format!("« {title} » est prête dans la page Réunions."),
        );
    }

    /// Reprend les réunions interrompues par un arrêt de l'application
    /// (audio sur disque : transcription de la suite), et supprime celles qui
    /// ont dépassé la durée de conservation.
    pub fn recover(self: &Arc<Self>) {
        let keep_days = self.config().meetings.keep_days;
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(mut record) = MeetingRecord::load(&dir) else {
                continue;
            };
            let age_days = now_ms().saturating_sub(record.started_at_ms) / 86_400_000;
            if keep_days > 0 && age_days >= u64::from(keep_days) {
                log::info!("réunion {} supprimée ({age_days} jours)", record.id);
                if let Err(e) = fs::remove_dir_all(&dir) {
                    log::warn!("suppression de {} : {e}", dir.display());
                }
                continue;
            }
            if self.busy(&record.id)
                || matches!(
                    record.status,
                    MeetingStatus::Terminee | MeetingStatus::Interrompue
                )
            {
                continue;
            }
            if record.audio == MeetingAudio::Memoire {
                record.status = MeetingStatus::Interrompue;
                record.notes.push(
                    "Application arrêtée pendant la réunion : la fin n'a pas pu être transcrite \
                     (audio gardé en mémoire seulement)."
                        .into(),
                );
                record.save(&dir);
                continue;
            }
            log::info!("réunion {} : reprise de la transcription", record.id);
            if let Err(e) = self.transcribe_audio(record, dir, false) {
                log::warn!("reprise de la réunion : {e}");
            }
        }
    }

    /// Transcrit l'audio enregistré d'une réunion : la suite de ce qui l'est
    /// déjà (reprise), ou tout (`again`).
    fn transcribe_audio(
        self: &Arc<Self>,
        mut record: MeetingRecord,
        dir: PathBuf,
        again: bool,
    ) -> Result<(), String> {
        if again {
            record.segments.clear();
            record.failed_chunks = 0;
        }
        let tracks: Vec<(Track, Vec<f32>, u64)> = [Track::Moi, Track::Eux]
            .into_iter()
            .filter(|&t| t == Track::Moi || record.system)
            .map(|track| {
                let samples =
                    meeting::read_audio(&dir.join(track.file()), vtt_platform::unprotect)?;
                Ok((track, samples, record.transcribed_until(track)))
            })
            .collect::<Result<_, String>>()?;
        let duration = tracks
            .iter()
            .map(|(_, s, _)| s.len() as u64 * 1000 / u64::from(vtt_core::SAMPLE_RATE))
            .max()
            .unwrap_or(record.duration_ms);
        record.duration_ms = record.duration_ms.max(duration);
        record.status = MeetingStatus::Transcription;
        record.save(&dir);
        let id = record.id.clone();
        let session = Arc::new(Session {
            dir,
            record: Mutex::new(record),
            pending: AtomicUsize::new(0),
            capturing: AtomicBool::new(true),
            app: self.app.clone(),
        });
        lock(&self.sessions).insert(id, Arc::clone(&session));
        for (track, samples, from_ms) in tracks {
            let from = (from_ms * u64::from(vtt_core::SAMPLE_RATE) / 1000) as usize;
            let mut chunker = Chunker::starting_at(from_ms);
            let rest = samples.get(from..).unwrap_or_default();
            for chunk in chunker.feed(rest) {
                self.enqueue(&session, track, chunk);
            }
            if let Some(chunk) = chunker.finish() {
                self.enqueue(&session, track, chunk);
            }
        }
        session.capturing.store(false, Ordering::SeqCst);
        let _ = self.app.emit("meetings", ());
        session.finish_if_done();
        Ok(())
    }

    fn meeting_dir(&self, id: &str) -> Result<PathBuf, String> {
        // Identifiant = horodatage : rien d'autre n'est accepté (chemin).
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err("réunion inconnue".into());
        }
        let dir = self.dir.join(id);
        if dir.join(RECORD_FILE).is_file() {
            Ok(dir)
        } else {
            Err("réunion introuvable".into())
        }
    }

    /// Dernier état connu : en mémoire si elle est en cours, sinon sur disque.
    fn load(&self, id: &str) -> Result<(MeetingRecord, PathBuf), String> {
        let dir = self.meeting_dir(id)?;
        if let Some(session) = lock(&self.sessions).get(id) {
            return Ok((lock(&session.record).clone(), dir));
        }
        Ok((MeetingRecord::load(&dir)?, dir))
    }

    fn busy(&self, id: &str) -> bool {
        lock(&self.sessions).contains_key(id)
    }

    /// Modifie une réunion terminée (renommage, compte rendu).
    fn update(&self, id: &str, change: impl FnOnce(&mut MeetingRecord)) -> Result<(), String> {
        if let Some(session) = lock(&self.sessions).get(id) {
            let mut record = lock(&session.record);
            change(&mut record);
            record.save(&session.dir);
        } else {
            let dir = self.meeting_dir(id)?;
            let mut record = MeetingRecord::load(&dir)?;
            change(&mut record);
            record.save(&dir);
        }
        let _ = self.app.emit("meetings", ());
        Ok(())
    }

    /// Compte rendu par le LLM actif ; remplace le précédent. `language` :
    /// langue voulue (`None` : celle des réglages des réunions ; vide : celle
    /// de la réunion).
    pub fn summarize(&self, id: &str, language: Option<String>) -> Result<(), String> {
        let (record, _) = self.load(id)?;
        if record.segments.is_empty() {
            return Err("transcription vide : rien à résumer".into());
        }
        let config = self.config();
        let cleaner = settings::summarizer(&config)
            .ok_or("nettoyage LLM désactivé : activez-le dans les réglages")?;
        let transcript = meeting::transcript_text(&record.segments());
        let language = language
            .unwrap_or_else(|| config.meetings.summary_language.clone())
            .trim()
            .to_owned();
        log::info!(
            "compte rendu de la réunion {id} ({} caractères) via {}",
            transcript.chars().count(),
            config.llm.model
        );
        match cleaner.summarize(
            &record.title,
            &transcript,
            Some(language.as_str()).filter(|l| !l.is_empty()),
        ) {
            Ok(cleaned) => {
                let pricing = &config.llm.pricing;
                let cost = cleaned.usage.map(|u: Usage| pricing.cost(&u));
                if let Some(usage) = &cleaned.usage {
                    costs::record(&self.app.state::<LedgerStore>(), None, usage, pricing);
                }
                let summary = MeetingSummary {
                    text: cleaned.text,
                    at_ms: now_ms(),
                    model: config.llm.model.clone(),
                    cost,
                    currency: pricing.currency.clone(),
                };
                self.update(id, |r| {
                    r.summary = Some(summary);
                    r.summary_error = None;
                })
            }
            Err(e) => {
                let error = e.to_string();
                self.update(id, |r| r.summary_error = Some(error.clone()))?;
                Err(error)
            }
        }
    }

    /// « Copier les nouvelles répliques » : colle dans l'application active
    /// ce qui a été dit depuis la dernière copie (réunion en cours).
    pub fn paste_new(&self) {
        let session = lock(&self.active).as_ref().map(|a| Arc::clone(&a.session));
        let Some(session) = session else {
            let _ = self
                .app
                .emit("meeting-error", "aucune réunion en cours d'enregistrement");
            return;
        };
        let text = {
            let mut record = lock(&session.record);
            let fresh: Vec<Segment> = record
                .segments()
                .into_iter()
                .filter(|s| !s.copied)
                .collect();
            for segment in &mut record.segments {
                segment.copied = true;
            }
            record.save(&session.dir);
            meeting::turns(&fresh)
                .into_iter()
                .map(|(track, _, text)| format!("{} : {text}", track.label()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if text.is_empty() {
            let _ = self
                .app
                .emit("meeting-error", "rien de nouveau depuis la dernière copie");
            return;
        }
        if !raw_paste::wait_modifiers_released() {
            log::warn!("nouvelles répliques : touches du raccourci toujours enfoncées");
            return;
        }
        match ClipboardPaster.insert(&text) {
            Ok(()) => log::info!("nouvelles répliques collées ({} caractères)", text.len()),
            Err(e) => log::warn!("nouvelles répliques : {e}"),
        }
    }
}

/// « Réunion du 4 nov. 2026 à 14:30 », ou « Appel Zoom du … ».
fn default_title(call: Option<&str>) -> String {
    const MONTHS: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let kind = call.map_or_else(|| "Réunion".to_owned(), |label| format!("Appel {label}"));
    format!(
        "{kind} du {} {} {} à {:02}:{:02}",
        now.day(),
        MONTHS[usize::from(u8::from(now.month())) - 1],
        now.year(),
        now.hour(),
        now.minute()
    )
}

/// Enregistre un raccourci global ; vide : désactivé.
fn register(
    app: &AppHandle,
    shortcut: &str,
    label: &str,
    action: impl Fn() + Send + Sync + 'static,
) -> Result<(), String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Ok(());
    }
    let action = Arc::new(action);
    app.global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| {
            if event.state == ShortcutState::Pressed {
                let action = Arc::clone(&action);
                thread::spawn(move || action());
            }
        })
        .map(|()| log::info!("{label} : {shortcut}"))
        .map_err(|e| {
            log::warn!("raccourci « {shortcut} » indisponible : {e}");
            format!("raccourci « {shortcut} » indisponible (déjà pris par une autre application ?)")
        })
}

pub fn register_toggle(
    app: &AppHandle,
    shortcut: &str,
    meetings: MeetingsState,
) -> Result<(), String> {
    register(app, shortcut, "réunion", move || meetings.toggle())
}

pub fn register_paste(
    app: &AppHandle,
    shortcut: &str,
    meetings: MeetingsState,
) -> Result<(), String> {
    register(app, shortcut, "nouvelles répliques", move || {
        meetings.paste_new()
    })
}

// ---------- Commandes de la fenêtre principale ----------

#[derive(Serialize)]
pub struct MeetingRow {
    id: String,
    title: String,
    started_at_ms: u64,
    duration_ms: u64,
    status: MeetingStatus,
    segments: usize,
    has_summary: bool,
    audio_kept: bool,
}

#[derive(Serialize)]
pub struct MeetingsView {
    recording: Option<String>,
    elapsed_ms: u64,
    meetings: Vec<MeetingRow>,
    settings: MeetingsConfig,
    keep_days: &'static [u32],
    llm_enabled: bool,
    /// Services d'appel reconnus et le choix pour chacun.
    providers: Vec<ProviderView>,
    languages: &'static [&'static str],
}

#[derive(Serialize)]
pub struct ProviderView {
    key: &'static str,
    label: &'static str,
    browser: bool,
    policy: Policy,
}

#[tauri::command]
pub fn meetings_view(
    meetings: State<'_, MeetingsState>,
    config: State<'_, SharedConfig>,
) -> MeetingsView {
    let mut rows: Vec<MeetingRow> = fs::read_dir(&meetings.dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            let (record, dir) = meetings.load(&id).ok()?;
            Some(MeetingRow {
                audio_kept: record.audio == MeetingAudio::Conserve
                    && dir.join(Track::Moi.file()).is_file(),
                id: record.id,
                title: record.title,
                started_at_ms: record.started_at_ms,
                duration_ms: record.duration_ms,
                status: record.status,
                segments: record.segments.len(),
                has_summary: record.summary.is_some(),
            })
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
    let state = meetings.state();
    let active = lock(&meetings.active).as_ref().map(|a| a.session.id());
    let config = settings::read(&config);
    MeetingsView {
        recording: active,
        elapsed_ms: state.elapsed_ms,
        meetings: rows,
        settings: config.meetings.clone(),
        keep_days: MEETING_KEEP_DAYS,
        llm_enabled: config.llm.enabled,
        providers: calls::providers()
            .map(|p| ProviderView {
                key: p.key,
                label: p.label,
                browser: p.browser,
                policy: config.meetings.policy(p.key),
            })
            .collect(),
        languages: TRANSLATION_LANGUAGES,
    }
}

#[derive(Serialize)]
pub struct Turn {
    track: Track,
    start_ms: u64,
    text: String,
}

#[derive(Serialize)]
pub struct MeetingDetail {
    record: MeetingRecord,
    turns: Vec<Turn>,
    busy: bool,
    audio_kept: bool,
}

#[tauri::command]
pub fn meeting_detail(
    meetings: State<'_, MeetingsState>,
    id: String,
) -> Result<MeetingDetail, String> {
    let (record, dir) = meetings.load(&id)?;
    let turns = meeting::turns(&record.segments())
        .into_iter()
        .map(|(track, start_ms, text)| Turn {
            track,
            start_ms,
            text,
        })
        .collect();
    Ok(MeetingDetail {
        audio_kept: dir.join(Track::Moi.file()).is_file() && record.audio == MeetingAudio::Conserve,
        busy: meetings.busy(&id),
        record,
        turns,
    })
}

#[derive(Serialize)]
pub struct OfferView {
    label: &'static str,
    browser: bool,
}

#[tauri::command]
pub fn offer_view(meetings: State<'_, MeetingsState>) -> Option<OfferView> {
    lock(&meetings.offer).map(|(p, _)| OfferView {
        label: p.label,
        browser: p.browser,
    })
}

/// `choice` : « enregistrer », « plus-tard », « toujours », « jamais ».
#[tauri::command]
pub fn offer_answer(meetings: State<'_, MeetingsState>, choice: String) -> Result<(), String> {
    meetings.answer_offer(&choice)
}

#[tauri::command]
pub fn meeting_state(meetings: State<'_, MeetingsState>) -> MeetingState {
    meetings.state()
}

#[tauri::command]
pub fn start_meeting(meetings: State<'_, MeetingsState>) -> Result<String, String> {
    meetings.start(None)
}

#[tauri::command]
pub fn stop_meeting(meetings: State<'_, MeetingsState>) -> Result<(), String> {
    meetings.stop()
}

#[tauri::command]
pub fn rename_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    title: String,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("titre vide ou trop long (200 caractères au plus)".into());
    }
    meetings.update(&id, |r| r.title = title.to_owned())
}

#[tauri::command(async)]
pub fn summarize_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    language: Option<String>,
) -> Result<(), String> {
    meetings.summarize(&id, language)
}

#[tauri::command]
pub fn delete_meeting(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    if meetings.busy(&id) {
        return Err("réunion en cours de transcription : attendez la fin".into());
    }
    let dir = meetings.meeting_dir(&id)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("suppression : {e}"))?;
    log::info!("réunion {id} supprimée");
    let _ = meetings.app.emit("meetings", ());
    Ok(())
}

/// Nouvelle transcription de tout l'audio conservé (après un changement de
/// modèle, par exemple).
#[tauri::command]
pub fn retranscribe_meeting(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    if meetings.busy(&id) {
        return Err("réunion déjà en cours de transcription".into());
    }
    let (record, dir) = meetings.load(&id)?;
    if !dir.join(Track::Moi.file()).is_file() {
        return Err("audio non conservé pour cette réunion".into());
    }
    meetings.inner().transcribe_audio(record, dir, true)
}

#[tauri::command]
pub fn copy_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    part: String,
) -> Result<(), String> {
    let (record, _) = meetings.load(&id)?;
    let text = match part.as_str() {
        "summary" => record.summary.map(|s| s.text).unwrap_or_default(),
        "transcript" => meeting::transcript_text(&record.segments()),
        _ => markdown(&record),
    };
    vtt_platform::copy_text(&text)
}

fn markdown(record: &MeetingRecord) -> String {
    let date = time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(record.started_at_ms) * 1_000_000,
    )
    .ok()
    .and_then(|d| {
        d.format(time::macros::format_description!(
            "[day]/[month]/[year] [hour]:[minute]"
        ))
        .ok()
    })
    .unwrap_or_default();
    meeting::to_markdown(
        &record.title,
        &date,
        record.duration_ms,
        record.summary.as_ref().map(|s| s.text.as_str()),
        &record.segments(),
    )
}

/// Nom de fichier sûr d'après le titre.
fn file_name(title: &str, extension: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("{}.{extension}", safe.trim().trim_matches('.'))
}

#[tauri::command(async)]
pub fn export_meeting(
    app: AppHandle,
    meetings: State<'_, MeetingsState>,
    id: String,
) -> Result<Option<String>, String> {
    let (record, _) = meetings.load(&id)?;
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Exporter la réunion")
        .add_filter("Markdown", &["md"])
        .set_file_name(file_name(&record.title, "md"))
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, markdown(&record)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// Exporte l'audio conservé d'une piste en WAV (déchiffré).
#[tauri::command(async)]
pub fn export_meeting_audio(
    app: AppHandle,
    meetings: State<'_, MeetingsState>,
    id: String,
    track: Track,
) -> Result<Option<String>, String> {
    let (record, dir) = meetings.load(&id)?;
    let samples = meeting::read_audio(&dir.join(track.file()), vtt_platform::unprotect)?;
    if samples.is_empty() {
        return Err("aucun audio conservé pour cette piste".into());
    }
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Exporter l'audio")
        .add_filter("WAV", &["wav"])
        .set_file_name(file_name(
            &format!("{} - {}", record.title, track.label()),
            "wav",
        ))
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, meeting::wav_bytes(&samples))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub fn save_meeting_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    meetings: State<'_, MeetingsState>,
    settings: MeetingsConfig,
) -> Result<(), String> {
    let (mut config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    settings.validate()?;
    config.meetings = MeetingsConfig {
        // Le prompt se choisit dans la page Prompts.
        summary_prompt: config.meetings.summary_prompt.clone(),
        summary_language: settings.summary_language.trim().to_owned(),
        ..settings
    };
    paths.write_config(&config)?;
    crate::app_settings::apply(&app, &paths, &config);
    // Durée de conservation raccourcie : appliquée tout de suite.
    meetings.recover();
    log::info!("réglages des réunions appliqués");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> MeetingRecord {
        MeetingRecord {
            id: "1793790000000".into(),
            title: "Point projet".into(),
            started_at_ms: 1_793_790_000_000,
            duration_ms: 60_000,
            status: MeetingStatus::Transcription,
            audio: MeetingAudio::Temporaire,
            system: true,
            segments: vec![
                Segment {
                    track: Track::Moi,
                    start_ms: 0,
                    end_ms: 7_000,
                    text: "Bonjour.".into(),
                    copied: false,
                },
                Segment {
                    track: Track::Eux,
                    start_ms: 2_000,
                    end_ms: 9_500,
                    text: "Salut.".into(),
                    copied: true,
                },
            ],
            summary: None,
            summary_error: None,
            failed_chunks: 0,
            notes: Vec::new(),
            echo_filter: true,
            echo_chunks: 0,
        }
    }

    #[test]
    fn record_round_trips_and_resumes_after_each_track() {
        let dir = std::env::temp_dir().join(format!("vtt-meetings-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        record().save(&dir);
        let loaded = MeetingRecord::load(&dir).unwrap();
        assert_eq!(loaded.segments, record().segments);
        assert_eq!(loaded.status, MeetingStatus::Transcription);
        assert_eq!(loaded.transcribed_until(Track::Moi), 7_000);
        assert_eq!(loaded.transcribed_until(Track::Eux), 9_500);
        assert!(markdown(&loaded).contains("**Moi** [00:00] : Bonjour."));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_names_drop_forbidden_characters() {
        assert_eq!(
            file_name("Point : budget / Q4?", "md"),
            "Point - budget - Q4-.md"
        );
        assert_eq!(file_name(" Réunion. ", "wav"), "Réunion.wav");
    }
}
