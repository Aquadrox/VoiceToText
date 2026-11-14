//! Réunions sur le téléphone : enregistrement au micro ou import d'un
//! fichier audio, puis, à la fin, transcription par blocs avec le moteur de
//! la dictée et compte rendu par le LLM.
//!
//! Une réunion par dossier (`meetings/<id>`), au même format que sur le PC
//! (`meeting.json`, [`MeetingRecord`]). L'audio, en PCM 16 bits 16 kHz mono,
//! va selon le choix de l'utilisateur dans le stockage privé de
//! l'application (chiffré par Android, inaccessible aux autres applications)
//! ou en mémoire seulement.
//!
//! L'application Kotlin capte ou décode l'audio et le transmet tel quel
//! ([`feed`]) : la conversion en 16 kHz mono est faite ici.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vtt_core::config::MeetingAudio;
use vtt_core::meeting::{
    self, Chunk, Chunker, MeetingKind, MeetingRecord, MeetingStatus, MeetingSummary, Output,
    RECORD_FILE, Segment, Track,
};
use vtt_core::{SAMPLE_RATE, audio};

use vtt_core::diarize;

use crate::{engine, models, settings};

/// Un jour, en millisecondes (durée de conservation).
const DAY_MS: u64 = 24 * 3600 * 1000;
/// Durée d'un enregistrement en cours écrite dans `meeting.json`.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Avancement de la lecture d'un fichier importé : 0 à 100 %.
const MAX_PERCENT: u8 = 100;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn samples_to_ms(samples: u64) -> u64 {
    samples * 1000 / u64::from(SAMPLE_RATE)
}

fn ms_to_samples(ms: u64) -> usize {
    (ms * u64::from(SAMPLE_RATE) / 1000) as usize
}

/// Audio sur le stockage privé : déjà chiffré par Android (chiffrement par
/// fichier, lié au déverrouillage du téléphone). Les blocs sont écrits tels
/// quels, au format du PC.
fn plain(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}

/// Piste où va l'audio : le micro pour un enregistrement, « l'autre côté »
/// pour un fichier importé (comme sur le PC).
fn track(record: &MeetingRecord) -> Track {
    if record.kind == MeetingKind::Fichier {
        Track::Eux
    } else {
        Track::Moi
    }
}

/// Conversion au fil de l'eau vers 16 kHz : l'état (reste d'entrée,
/// position) est gardé d'un envoi à l'autre, sans décalage cumulé.
#[derive(Debug)]
struct Resampler {
    from: u32,
    /// Entrée pas encore consommée.
    carry: Vec<f32>,
    /// Position (dans `carry`) du prochain échantillon de sortie.
    position: f64,
}

impl Resampler {
    fn new(from: u32) -> Self {
        Self {
            from,
            carry: Vec::new(),
            position: 0.0,
        }
    }

    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if self.from == SAMPLE_RATE {
            return input.to_vec();
        }
        self.carry.extend_from_slice(input);
        let step = f64::from(self.from) / f64::from(SAMPLE_RATE);
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 1);
        if step > 1.0 {
            // Moyenne de la fenêtre couverte (filtre anti-repliement simple).
            while self.position + step <= self.carry.len() as f64 {
                let start = self.position as usize;
                let end = ((self.position + step) as usize).max(start + 1);
                let window = &self.carry[start..end];
                out.push(window.iter().sum::<f32>() / window.len() as f32);
                self.position += step;
            }
        } else {
            // Interpolation linéaire (enregistrement d'appel à 8 kHz…).
            while self.position + 1.0 < self.carry.len() as f64 {
                let index = self.position as usize;
                let fraction = (self.position - index as f64) as f32;
                let (a, b) = (self.carry[index], self.carry[index + 1]);
                out.push(a + (b - a) * fraction);
                self.position += step;
            }
        }
        let consumed = (self.position as usize).min(self.carry.len());
        self.carry.drain(..consumed);
        self.position -= consumed as f64;
        out
    }
}

/// Audio d'une réunion en cours de réception (enregistrement ou import).
struct Live {
    /// Échantillons reçus (16 kHz).
    samples: u64,
    /// Où va l'audio (choix de l'utilisateur) et fichier de la piste.
    audio: MeetingAudio,
    file: PathBuf,
    resampler: Option<Resampler>,
    saved_at: Instant,
}

/// Réunions qui reçoivent de l'audio, par identifiant.
static LIVE: Mutex<Option<HashMap<String, Live>>> = Mutex::new(None);
/// Audio « en mémoire seulement », par identifiant (16 bits : deux fois
/// moins de place que des flottants, 115 Mo par heure).
static MEMORY: Mutex<Option<HashMap<String, Vec<i16>>>> = Mutex::new(None);
/// Ce que le moteur fait pour une réunion : « transcription » ou
/// « summary » (compte rendu).
static STEPS: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
/// Une seule modification de `meeting.json` à la fois (transcription et
/// changement de titre, par exemple).
static RECORDS: Mutex<()> = Mutex::new(());

fn live<T>(f: impl FnOnce(&mut HashMap<String, Live>) -> T) -> T {
    f(lock(&LIVE).get_or_insert_with(HashMap::new))
}

fn memory<T>(f: impl FnOnce(&mut HashMap<String, Vec<i16>>) -> T) -> T {
    f(lock(&MEMORY).get_or_insert_with(HashMap::new))
}

fn steps<T>(f: impl FnOnce(&mut HashMap<String, &'static str>) -> T) -> T {
    f(lock(&STEPS).get_or_insert_with(HashMap::new))
}

fn step(id: &str) -> Option<&'static str> {
    steps(|s| s.get(id).copied())
}

/// Dossier d'une réunion ; l'identifiant vient de l'application : chiffres
/// seulement, jamais un chemin.
fn dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("réunion inconnue : {id}"));
    }
    Ok(root.join(id))
}

fn load(root: &Path, id: &str) -> Result<(MeetingRecord, PathBuf), String> {
    let dir = dir(root, id)?;
    if !dir.join(RECORD_FILE).is_file() {
        return Err("réunion introuvable (supprimée ?)".into());
    }
    Ok((MeetingRecord::load(&dir)?, dir))
}

/// Relit, modifie et réécrit la fiche, sans croiser une autre modification.
fn update<T>(root: &Path, id: &str, f: impl FnOnce(&mut MeetingRecord) -> T) -> Result<T, String> {
    let _guard = lock(&RECORDS);
    let (mut record, dir) = load(root, id)?;
    let result = f(&mut record);
    record.save(&dir);
    Ok(result)
}

/// Nombre d'échantillons d'une piste sur disque, d'après les en-têtes des
/// blocs (sans tout relire).
fn audio_samples(path: &Path) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return 0;
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (mut position, mut samples) = (0u64, 0u64);
    loop {
        let mut header = [0u8; 4];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let block = u64::from(u32::from_le_bytes(header));
        position += 4 + block;
        if position > length || file.seek(SeekFrom::Start(position)).is_err() {
            break;
        }
        samples += block / 2;
    }
    samples
}

/// Échantillons enregistrés pour une réunion (mémoire ou disque).
fn stored_samples(record: &MeetingRecord, dir: &Path) -> u64 {
    if record.audio == MeetingAudio::Memoire {
        memory(|m| m.get(&record.id).map_or(0, |a| a.len() as u64))
    } else {
        audio_samples(&dir.join(track(record).file()))
    }
}

fn has_audio(record: &MeetingRecord, dir: &Path) -> bool {
    stored_samples(record, dir) > 0
}

fn delete_audio(record: &MeetingRecord, dir: &Path) {
    memory(|m| m.remove(&record.id));
    meeting::delete_audio(dir);
}

/// Relit l'audio reçu, une seconde environ à la fois (mémoire ou disque) ;
/// `f` renvoie `false` pour arrêter.
fn for_each_block(
    record: &MeetingRecord,
    dir: &Path,
    mut f: impl FnMut(&[f32]) -> bool,
) -> Result<(), String> {
    if record.audio == MeetingAudio::Memoire {
        let audio = memory(|m| m.get(&record.id).cloned()).unwrap_or_default();
        for block in audio.chunks(SAMPLE_RATE as usize) {
            let block: Vec<f32> = block
                .iter()
                .map(|s| f32::from(*s) / f32::from(i16::MAX))
                .collect();
            if !f(&block) {
                break;
            }
        }
        Ok(())
    } else {
        meeting::for_each_audio_block(&dir.join(track(record).file()), plain, f)
    }
}

/// Ce que l'utilisateur a choisi pour une nouvelle réunion.
#[derive(Debug, Clone, PartialEq)]
pub struct New {
    pub title: String,
    /// Compte rendu de conférence (cours, exposé) plutôt que de réunion.
    pub conference: bool,
    pub audio: MeetingAudio,
    /// Compte rendu à la fin de la transcription.
    pub summary: bool,
    /// Nom du fichier importé ; `None` : enregistrement au micro.
    pub source: Option<String>,
    /// Distinguer les intervenants à la fin (modèles téléchargés).
    pub speakers: bool,
}

/// Crée une réunion et renvoie son identifiant (heure de début en ms).
pub fn create(root: &Path, new: &New) -> Result<String, String> {
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    // Identifiant libre, sur le disque comme en mémoire, réservé aussitôt
    // (réception de l'audio) : deux créations simultanées ne peuvent pas
    // tomber sur le même.
    let started_at_ms = live(|l| {
        let mut at = now_ms();
        loop {
            let id = at.to_string();
            if !root.join(&id).exists()
                && !l.contains_key(&id)
                && !memory(|m| m.contains_key(&id))
                && step(&id).is_none()
            {
                let track = if new.source.is_some() {
                    Track::Eux
                } else {
                    Track::Moi
                };
                l.insert(
                    id.clone(),
                    Live {
                        samples: 0,
                        audio: new.audio,
                        file: root.join(&id).join(track.file()),
                        resampler: None,
                        saved_at: Instant::now(),
                    },
                );
                return at;
            }
            at += 1;
        }
    });
    let id = started_at_ms.to_string();
    let dir = root.join(&id);
    if let Err(e) = fs::create_dir_all(&dir) {
        live(|l| l.remove(&id));
        return Err(format!("{}: {e}", dir.display()));
    }
    // Téléphone : une seule source. Un enregistrement est une
    // « conférence au micro » au sens du PC (intervenants à distinguer sur
    // la piste du micro) ; le modèle de compte rendu est choisi à part.
    let kind = if new.source.is_some() {
        MeetingKind::Fichier
    } else {
        MeetingKind::Conference
    };
    let title = match new.title.trim() {
        "" => new
            .source
            .as_deref()
            .map(file_stem)
            .unwrap_or_else(|| "Réunion".to_owned()),
        title => title.to_owned(),
    };
    let mut record = MeetingRecord::new(id.clone(), title, started_at_ms, kind);
    record.audio = new.audio;
    record.mic = new.source.is_none();
    record.source = new.source.clone();
    record.conference = Some(new.conference);
    record.output = if new.summary {
        Output::LesDeux
    } else {
        Output::Transcription
    };
    if new.source.is_some() {
        record.progress = Some(0);
    }
    // Intervenants demandés : la piste est notée dès maintenant, les tours
    // de parole viendront à la fin (reprise comprise).
    if new.speakers {
        record.speaker_track = Some(track(&record));
    }
    record.save(&dir);
    log::info!(
        "réunion {id} : {} (audio : {:?})",
        if new.source.is_some() {
            "import"
        } else {
            "enregistrement"
        },
        new.audio
    );
    Ok(id)
}

/// Nom d'un fichier sans son extension.
fn file_stem(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

/// Reçoit de l'audio (PCM 16 bits entrelacé, `rate` Hz, `channels`
/// canaux) ; renvoie la durée reçue depuis le début (ms).
pub fn feed(root: &Path, id: &str, pcm: &[i16], rate: u32, channels: u32) -> Result<u64, String> {
    if rate == 0 || channels == 0 {
        return Err(format!(
            "format audio invalide : {rate} Hz, {channels} canaux"
        ));
    }
    let interleaved: Vec<f32> = pcm.iter().map(|s| f32::from(*s) / 32768.0).collect();
    let mono = audio::downmix(&interleaved, channels as usize);
    let (samples, total, save, audio, file) = live(|l| {
        let entry = l
            .get_mut(id)
            .ok_or_else(|| "réunion terminée : plus d'audio attendu".to_owned())?;
        let resampler = entry.resampler.get_or_insert_with(|| Resampler::new(rate));
        if resampler.from != rate {
            *resampler = Resampler::new(rate);
        }
        let samples = resampler.push(&mono);
        entry.samples += samples.len() as u64;
        let save = entry.saved_at.elapsed() >= SAVE_EVERY;
        if save {
            entry.saved_at = Instant::now();
        }
        Ok::<_, String>((
            samples,
            entry.samples,
            save,
            entry.audio,
            entry.file.clone(),
        ))
    })?;
    if audio == MeetingAudio::Memoire {
        memory(|m| {
            m.entry(id.to_owned()).or_default().extend(
                samples
                    .iter()
                    .map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16),
            );
        });
    } else {
        meeting::append_audio(&file, &samples, plain)?;
    }
    let duration_ms = samples_to_ms(total);
    if save {
        update(root, id, |r| r.duration_ms = duration_ms)?;
    }
    Ok(duration_ms)
}

/// Avancement de la lecture d'un fichier importé (pourcentage).
pub fn set_progress(root: &Path, id: &str, percent: u8) -> Result<(), String> {
    update(root, id, |r| r.progress = Some(percent.min(MAX_PERCENT)))
}

/// Marque le moment présent de l'enregistrement ; renvoie sa position (ms).
pub fn mark(root: &Path, id: &str) -> Result<u64, String> {
    let at = live(|l| l.get(id).map(|e| samples_to_ms(e.samples)))
        .ok_or("aucun enregistrement en cours")?;
    update(root, id, |r| {
        r.marks.push(at);
        r.marks.sort_unstable();
    })?;
    Ok(at)
}

/// Fin de l'enregistrement ou de la lecture du fichier : la réunion attend
/// sa transcription ([`process`]).
pub fn stop(root: &Path, id: &str) -> Result<(), String> {
    let received = live(|l| l.remove(id)).map(|e| e.samples);
    let _guard = lock(&RECORDS);
    let (mut record, dir) = load(root, id)?;
    let samples = received.unwrap_or_else(|| stored_samples(&record, &dir));
    record.duration_ms = samples_to_ms(samples);
    record.progress = None;
    if samples == 0 {
        record.status = MeetingStatus::Interrompue;
        record.notes.push("Aucun son reçu.".into());
        delete_audio(&record, &dir);
    } else {
        record.status = MeetingStatus::Transcription;
    }
    record.save(&dir);
    log::info!(
        "réunion {id} : {} reçues",
        meeting::timestamp(record.duration_ms)
    );
    Ok(())
}

/// Après un arrêt de l'application : enregistrement interrompu repris s'il
/// reste de l'audio, sinon marqué interrompu. Renvoie les réunions à
/// transcrire (ni en cours de réception, ni déjà en traitement).
pub fn recover(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut pending = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if live(|l| l.contains_key(&id)) || step(&id).is_some() {
            continue;
        }
        let Ok((record, dir)) = load(root, &id) else {
            continue;
        };
        match record.status {
            MeetingStatus::Enregistrement => {
                let resumed = has_audio(&record, &dir);
                let result = update(root, &id, |r| {
                    if resumed {
                        r.status = MeetingStatus::Transcription;
                        r.duration_ms = samples_to_ms(stored_samples(r, &dir));
                        r.progress = None;
                        r.notes
                            .push("Enregistrement coupé par le téléphone : audio repris.".into());
                    } else {
                        r.status = MeetingStatus::Interrompue;
                        r.progress = None;
                        r.notes.push(
                            "Enregistrement coupé par le téléphone, audio perdu (gardé en \
                             mémoire seulement)."
                                .into(),
                        );
                    }
                });
                if resumed && result.is_ok() {
                    pending.push(id);
                }
            }
            MeetingStatus::Transcription if has_audio(&record, &dir) => pending.push(id),
            MeetingStatus::Transcription => {
                let _ = update(root, &id, |r| {
                    r.status = if r.segments.is_empty() {
                        MeetingStatus::Interrompue
                    } else {
                        MeetingStatus::Terminee
                    };
                    r.notes
                        .push("Transcription coupée, audio perdu : texte incomplet.".into());
                });
            }
            _ => {}
        }
    }
    pending.sort();
    pending
}

/// Retire l'étape en cours quand le traitement se termine, même en erreur.
struct StepGuard(String);

impl StepGuard {
    fn start(id: &str, name: &'static str) -> Result<Self, String> {
        steps(|s| match s.get(id) {
            Some(_) => Err("traitement déjà en cours".to_owned()),
            None => {
                s.insert(id.to_owned(), name);
                Ok(Self(id.to_owned()))
            }
        })
    }

    fn set(&self, name: &'static str) {
        steps(|s| s.insert(self.0.clone(), name));
    }
}

impl Drop for StepGuard {
    fn drop(&mut self) {
        steps(|s| s.remove(&self.0));
    }
}

/// Transcrit l'audio reçu (reprise là où une transcription précédente
/// s'est arrêtée), distingue les intervenants si demandé, puis rédige le
/// compte rendu si demandé. Bloquant : des
/// minutes pour une longue réunion. Le moteur est pris bloc par bloc, les
/// dictées passent entre deux.
pub fn process(root: &Path, id: &str, toml: &str, models_dir: &Path) -> Result<(), String> {
    let guard = StepGuard::start(id, "transcription")?;
    let (record, dir) = load(root, id)?;
    match record.status {
        MeetingStatus::Transcription => {}
        MeetingStatus::Terminee | MeetingStatus::Interrompue => return Ok(()),
        MeetingStatus::Enregistrement => return Err("enregistrement en cours".into()),
    }
    let track = track(&record);
    let resume_ms = record.transcribed_until(track);
    let total_ms = record.duration_ms.max(1);
    let resume = ms_to_samples(resume_ms) as u64;
    log::info!(
        "réunion {id} : transcription depuis {}",
        meeting::timestamp(resume_ms)
    );
    let mut chunker = Chunker::starting_at(resume_ms);
    let mut position = 0u64;
    let mut failure: Option<String> = None;
    let mut on_block = |block: &[f32]| -> bool {
        let start = position;
        position += block.len() as u64;
        if position <= resume {
            return true;
        }
        let skip = resume.saturating_sub(start) as usize;
        for chunk in chunker.feed(&block[skip..]) {
            if let Err(e) = transcribe(root, id, track, &chunk, total_ms) {
                failure = Some(e);
                return false;
            }
        }
        true
    };
    for_each_block(&record, &dir, &mut on_block)?;
    if failure.is_none()
        && let Some(chunk) = chunker.finish()
        && let Err(e) = transcribe(root, id, track, &chunk, total_ms)
    {
        failure = Some(e);
    }
    if let Some(e) = failure {
        // Moteur libéré, réunion supprimée… : la transcription reprendra
        // là où elle s'est arrêtée.
        return Err(e);
    }
    log::info!("réunion {id} transcrite");
    let (record, _) = load(root, id)?;
    if record.speaker_track.is_some()
        && record.speaker_turns.is_empty()
        && !record.segments.is_empty()
    {
        guard.set("speakers");
        if let Err(e) = analyze_voices(root, id, models_dir) {
            log::warn!("intervenants de {id} : {e}");
            update(root, id, |r| {
                r.notes.push(format!("Intervenants non distingués : {e}"))
            })?;
        }
    }
    let summarize = update(root, id, |r| {
        r.status = MeetingStatus::Terminee;
        r.progress = None;
        if r.audio != MeetingAudio::Conserve {
            delete_audio(r, &dir);
        }
        r.output != Output::Transcription && !r.segments.is_empty()
    })?;
    if summarize {
        guard.set("summary");
        let config = settings::parse(toml)?.0;
        if config.llm.enabled
            && let Err(e) = write_summary(root, id, &config, None)
        {
            log::warn!("compte rendu de {id} : {e}");
        }
    }
    Ok(())
}

/// Transcrit un bloc et l'ajoute à la fiche.
fn transcribe(
    root: &Path,
    id: &str,
    track: Track,
    chunk: &Chunk,
    total_ms: u64,
) -> Result<(), String> {
    let end_ms = chunk.end_ms();
    let result = if chunk.is_silent() {
        Ok(None)
    } else {
        engine::transcribe(chunk.samples.clone())
    };
    let percent = (end_ms * 100 / total_ms).min(99) as u8;
    match result {
        Ok(text) => update(root, id, |r| {
            if let Some(text) = text {
                r.segments.push(Segment {
                    track,
                    start_ms: chunk.start_ms,
                    end_ms,
                    text,
                    copied: false,
                    speaker: None,
                });
                meeting::sort_segments(&mut r.segments);
            }
            r.progress = Some(percent);
        }),
        // Moteur absent (libéré, modèle changé) : on s'arrête, reprise
        // ensuite. Autre erreur : bloc compté, on continue.
        Err(e) if !engine::loaded() => Err(e),
        Err(e) => {
            log::warn!("réunion {id} : bloc non transcrit : {e}");
            update(root, id, |r| {
                r.failed_chunks += 1;
                r.progress = Some(percent);
            })
        }
    }
}

/// Analyse des voix gardée (empreintes, pour changer le nombre
/// d'intervenants sans l'audio), dans le stockage privé de l'application.
const ANALYSIS_FILE: &str = "voix.bin";
/// Audio envoyé à l'analyse des voix par morceaux de 30 s.
const VOICE_PIECE: usize = SAMPLE_RATE as usize * 30;

/// Distingue les intervenants d'après l'audio reçu (au fil de la lecture,
/// sans tout garder en mémoire), puis répartit les répliques. Bloquant :
/// quelques minutes pour une heure.
fn analyze_voices(root: &Path, id: &str, models_dir: &Path) -> Result<(), String> {
    let (record, dir) = load(root, id)?;
    let (segmentation, embedding) =
        models::diarization_local(models_dir).ok_or("modèles des intervenants non téléchargés")?;
    // La moitié des cœurs (les rapides suffisent) : le téléphone reste fluide.
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4))
        .unwrap_or(2);
    let mut diarizer = vtt_asr::Diarizer::load(
        &segmentation,
        &embedding,
        Path::new("libonnxruntime.so"),
        threads,
    )
    .map_err(|e| e.to_string())?;
    let total = stored_samples(&record, &dir).max(1);
    let mut analysis = diarize::Analysis::new(diarize::Options::default().step);
    let mut piece: Vec<f32> = Vec::with_capacity(VOICE_PIECE);
    let mut sent = 0u64;
    let mut failure: Option<String> = None;
    let mut percent = 0u8;
    let mut send = |piece: &mut Vec<f32>, sent: &mut u64| -> Result<(), String> {
        let start_ms = samples_to_ms(*sent);
        *sent += piece.len() as u64;
        analysis
            .add(start_ms, piece, &mut diarizer)
            .map_err(|e| e.to_string())?;
        piece.clear();
        let now = (*sent * 100 / total).min(99) as u8;
        if now > percent {
            percent = now;
            update(root, id, |r| r.progress = Some(now))?;
        }
        Ok(())
    };
    for_each_block(&record, &dir, |block| {
        piece.extend_from_slice(block);
        if piece.len() >= VOICE_PIECE
            && let Err(e) = send(&mut piece, &mut sent)
        {
            failure = Some(e);
            return false;
        }
        true
    })?;
    if let Some(e) = failure {
        return Err(e);
    }
    if !piece.is_empty() {
        send(&mut piece, &mut sent)?;
    }
    analysis.finish(&mut diarizer).map_err(|e| e.to_string())?;
    let keep = update(root, id, |r| {
        r.set_speakers(&analysis);
        r.progress = None;
        r.keeps_voices()
    })?;
    if keep {
        save_analysis(&dir, &analysis)?;
    }
    Ok(())
}

fn save_analysis(dir: &Path, analysis: &diarize::Analysis) -> Result<(), String> {
    let tmp = dir.join("voix.bin.tmp");
    fs::write(&tmp, analysis.to_bytes())
        .and_then(|()| fs::rename(&tmp, dir.join(ANALYSIS_FILE)))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn load_analysis(dir: &Path) -> Result<diarize::Analysis, String> {
    let path = dir.join(ANALYSIS_FILE);
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    diarize::Analysis::from_bytes(&bytes)
}

/// Distingue les intervenants après coup (audio gardé). Bloquant.
pub fn identify(root: &Path, id: &str, models_dir: &Path) -> Result<(), String> {
    let _guard = StepGuard::start(id, "speakers")?;
    let (record, dir) = load(root, id)?;
    if record.status != MeetingStatus::Terminee {
        return Err("réunion pas encore transcrite".into());
    }
    if !has_audio(&record, &dir) {
        return Err(
            "audio non gardé : impossible de distinguer les intervenants après coup".into(),
        );
    }
    update(root, id, |r| r.speaker_track = Some(track(r)))?;
    let result = analyze_voices(root, id, models_dir);
    if result.is_err() {
        update(root, id, |r| r.progress = None)?;
    }
    result
}

/// Nombre d'intervenants imposé (`None` : selon les voix), recalculé
/// d'après l'analyse gardée.
pub fn set_speaker_count(root: &Path, id: &str, count: Option<usize>) -> Result<(), String> {
    if count.is_some_and(|c| !(1..=20).contains(&c)) {
        return Err("nombre d'intervenants : de 1 à 20".into());
    }
    let dir = dir(root, id)?;
    let analysis = load_analysis(&dir).map_err(|e| {
        log::warn!("analyse des voix de {id} : {e}");
        "analyse des voix non gardée : relancez « Distinguer les intervenants »".to_owned()
    })?;
    update(root, id, |r| {
        r.speaker_count = count;
        r.set_speakers(&analysis);
    })
}

/// Nom d'un intervenant (vide : « Intervenant N »).
pub fn rename_speaker(root: &Path, id: &str, index: usize, name: &str) -> Result<(), String> {
    let mut result = Ok(());
    update(root, id, |r| result = r.rename_speaker(index, name))?;
    result
}

/// Ne plus distinguer les intervenants : tours, noms et empreintes vocales
/// effacés.
pub fn forget_speakers(root: &Path, id: &str) -> Result<(), String> {
    let dir = dir(root, id)?;
    update(root, id, MeetingRecord::forget_speakers)?;
    let _ = fs::remove_file(dir.join(ANALYSIS_FILE));
    Ok(())
}

/// Rédige (ou refait) le compte rendu. Bloquant (requête au LLM).
pub fn summarize(root: &Path, id: &str, toml: &str, language: &str) -> Result<(), String> {
    let _guard = StepGuard::start(id, "summary")?;
    let config = settings::parse(toml)?.0;
    let language = Some(language.trim()).filter(|l| !l.is_empty());
    write_summary(root, id, &config, language)
}

fn write_summary(
    root: &Path,
    id: &str,
    config: &vtt_core::Config,
    language: Option<&str>,
) -> Result<(), String> {
    let (record, _) = load(root, id)?;
    if record.segments.is_empty() {
        return Err("transcription vide : rien à résumer".into());
    }
    let cleaner = engine::summarizer(config)
        .ok_or("nettoyage par LLM désactivé : activez-le dans les réglages")?;
    let transcript = meeting::transcript_text(&record.segments(), &record.names(), false);
    let language = language
        .map(str::to_owned)
        .unwrap_or_else(|| config.meetings.summary_language.trim().to_owned());
    log::info!(
        "compte rendu de {id} ({} caractères) via {}",
        transcript.chars().count(),
        config.llm.model
    );
    let result = cleaner.summarize(
        &record.title,
        &transcript,
        Some(language.as_str()).filter(|l| !l.is_empty()),
        record.conference_summary(),
    );
    let pricing = &config.llm.pricing;
    match result {
        Ok(cleaned) => update(root, id, |r| {
            r.summary = Some(MeetingSummary {
                text: cleaned.text,
                at_ms: now_ms(),
                model: config.llm.model.clone(),
                cost: cleaned.usage.map(|u| pricing.cost(&u)),
                currency: pricing.currency.clone(),
            });
            r.summary_error = None;
        }),
        Err(e) => {
            let message = e.to_string();
            update(root, id, |r| r.summary_error = Some(message.clone()))?;
            Err(message)
        }
    }
}

/// Ligne de la liste des réunions.
#[derive(Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub status: MeetingStatus,
    /// « transcription » ou « summary » pendant le traitement.
    pub step: Option<&'static str>,
    pub progress: Option<u8>,
    pub imported: bool,
    pub conference: bool,
    pub summary: bool,
    /// Intervenants distingués.
    pub speakers: usize,
}

fn speaker_count(record: &MeetingRecord) -> usize {
    record
        .speaker_turns
        .iter()
        .map(|t| t.speaker + 1)
        .max()
        .unwrap_or(0)
}

/// Réunions, la plus récente d'abord ; celles plus vieilles que la durée de
/// conservation des réglages sont supprimées au passage.
pub fn list(root: &Path, toml: &str) -> Result<Vec<Row>, String> {
    let keep_days = settings::parse(toml)
        .map(|(c, _)| c.meetings.keep_days)
        .unwrap_or(0);
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let now = now_ms();
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let Ok((record, dir)) = load(root, &id) else {
            continue;
        };
        let done = matches!(
            record.status,
            MeetingStatus::Terminee | MeetingStatus::Interrompue
        );
        if keep_days > 0
            && done
            && step(&id).is_none()
            && now.saturating_sub(record.started_at_ms) > u64::from(keep_days) * DAY_MS
        {
            log::info!("réunion {id} supprimée (plus de {keep_days} jours)");
            remove(&record, &dir);
            continue;
        }
        rows.push(Row {
            step: step(&id),
            imported: record.kind == MeetingKind::Fichier,
            conference: record.conference_summary(),
            summary: record.summary.is_some(),
            speakers: speaker_count(&record),
            id,
            title: record.title,
            started_at_ms: record.started_at_ms,
            duration_ms: record.duration_ms,
            status: record.status,
            progress: record.progress,
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
    Ok(rows)
}

/// Réplique affichée.
#[derive(Debug, Serialize)]
pub struct Reply {
    pub start_ms: u64,
    /// Intervenant (nom donné ou « Intervenant N ») ; `None` : inconnu.
    pub label: Option<String>,
    pub speaker: Option<usize>,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SummaryView {
    pub text: String,
    pub model: String,
    pub cost: Option<f64>,
    pub currency: String,
}

/// Réunion ouverte.
#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub row: Row,
    pub audio: MeetingAudio,
    /// Un passage peut être réécouté.
    pub replay: bool,
    pub source: Option<String>,
    pub summary_text: Option<SummaryView>,
    pub summary_error: Option<String>,
    pub notes: Vec<String>,
    pub failed_chunks: u32,
    pub marks: Vec<u64>,
    pub replies: Vec<Reply>,
    pub speaker_names: Vec<String>,
    /// Intervenants distingués, pour les reconnaître et les nommer.
    pub speaker_list: Vec<SpeakerView>,
    /// Nombre imposé (`None` : selon les voix).
    pub speaker_count: Option<usize>,
    /// Intervenants demandés mais pas encore distingués.
    pub speakers_pending: bool,
    /// Analyse gardée : le nombre peut changer sans l'audio.
    pub voices_kept: bool,
}

/// Intervenant distingué.
#[derive(Debug, Serialize)]
pub struct SpeakerView {
    pub index: usize,
    /// « Intervenant N ».
    pub label: String,
    pub name: String,
    pub talk_ms: u64,
    /// Première réplique, pour reconnaître la voix (début, et moment).
    pub first: String,
    pub first_ms: u64,
    /// `name` vient de sa présentation : la phrase, et quand.
    pub found: Option<String>,
    pub found_ms: Option<u64>,
}

/// Début d'un texte, coupé sur un caractère.
fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

pub fn detail(root: &Path, id: &str) -> Result<Detail, String> {
    let (record, dir) = load(root, id)?;
    let turns = meeting::turns(&record.segments());
    let names = record.names();
    let mut speaker_list: Vec<SpeakerView> = Vec::new();
    for reply in &turns {
        if let Some(index) = reply.speaker
            && !speaker_list.iter().any(|s| s.index == index)
        {
            speaker_list.push(SpeakerView {
                index,
                label: meeting::speaker_label(reply.track, Some(index), &[]),
                name: names.get(index).cloned().unwrap_or_default(),
                found: record.name_guess(index).map(|g| g.quote.clone()),
                found_ms: record.name_guess(index).map(|g| g.at_ms),
                talk_ms: record
                    .speaker_turns
                    .iter()
                    .filter(|t| t.speaker == index)
                    .map(|t| t.end_ms.saturating_sub(t.start_ms))
                    .sum(),
                first: excerpt(&reply.text, 90),
                first_ms: reply.start_ms,
            });
        }
    }
    speaker_list.sort_by_key(|s| s.index);
    let replies = turns
        .into_iter()
        .map(|r| Reply {
            label: r
                .speaker
                .map(|_| meeting::speaker_label(r.track, r.speaker, &names)),
            start_ms: r.start_ms,
            speaker: r.speaker,
            text: r.text,
        })
        .collect();
    // Audio temporaire : effacé après la transcription.
    let replay = record.audio != MeetingAudio::Temporaire && has_audio(&record, &dir);
    Ok(Detail {
        row: Row {
            id: record.id.clone(),
            title: record.title.clone(),
            started_at_ms: record.started_at_ms,
            duration_ms: record.duration_ms,
            status: record.status,
            step: step(id),
            progress: record.progress,
            imported: record.kind == MeetingKind::Fichier,
            conference: record.conference_summary(),
            summary: record.summary.is_some(),
            speakers: speaker_count(&record),
        },
        audio: record.audio,
        replay,
        source: record.source.clone(),
        summary_text: record.summary.as_ref().map(|s| SummaryView {
            text: s.text.clone(),
            model: s.model.clone(),
            cost: s.cost,
            currency: s.currency.clone(),
        }),
        summary_error: record.summary_error.clone(),
        notes: record.notes.clone(),
        failed_chunks: record.failed_chunks,
        marks: record.marks.clone(),
        replies,
        speaker_names: names,
        speaker_list,
        speaker_count: record.speaker_count,
        speakers_pending: record.speaker_track.is_some() && record.speaker_turns.is_empty(),
        voices_kept: dir.join(ANALYSIS_FILE).is_file(),
    })
}

pub fn rename(root: &Path, id: &str, title: &str) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("titre vide".into());
    }
    update(root, id, |r| r.title = title.to_owned())
}

/// Choix du modèle de compte rendu (réunion ou conférence).
pub fn set_conference(root: &Path, id: &str, conference: bool) -> Result<(), String> {
    update(root, id, |r| r.conference = Some(conference))
}

fn remove(record: &MeetingRecord, dir: &Path) {
    memory(|m| m.remove(&record.id));
    if let Err(e) = fs::remove_dir_all(dir) {
        log::warn!("suppression de {} : {e}", dir.display());
    }
}

/// Supprime une réunion (transcription, compte rendu, audio). Une
/// transcription en cours s'arrête au bloc suivant.
pub fn delete(root: &Path, id: &str) -> Result<(), String> {
    live(|l| l.remove(id));
    let _guard = lock(&RECORDS);
    let (record, dir) = load(root, id)?;
    remove(&record, &dir);
    log::info!("réunion {id} supprimée");
    Ok(())
}

/// Texte à partager : « markdown » (tout), « summary » (compte rendu) ou
/// « transcript » (transcription). `date` : date lisible, mise en forme par
/// l'application.
pub fn export(root: &Path, id: &str, what: &str, date: &str) -> Result<String, String> {
    let (record, _) = load(root, id)?;
    let segments = record.segments();
    match what {
        "summary" => record
            .summary
            .map(|s| format!("# {}\n\n{}", record.title, s.text.trim()))
            .ok_or_else(|| "pas encore de compte rendu".to_owned()),
        "transcript" => Ok(meeting::transcript_text(&segments, &record.names(), false)),
        _ => Ok(meeting::to_markdown(
            &record.title,
            date,
            record.duration_ms,
            record.summary.as_ref().map(|s| s.text.as_str()),
            &segments,
            &record.names(),
            false,
        )),
    }
}

/// Passage de l'audio conservé, `start_ms..end_ms`, en PCM 16 bits 16 kHz
/// pour la réécoute.
pub fn clip(root: &Path, id: &str, start_ms: u64, end_ms: u64) -> Result<Vec<i16>, String> {
    let (record, dir) = load(root, id)?;
    let (start, end) = (ms_to_samples(start_ms), ms_to_samples(end_ms.max(start_ms)));
    if record.audio == MeetingAudio::Memoire {
        return Ok(memory(|m| {
            m.get(id)
                .map(|a| a[start.min(a.len())..end.min(a.len())].to_vec())
                .unwrap_or_default()
        }));
    }
    let samples = meeting::read_audio_range(&dir.join(track(&record).file()), plain, start, end)?;
    Ok(samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect())
}

/// Réunions qui reçoivent encore de l'audio (enregistrement ou import).
pub fn receiving() -> HashSet<String> {
    live(|l| l.keys().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les identifiants sont des heures en ms, uniques dans un dossier ;
    /// les tests utilisent chacun le leur : un à la fois, sinon deux
    /// réunions de dossiers différents pourraient porter le même.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vtt-android-meetings-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn new(audio: MeetingAudio, source: Option<&str>) -> New {
        New {
            title: String::new(),
            conference: false,
            audio,
            summary: true,
            source: source.map(str::to_owned),
            speakers: false,
        }
    }

    /// Une seconde de « parole » (sinusoïde) en 48 kHz stéréo.
    fn second_48k_stereo() -> Vec<i16> {
        (0..48_000)
            .flat_map(|i| {
                let s = ((i as f32 * 0.05).sin() * 8_000.0) as i16;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn resampling_keeps_the_duration_across_calls() {
        for rate in [8_000, 11_025, 44_100, 48_000] {
            let mut resampler = Resampler::new(rate);
            let mut out = 0;
            // Envois de tailles irrégulières, 10 s en tout.
            let total = rate as usize * 10;
            let mut sent = 0;
            let mut size = 997;
            while sent < total {
                let n = size.min(total - sent);
                out += resampler.push(&vec![0.1; n]).len();
                sent += n;
                size = size * 7 % 4_093 + 100;
            }
            let expected = SAMPLE_RATE as usize * 10;
            assert!(
                out.abs_diff(expected) <= 2,
                "{rate} Hz : {out} échantillons au lieu de {expected}"
            );
        }
    }

    #[test]
    fn recorded_audio_is_kept_marked_and_replayed() {
        let _serial = lock(&SERIAL);
        let root = root("record");
        let id = create(&root, &new(MeetingAudio::Conserve, None)).unwrap();
        let (record, dir) = load(&root, &id).unwrap();
        assert_eq!(record.status, MeetingStatus::Enregistrement);
        assert_eq!(record.kind, MeetingKind::Conference);
        assert_eq!(record.title, "Réunion");
        assert!(!record.conference_summary());
        for _ in 0..3 {
            feed(&root, &id, &second_48k_stereo(), 48_000, 2).unwrap();
        }
        assert_eq!(mark(&root, &id).unwrap(), 3_000);
        let duration = feed(&root, &id, &second_48k_stereo(), 48_000, 2).unwrap();
        assert_eq!(duration, 4_000);
        assert!(receiving().contains(&id));
        stop(&root, &id).unwrap();
        assert!(
            feed(&root, &id, &[0; 10], 16_000, 1).is_err(),
            "après l'arrêt"
        );
        let (record, _) = load(&root, &id).unwrap();
        assert_eq!(record.status, MeetingStatus::Transcription);
        assert_eq!(
            (record.duration_ms, record.marks.clone()),
            (4_000, vec![3_000])
        );
        assert_eq!(audio_samples(&dir.join(Track::Moi.file())), 64_000);
        let clip = clip(&root, &id, 1_000, 1_500).unwrap();
        assert_eq!(clip.len(), 8_000);
        assert!(clip.iter().any(|s| s.abs() > 1_000));
        // Moteur non démarré : la transcription s'arrête, la réunion attend
        // la reprise.
        assert!(process(&root, &id, "", Path::new("/nulle-part")).is_err());
        assert_eq!(recover(&root), std::slice::from_ref(&id));
        assert_eq!(step(&id), None);
        delete(&root, &id).unwrap();
        assert!(!dir.exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn memory_only_audio_never_touches_the_disk() {
        let _serial = lock(&SERIAL);
        let root = root("memory");
        let id = create(&root, &new(MeetingAudio::Memoire, None)).unwrap();
        feed(&root, &id, &vec![1_000; 16_000], 16_000, 1).unwrap();
        let dir = root.join(&id);
        assert!(!dir.join(Track::Moi.file()).exists());
        stop(&root, &id).unwrap();
        assert_eq!(clip(&root, &id, 0, 500).unwrap().len(), 8_000);
        assert!(detail(&root, &id).unwrap().replay);
        // Application arrêtée : l'audio en mémoire est perdu.
        memory(|m| m.remove(&id));
        update(&root, &id, |r| r.status = MeetingStatus::Enregistrement).unwrap();
        assert!(recover(&root).is_empty());
        let (record, _) = load(&root, &id).unwrap();
        assert_eq!(record.status, MeetingStatus::Interrompue);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn interrupted_recordings_resume_from_the_disk() {
        let _serial = lock(&SERIAL);
        let root = root("recover");
        let id = create(&root, &new(MeetingAudio::Temporaire, None)).unwrap();
        feed(&root, &id, &vec![500; 32_000], 16_000, 1).unwrap();
        // Processus tué : plus rien en mémoire.
        live(|l| l.remove(&id));
        assert_eq!(recover(&root), std::slice::from_ref(&id));
        let (record, _) = load(&root, &id).unwrap();
        assert_eq!(record.status, MeetingStatus::Transcription);
        assert_eq!(record.duration_ms, 2_000);
        assert_eq!(record.notes.len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn imports_are_named_after_the_file_and_listed() {
        let _serial = lock(&SERIAL);
        let root = root("import");
        let id = create(
            &root,
            &New {
                conference: true,
                ..new(MeetingAudio::Temporaire, Some("Note vocale de Julie.opus"))
            },
        )
        .unwrap();
        feed(&root, &id, &vec![0; 8_000], 8_000, 1).unwrap();
        set_progress(&root, &id, 40).unwrap();
        let (record, dir) = load(&root, &id).unwrap();
        assert_eq!(record.title, "Note vocale de Julie");
        assert_eq!(record.kind, MeetingKind::Fichier);
        assert_eq!(record.progress, Some(40));
        assert!(dir.join(Track::Eux.file()).exists(), "piste du fichier");
        stop(&root, &id).unwrap();
        rename(&root, &id, " Message de Julie ").unwrap();
        assert!(rename(&root, &id, " ").is_err());
        let rows = list(&root, "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Message de Julie");
        assert!(rows[0].imported && rows[0].conference);
        assert!((999..=1_000).contains(&rows[0].duration_ms));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finished_meetings_are_exported_without_track_names() {
        let _serial = lock(&SERIAL);
        let root = root("export");
        let id = create(&root, &new(MeetingAudio::Conserve, None)).unwrap();
        stop(&root, &id).unwrap();
        update(&root, &id, |r| {
            r.status = MeetingStatus::Terminee;
            r.title = "Point projet".into();
            r.segments.push(Segment {
                track: Track::Moi,
                start_ms: 12_000,
                end_ms: 15_000,
                text: "On commence par le budget.".into(),
                copied: false,
                speaker: None,
            });
            r.summary = Some(MeetingSummary {
                text: "## Décisions\n- Budget validé".into(),
                at_ms: 0,
                model: "m".into(),
                cost: None,
                currency: "€".into(),
            });
        })
        .unwrap();
        let md = export(&root, &id, "markdown", "14/11/2026").unwrap();
        assert!(md.contains("# Point projet\n\n14/11/2026"));
        assert!(md.contains("[00:12] On commence par le budget."));
        assert!(!md.contains("Moi"));
        assert_eq!(
            export(&root, &id, "summary", "").unwrap(),
            "# Point projet\n\n## Décisions\n- Budget validé"
        );
        let detail = detail(&root, &id).unwrap();
        assert_eq!(detail.replies.len(), 1);
        assert_eq!(detail.replies[0].label, None);
        assert!(!detail.replay, "aucun audio reçu");
        assert!(dir(&root, "../etc").is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn keep_days_purges_old_meetings() {
        let _serial = lock(&SERIAL);
        let root = root("purge");
        let id = create(&root, &new(MeetingAudio::Temporaire, None)).unwrap();
        stop(&root, &id).unwrap();
        update(&root, &id, |r| {
            r.started_at_ms = 1;
            r.status = MeetingStatus::Terminee;
        })
        .unwrap();
        assert_eq!(list(&root, "").unwrap().len(), 1, "gardées par défaut");
        let toml = "[meetings]\nkeep_days = 30\n";
        assert!(list(&root, toml).unwrap().is_empty());
        assert!(!root.join(&id).exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn speakers_are_requested_named_and_forgotten() {
        let _serial = lock(&SERIAL);
        let root = root("speakers");
        let id = create(
            &root,
            &New {
                speakers: true,
                ..new(MeetingAudio::Conserve, None)
            },
        )
        .unwrap();
        assert!(detail(&root, &id).unwrap().speakers_pending);
        // Silence seulement : rien à transcrire, le moteur n'est pas appelé.
        feed(&root, &id, &vec![0; 16_000 * 8], 16_000, 1).unwrap();
        stop(&root, &id).unwrap();
        update(&root, &id, |r| {
            r.segments.push(Segment {
                track: Track::Moi,
                start_ms: 0,
                end_ms: 2_000,
                text: "Bonjour.".into(),
                copied: false,
                speaker: None,
            })
        })
        .unwrap();
        process(&root, &id, "", Path::new("/nulle-part")).unwrap();
        let (record, _) = load(&root, &id).unwrap();
        assert_eq!(record.status, MeetingStatus::Terminee);
        assert!(
            record
                .notes
                .iter()
                .any(|n| n.contains("modèles des intervenants")),
            "{:?}",
            record.notes
        );
        assert!(identify(&root, &id, Path::new("/nulle-part")).is_err());
        assert_eq!(step(&id), None);
        assert!(
            set_speaker_count(&root, &id, Some(2)).is_err(),
            "analyse absente"
        );
        assert!(set_speaker_count(&root, &id, Some(0)).is_err());

        // Tours de parole connus : répliques nommées, noms modifiables.
        update(&root, &id, |r| {
            r.speaker_turns = vec![diarize::Turn {
                start_ms: 0,
                end_ms: 2_000,
                speaker: 1,
            }];
        })
        .unwrap();
        rename_speaker(&root, &id, 1, "  Paul ").unwrap();
        let view = detail(&root, &id).unwrap();
        assert!(!view.speakers_pending);
        assert_eq!(view.speaker_list.len(), 1);
        assert_eq!(view.speaker_list[0].label, "Intervenant 2");
        assert_eq!(view.speaker_list[0].name, "Paul");
        assert_eq!(view.speaker_list[0].talk_ms, 2_000);
        assert_eq!(view.replies[0].label.as_deref(), Some("Paul"));
        assert_eq!(view.row.speakers, 2);
        assert!(
            export(&root, &id, "transcript", "")
                .unwrap()
                .contains("Paul : Bonjour.")
        );
        rename_speaker(&root, &id, 1, "").unwrap();
        assert!(load(&root, &id).unwrap().0.speaker_names.is_empty());
        forget_speakers(&root, &id).unwrap();
        let view = detail(&root, &id).unwrap();
        assert!(view.speaker_list.is_empty() && !view.speakers_pending);
        assert_eq!(view.replies[0].label, None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn long_texts_are_cut_on_characters() {
        assert_eq!(excerpt("Bonjour", 90), "Bonjour");
        assert_eq!(excerpt("Ça va très bien", 6), "Ça va…");
    }
}
