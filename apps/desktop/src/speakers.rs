//! Intervenants des réunions (diarisation : qui parle quand).
//!
//! - Modèles (33 Mo, plus ONNX Runtime si Parakeet ne l'a pas déjà
//!   apporté) téléchargés à l'activation du réglage, vérifiés.
//! - Pendant l'enregistrement (ou la lecture d'un fichier), les blocs de la
//!   piste analysée partent à un thread à part : l'audio n'y est pas gardé,
//!   seules restent les voix entendues et leurs empreintes
//!   ([`vtt_core::diarize::Analysis`]).
//! - À la fin, les tours de parole sont calculés ; l'analyse est enregistrée
//!   chiffrée (`voix.bin`), pour changer le nombre d'intervenants sans
//!   l'audio.
//!
//! Méthode : pyannote.audio (Hervé Bredin, CNRS) ; idée reprise de
//! pytranscribe (Xavier Beheydt) ; voir `vtt_core::diarize`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use vtt_asr::Diarizer;
use vtt_core::diarize::{Analysis, Options};

use crate::model;

/// Fichier de l'analyse des voix d'une réunion (chiffré).
const ANALYSIS_FILE: &str = "voix.bin";
/// Morceaux en attente d'analyse au-delà desquels un envoi en masse (fichier,
/// audio relu) attend : la mémoire reste faible.
const MAX_QUEUED: usize = 4;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Réglages de la diarisation pour un nombre d'intervenants imposé ou non.
pub fn options(speakers: Option<usize>) -> Options {
    Options {
        speakers,
        ..Options::default()
    }
}

/// État du téléchargement des modèles.
#[derive(Debug, Clone, Default, Serialize)]
struct Download {
    running: bool,
    percent: u8,
    error: Option<String>,
}

/// Pour la section Réunions des réglages.
#[derive(Debug, Clone, Serialize)]
pub struct VoicesView {
    installed: bool,
    /// Reste à télécharger (Mo).
    missing_mb: u64,
    downloading: bool,
    percent: u8,
    error: Option<String>,
}

/// Reçoit l'état du téléchargement (événement pour la fenêtre).
type Events = Box<dyn Fn(VoicesView) + Send + Sync>;

pub struct Voices {
    events: Events,
    models_dir: PathBuf,
    /// Modèles chargés, partagés par les analyses en cours ; libérés quand
    /// plus aucune ne tourne.
    diarizer: Mutex<Option<Diarizer>>,
    running: AtomicUsize,
    download: Mutex<Download>,
}

pub type VoicesState = Arc<Voices>;

impl Voices {
    pub fn new(app: AppHandle, models_dir: PathBuf) -> VoicesState {
        Self::with_events(
            models_dir,
            Box::new(move |view| {
                let _ = app.emit("voices", view);
            }),
        )
    }

    fn with_events(models_dir: PathBuf, events: Events) -> VoicesState {
        Arc::new(Self {
            events,
            models_dir,
            diarizer: Mutex::new(None),
            running: AtomicUsize::new(0),
            download: Mutex::new(Download::default()),
        })
    }

    pub fn installed(&self) -> bool {
        model::diarization_local(&self.models_dir).is_some()
    }

    pub fn view(&self) -> VoicesView {
        let download = lock(&self.download).clone();
        VoicesView {
            installed: self.installed(),
            missing_mb: model::diarization_missing_mb(&self.models_dir),
            downloading: download.running,
            percent: download.percent,
            error: download.error,
        }
    }

    fn emit(&self) {
        (self.events)(self.view());
    }

    /// Télécharge les modèles en arrière-plan (un seul téléchargement à la
    /// fois ; rien à faire s'ils sont là).
    pub fn download(self: &Arc<Self>) {
        if self.installed() {
            return;
        }
        {
            let mut download = lock(&self.download);
            if download.running {
                return;
            }
            *download = Download {
                running: true,
                ..Download::default()
            };
        }
        self.emit();
        let voices = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("voices-download".into())
            .spawn(move || {
                let result = model::ensure_diarization(&voices.models_dir, |percent| {
                    lock(&voices.download).percent = percent;
                    voices.emit();
                });
                {
                    let mut download = lock(&voices.download);
                    download.running = false;
                    match result {
                        Ok(_) => log::info!("modèles de la diarisation installés"),
                        Err(e) => {
                            log::warn!("modèles de la diarisation : {e}");
                            download.error = Some(e);
                        }
                    }
                }
                voices.emit();
            });
        if let Err(e) = spawned {
            log::warn!("téléchargement des modèles de la diarisation : {e}");
            lock(&self.download).running = false;
        }
    }

    /// Analyse des voix d'un enregistrement, dans un thread à part ; `None`
    /// si les modèles manquent.
    pub fn feed(self: &Arc<Self>) -> Option<VoiceFeed> {
        if !self.installed() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let voices = Arc::clone(self);
        let counter = Arc::clone(&queued);
        self.running.fetch_add(1, Ordering::SeqCst);
        let handle = thread::Builder::new().name("voices".into()).spawn(move || {
            let result = voices.analyze(&rx, &counter);
            if voices.running.fetch_sub(1, Ordering::SeqCst) == 1 {
                // Plus aucune analyse : mémoire rendue.
                *lock(&voices.diarizer) = None;
            }
            result
        });
        match handle {
            Ok(handle) => Some(VoiceFeed {
                tx: Some(tx),
                queued,
                failed: Arc::new(AtomicBool::new(false)),
                handle: Some(handle),
            }),
            Err(e) => {
                self.running.fetch_sub(1, Ordering::SeqCst);
                log::warn!("analyse des voix : {e}");
                None
            }
        }
    }

    /// Thread d'analyse : morceaux reçus jusqu'à la fermeture du canal.
    fn analyze(
        &self,
        rx: &Receiver<(u64, Vec<f32>)>,
        queued: &AtomicUsize,
    ) -> Result<Analysis, String> {
        let started = Instant::now();
        let mut analysis = Analysis::new(Options::default().step);
        let mut error = None;
        for (start_ms, samples) in rx {
            if error.is_none()
                && let Err(e) = self.with_diarizer(|d| {
                    analysis
                        .add(start_ms, &samples, d)
                        .map_err(|e| e.to_string())
                })
            {
                log::warn!("analyse des voix : {e}");
                error = Some(e);
            }
            queued.fetch_sub(1, Ordering::SeqCst);
        }
        if let Some(e) = error {
            return Err(e);
        }
        self.with_diarizer(|d| analysis.finish(d).map_err(|e| e.to_string()))?;
        log::info!(
            "voix analysées : {} s d'audio en {} ms",
            analysis.duration_ms() / 1000,
            started.elapsed().as_millis()
        );
        Ok(analysis)
    }

    /// Modèles chargés au premier besoin.
    fn with_diarizer<R>(
        &self,
        f: impl FnOnce(&mut Diarizer) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut slot = lock(&self.diarizer);
        if slot.is_none() {
            let models = model::diarization_local(&self.models_dir)
                .ok_or("modèles de la diarisation absents")?;
            // La moitié des cœurs : la transcription continue pendant l'appel.
            let threads = thread::available_parallelism()
                .map(|n| (n.get() / 2).clamp(1, 4))
                .unwrap_or(2);
            let diarizer = Diarizer::load(
                &models.segmentation,
                &models.embedding,
                &models.runtime,
                threads,
            )
            .map_err(|e| e.to_string())?;
            *slot = Some(diarizer);
        }
        f(slot.as_mut().expect("modèles chargés ci-dessus"))
    }
}

/// Envoi de l'audio d'un enregistrement à l'analyse des voix.
pub struct VoiceFeed {
    tx: Option<Sender<(u64, Vec<f32>)>>,
    queued: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<Analysis, String>>>,
}

impl VoiceFeed {
    /// `samples` commencent à `start_ms` depuis le début de l'enregistrement.
    pub fn send(&self, start_ms: u64, samples: Vec<f32>) {
        let Some(tx) = &self.tx else {
            return;
        };
        self.queued.fetch_add(1, Ordering::SeqCst);
        if tx.send((start_ms, samples)).is_err() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    /// Attend que l'analyse ait rattrapé son retard (envoi en masse).
    pub fn wait_backlog(&self) {
        while self.queued.load(Ordering::SeqCst) > MAX_QUEUED && !self.failed.load(Ordering::SeqCst)
        {
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Autre point d'envoi (thread qui relit l'audio) ; l'analyse se termine
    /// quand tous sont fermés.
    pub fn sender(&self) -> Option<FeedSender> {
        Some(FeedSender {
            tx: self.tx.clone()?,
            queued: Arc::clone(&self.queued),
            buffer: Vec::new(),
            sent: 0,
        })
    }

    /// Plus rien à envoyer : attend la fin de l'analyse.
    pub fn finish(mut self) -> Result<Analysis, String> {
        drop(self.tx.take());
        self.handle
            .take()
            .ok_or("analyse des voix déjà terminée")?
            .join()
            .map_err(|_| "analyse des voix interrompue".to_owned())?
    }
}

/// Envoi depuis un autre thread : audio relu (piste sur le disque, fichier
/// importé), contigu depuis le début, regroupé en morceaux de 30 s.
pub struct FeedSender {
    tx: Sender<(u64, Vec<f32>)>,
    queued: Arc<AtomicUsize>,
    buffer: Vec<f32>,
    /// Échantillons déjà envoyés.
    sent: usize,
}

impl FeedSender {
    const PIECE: usize = vtt_core::SAMPLE_RATE as usize * 30;

    /// Ajoute de l'audio ; `false` si l'analyse s'est arrêtée.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= Self::PIECE {
            let rest = self.buffer.split_off(Self::PIECE);
            let piece = std::mem::replace(&mut self.buffer, rest);
            if !self.send(piece) {
                return false;
            }
        }
        true
    }

    /// Envoie ce qui reste.
    pub fn flush(&mut self) {
        let piece = std::mem::take(&mut self.buffer);
        if !piece.is_empty() {
            self.send(piece);
        }
    }

    fn send(&mut self, piece: Vec<f32>) -> bool {
        // Sans prendre d'avance : la mémoire reste faible.
        while self.queued.load(Ordering::SeqCst) > MAX_QUEUED {
            thread::sleep(Duration::from_millis(100));
        }
        let start_ms = (self.sent as u64) * 1000 / u64::from(vtt_core::SAMPLE_RATE);
        self.sent += piece.len();
        self.queued.fetch_add(1, Ordering::SeqCst);
        if self.tx.send((start_ms, piece)).is_err() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        true
    }
}

/// Enregistre l'analyse, chiffrée (une empreinte vocale identifie une
/// personne : jamais en clair sur le disque).
pub fn save_analysis(dir: &Path, analysis: &Analysis) -> Result<(), String> {
    let data = vtt_platform::protect(&analysis.to_bytes())?;
    let tmp = dir.join("voix.bin.tmp");
    fs::write(&tmp, data)
        .and_then(|()| fs::rename(&tmp, dir.join(ANALYSIS_FILE)))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Analyse enregistrée d'une réunion.
pub fn load_analysis(dir: &Path) -> Result<Analysis, String> {
    let path = dir.join(ANALYSIS_FILE);
    let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Analysis::from_bytes(&vtt_platform::unprotect(&data)?)
}

pub fn analysis_kept(dir: &Path) -> bool {
    dir.join(ANALYSIS_FILE).is_file()
}

pub fn delete_analysis(dir: &Path) {
    let _ = fs::remove_file(dir.join(ANALYSIS_FILE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_analyzed_without_the_models() {
        let voices = Voices::with_events(PathBuf::from("/nulle-part"), Box::new(|_| {}));
        assert!(!voices.installed());
        assert!(voices.feed().is_none());
        assert!(voices.view().missing_mb > 0);
    }

    /// Analyse réelle, comme pendant une réunion : blocs de 6 à 15 s envoyés
    /// au thread, puis tours de parole. `VTT_MODELS` : dossier des modèles
    /// (avec `diarisation-1` et ONNX Runtime) ; `VTT_TEST_WAV` : WAV 16 kHz
    /// mono 16 bits à plusieurs voix.
    /// `cargo test -p voicetotext -- --ignored meeting_like_analysis`
    #[test]
    #[ignore]
    fn meeting_like_analysis() {
        let models = PathBuf::from(std::env::var("VTT_MODELS").unwrap());
        let wav = std::env::var("VTT_TEST_WAV").unwrap();
        let bytes = fs::read(wav).unwrap();
        let samples: Vec<f32> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        let voices = Voices::with_events(models, Box::new(|_| {}));
        let feed = voices.feed().expect("modèles installés");
        let mut at = 0;
        for seconds in [6usize, 15, 9, 12, 7].iter().cycle() {
            if at >= samples.len() {
                break;
            }
            let end = (at + seconds * 16_000).min(samples.len());
            feed.send(at as u64 / 16, samples[at..end].to_vec());
            at = end;
        }
        let analysis = feed.finish().unwrap();
        let turns = analysis.turns(&options(None));
        let speakers = turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0);
        eprintln!("{speakers} intervenant(s), {} tours", turns.len());
        assert!(speakers >= 2, "{turns:?}");
        // Modèles libérés une fois l'analyse terminée.
        assert!(lock(&voices.diarizer).is_none());
    }
}
