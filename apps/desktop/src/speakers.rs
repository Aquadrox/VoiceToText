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
use vtt_core::meeting;

use crate::model;

/// Morceaux en attente d'analyse au-delà desquels un envoi en masse (fichier,
/// audio relu) attend : la mémoire reste faible.
const MAX_QUEUED: usize = 4;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
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
    meeting::save_analysis(dir, analysis, vtt_platform::protect)
}

/// Analyse enregistrée d'une réunion.
pub fn load_analysis(dir: &Path) -> Result<Analysis, String> {
    meeting::load_analysis(dir, vtt_platform::unprotect)
}

pub fn analysis_kept(dir: &Path) -> bool {
    meeting::has_analysis(dir)
}

pub fn delete_analysis(dir: &Path) {
    meeting::delete_analysis(dir);
}

/// Enregistre le carnet de voix chiffré (ou supprime le fichier s'il est vide).
pub fn save_or_delete_voicebook(
    path: &Path,
    book: &vtt_core::voicebook::VoiceBook,
    protect: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if book.is_empty() {
        vtt_core::voicebook::VoiceBook::delete_file(path);
        return Ok(());
    }
    book.save(path, protect)
}

/// Mémorise dans `book` l'empreinte d'un intervenant extrait d'une analyse vocale.
pub fn enroll_speaker_in_book(
    book: &mut vtt_core::voicebook::VoiceBook,
    analysis: &Analysis,
    speaker_count: Option<usize>,
    speaker_index: usize,
    name: &str,
    now_ms: u64,
) -> Result<vtt_core::voicebook::VoiceEntryView, String> {
    let profiles = analysis.speaker_profiles(&Options {
        speakers: speaker_count,
        ..Options::default()
    });
    let profile = profiles
        .into_iter()
        .find(|p| p.speaker == speaker_index)
        .ok_or_else(|| "intervenant introuvable dans l'analyse vocale".to_owned())?;
    let samples = u32::try_from(profile.samples).unwrap_or(u32::MAX);
    let entry = book.enroll(name, &profile.embedding, samples, now_ms)?;
    Ok(vtt_core::voicebook::VoiceEntryView::from(&entry))
}

/// Accès synchronisé au carnet de voix local chiffré (`voicebook.bin`).
pub struct VoiceBookStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl VoiceBookStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load_locked(&self) -> Result<vtt_core::voicebook::VoiceBook, String> {
        vtt_core::voicebook::VoiceBook::load(&self.path, vtt_platform::unprotect)
    }

    fn save_locked(&self, book: &vtt_core::voicebook::VoiceBook) -> Result<(), String> {
        save_or_delete_voicebook(&self.path, book, vtt_platform::protect)
    }

    /// Charge le carnet pour la reconnaissance automatique : en cas d'erreur
    /// de lecture/déchiffrement, journalise et renvoie un carnet vide sans
    /// bloquer la fin de réunion.
    pub fn load_or_empty(&self) -> vtt_core::voicebook::VoiceBook {
        let _guard = lock(&self.lock);
        match self.load_locked() {
            Ok(book) => book,
            Err(e) => {
                log::warn!("carnet de voix illisible : {e}");
                vtt_core::voicebook::VoiceBook::default()
            }
        }
    }

    /// Liste des voix mémorisées (sans vecteurs biométriques).
    pub fn views(&self) -> Result<Vec<vtt_core::voicebook::VoiceEntryView>, String> {
        let _guard = lock(&self.lock);
        Ok(self.load_locked()?.views())
    }

    /// Mémorise (ou enrichit) la voix d'un intervenant depuis l'analyse vocale.
    pub fn enroll_from_analysis(
        &self,
        analysis: &Analysis,
        speaker_count: Option<usize>,
        speaker_index: usize,
        name: &str,
        now_ms: u64,
    ) -> Result<vtt_core::voicebook::VoiceEntryView, String> {
        let _guard = lock(&self.lock);
        let mut book = self.load_locked()?;
        let view = enroll_speaker_in_book(
            &mut book,
            analysis,
            speaker_count,
            speaker_index,
            name,
            now_ms,
        )?;
        self.save_locked(&book)?;
        Ok(view)
    }

    /// Renomme une voix mémorisée dans le carnet.
    pub fn rename(&self, id: &str, name: &str, now_ms: u64) -> Result<(), String> {
        let _guard = lock(&self.lock);
        let mut book = self.load_locked()?;
        book.rename(id, name, now_ms)?;
        self.save_locked(&book)
    }

    /// Supprime une voix mémorisée en un clic.
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let _guard = lock(&self.lock);
        let mut book = self.load_locked()?;
        let removed = book.remove(id);
        if removed {
            self.save_locked(&book)?;
        }
        Ok(removed)
    }

    /// Supprime tout le carnet de voix local.
    pub fn clear(&self) {
        let _guard = lock(&self.lock);
        vtt_core::voicebook::VoiceBook::delete_file(&self.path);
    }
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

    struct FakeModels;

    impl vtt_core::diarize::Models for FakeModels {
        fn segment(
            &mut self,
            window: &[f32],
        ) -> Result<Vec<[f32; vtt_core::diarize::CLASSES]>, vtt_core::Error> {
            use vtt_core::diarize::{CLASSES, FRAME_SIZE, FRAME_STEP, WINDOW};
            let frames = (WINDOW - FRAME_SIZE) / FRAME_STEP + 1;
            let mut local: Vec<i32> = Vec::new();
            Ok((0..frames)
                .map(|f| {
                    let mid = f * FRAME_STEP + FRAME_SIZE / 2;
                    let v = (window[mid] * 10.0).round() as i32;
                    let mut row = [0.0; CLASSES];
                    let class = if v <= 0 {
                        0
                    } else {
                        let slot = local.iter().position(|&x| x == v).unwrap_or_else(|| {
                            local.push(v);
                            local.len() - 1
                        });
                        1 + slot.min(2)
                    };
                    row[class] = 1.0;
                    row
                })
                .collect())
        }

        fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, vtt_core::Error> {
            let v = (samples[samples.len() / 2] * 10.0).round() as i32;
            let mut e = vec![0.05f32; 3];
            if (1..=3).contains(&v) {
                e[(v - 1) as usize] = 1.0;
            }
            Ok(e)
        }
    }

    #[test]
    fn enrolls_speaker_from_analysis_and_cleans_up_empty_voicebook_file() {
        const SECOND: usize = vtt_core::SAMPLE_RATE as usize;
        let mut audio = vec![0.1f32; 8 * SECOND];
        audio.extend(std::iter::repeat_n(0.2f32, 8 * SECOND));
        let mut analysis = Analysis::new(Options::default().step);
        analysis.add(0, &audio, &mut FakeModels).unwrap();
        analysis.finish(&mut FakeModels).unwrap();

        let mut book = vtt_core::voicebook::VoiceBook::default();
        let marie = enroll_speaker_in_book(&mut book, &analysis, None, 0, "Marie", 1_000).unwrap();
        assert_eq!(marie.name, "Marie");
        assert_eq!(marie.meetings, 1);
        assert!(marie.samples > 0);
        // Intervenant inexistant refusé.
        assert!(enroll_speaker_in_book(&mut book, &analysis, None, 5, "Inconnu", 1_001).is_err());

        // Persistance chiffrée puis suppression automatique quand le carnet est vidé.
        let dir = std::env::temp_dir().join(format!("vtt-desktop-vb-{}", std::process::id()));
        let path = dir.join(vtt_core::voicebook::VOICEBOOK_FILE);
        let xor = |data: &[u8]| Ok(data.iter().map(|b| b ^ 0x42).collect());
        save_or_delete_voicebook(&path, &book, xor).unwrap();
        assert!(path.is_file());
        let loaded = vtt_core::voicebook::VoiceBook::load(&path, xor).unwrap();
        assert_eq!(loaded.views(), vec![marie.clone()]);

        book.remove(&marie.id);
        save_or_delete_voicebook(&path, &book, xor).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
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
        let bytes = std::fs::read(wav).unwrap();
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
        let turns = analysis.turns(&Options::default());
        let speakers = turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0);
        eprintln!("{speakers} intervenant(s), {} tours", turns.len());
        assert!(speakers >= 2, "{turns:?}");
        // Modèles libérés une fois l'analyse terminée.
        assert!(lock(&voices.diarizer).is_none());
    }
}
