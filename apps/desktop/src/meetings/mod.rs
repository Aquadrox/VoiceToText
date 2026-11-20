//! Réunions : enregistrement d'un appel (micro « Moi » et son du PC « Eux »),
//! transcription au fil de l'eau par le thread de traitement (entre deux
//! dictées), compte rendu par le LLM, page Réunions.
//!
//! Une réunion par dossier : `meeting.json` (transcription en clair, comme
//! l'historique) et, selon le réglage, l'audio chiffré par Windows.

mod capture;
mod commands;
mod export;
mod storage;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vtt_core::calls::Provider;
use vtt_core::meeting::{Chunk, MeetingKind, MeetingRecord, Track};
use vtt_core::{AudioClip, Pipeline};

use crate::controller::{Work, WorkerHandle};
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};
use crate::speakers::{VoiceBookStore, VoiceFeed, VoicesState};

pub(crate) use commands::*;

/// Une réunion est en cours d'enregistrement (lu par la dictée : pas de
/// pause des médias, qui couperait un appel dans le navigateur).
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

pub(super) fn set_recording(active: bool) {
    RECORDING.store(active, Ordering::SeqCst);
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
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
pub(super) struct Session {
    pub(super) dir: PathBuf,
    pub(super) record: Mutex<MeetingRecord>,
    /// Blocs envoyés au thread de traitement, pas encore transcrits.
    pub(super) pending: AtomicUsize,
    /// L'audio arrive encore (enregistrement en cours).
    pub(super) capturing: AtomicBool,
    pub(super) app: AppHandle,
    /// Analyse des voix de la piste `voice_track` (intervenants).
    pub(super) voices: Mutex<Option<VoiceFeed>>,
    pub(super) voice_track: Option<Track>,
    /// Fin de réunion déjà engagée.
    pub(super) finishing: AtomicBool,
}

/// Enregistrement en cours.
pub(super) struct Active {
    pub(super) session: Arc<Session>,
    pub(super) stop: mpsc::Sender<()>,
    pub(super) started: Instant,
}

pub struct Meetings {
    pub(super) app: AppHandle,
    pub(super) dir: PathBuf,
    pub(super) voicebook: VoiceBookStore,
    pub(super) queue: MeetingQueue,
    pub(super) worker: WorkerHandle,
    pub(super) tray_item: MenuItem<Wry>,
    pub(super) active: Mutex<Option<Active>>,
    /// Réunions en cours de transcription, par identifiant.
    pub(super) sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Appel détecté, proposé à l'enregistrement (et quand).
    pub(super) offer: Mutex<Option<(&'static Provider, Instant)>>,
}

pub type MeetingsState = Arc<Meetings>;

/// État affiché par la pastille.
#[derive(Clone, Serialize)]
pub struct MeetingState {
    pub(super) recording: bool,
    pub(super) elapsed_ms: u64,
    /// Rappeler de prévenir les participants (au démarrage).
    pub(super) consent: bool,
    /// Conférence plutôt que réunion (libellé de la pastille).
    pub(super) conference: bool,
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
            voicebook: VoiceBookStore::new(paths.voicebook_file.clone()),
            queue,
            worker,
            tray_item,
            active: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            offer: Mutex::new(None),
        })
    }

    pub(super) fn config(&self) -> vtt_core::Config {
        settings::read(&self.app.state::<SharedConfig>()).clone()
    }

    pub(super) fn enqueue(&self, session: &Arc<Session>, track: Track, chunk: Chunk) {
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

    /// Session de transcription ; l'analyse des voix s'y ajoute si le
    /// réglage est actif (modèles téléchargés pour la prochaine fois s'ils
    /// manquent).
    pub(super) fn new_session(&self, dir: PathBuf, mut record: MeetingRecord) -> Arc<Session> {
        let voice_track = record.voice_track();
        let voices = if self.config().meetings.diarize && voice_track.is_some() {
            let state = self.app.state::<VoicesState>();
            let feed = state.feed();
            if feed.is_none() {
                record.notes.push(
                    "Intervenants non distingués : modèles pas encore téléchargés \
                     (Réglages → Réunions)."
                        .into(),
                );
                state.download();
            }
            feed
        } else {
            None
        };
        Arc::new(Session {
            dir,
            record: Mutex::new(record),
            pending: AtomicUsize::new(0),
            capturing: AtomicBool::new(true),
            app: self.app.clone(),
            voices: Mutex::new(voices),
            voice_track,
            finishing: AtomicBool::new(false),
        })
    }

    pub(super) fn emit_state(&self, consent: bool) {
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
            conference: active
                .as_ref()
                .is_some_and(|a| lock(&a.session.record).kind == MeetingKind::Conference),
        }
    }
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
