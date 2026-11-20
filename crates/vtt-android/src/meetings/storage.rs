//! Stockage local, état partagé en mémoire, rééchantillonnage et lecture audio
//! des réunions Android, appuyés sur `vtt_core::meeting`.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use vtt_core::SAMPLE_RATE;
use vtt_core::config::MeetingAudio;
use vtt_core::diarize;
use vtt_core::meeting::{self, MeetingKind, MeetingRecord, Track};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Audio sur le stockage privé : déjà chiffré par Android (chiffrement par
/// fichier, lié au déverrouillage du téléphone). Les blocs sont écrits tels
/// quels, au format du PC.
pub(super) fn plain(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}

/// Piste où va l'audio : le micro pour un enregistrement, « l'autre côté »
/// pour un fichier importé (comme sur le PC).
pub(super) fn track(record: &MeetingRecord) -> Track {
    if record.kind == MeetingKind::Fichier {
        Track::Eux
    } else {
        Track::Moi
    }
}

/// Conversion au fil de l'eau vers 16 kHz : l'état (reste d'entrée,
/// position) est gardé d'un envoi à l'autre, sans décalage cumulé.
#[derive(Debug)]
pub(super) struct Resampler {
    pub(super) from: u32,
    /// Entrée pas encore consommée.
    carry: Vec<f32>,
    /// Position (dans `carry`) du prochain échantillon de sortie.
    position: f64,
}

impl Resampler {
    pub(super) fn new(from: u32) -> Self {
        Self {
            from,
            carry: Vec::new(),
            position: 0.0,
        }
    }

    pub(super) fn push(&mut self, input: &[f32]) -> Vec<f32> {
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
pub(super) struct Live {
    /// Échantillons reçus (16 kHz).
    pub(super) samples: u64,
    /// Où va l'audio (choix de l'utilisateur) et fichier de la piste.
    pub(super) audio: MeetingAudio,
    pub(super) file: PathBuf,
    pub(super) resampler: Option<Resampler>,
    pub(super) saved_at: Instant,
}

/// Réunions qui reçoivent de l'audio, par identifiant.
static LIVE: Mutex<Option<HashMap<String, Live>>> = Mutex::new(None);
/// Audio « en mémoire seulement », par identifiant (16 bits : deux fois
/// moins de place que des flottants, 115 Mo par heure).
static MEMORY: Mutex<Option<HashMap<String, Vec<i16>>>> = Mutex::new(None);
/// Ce que le moteur fait pour une réunion : « transcription », « speakers »,
/// « summary » ou « report ».
static STEPS: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
/// Une seule modification de `meeting.json` à la fois (transcription et
/// changement de titre, par exemple).
pub(super) static RECORDS: Mutex<()> = Mutex::new(());

pub(super) fn live<T>(f: impl FnOnce(&mut HashMap<String, Live>) -> T) -> T {
    f(lock(&LIVE).get_or_insert_with(HashMap::new))
}

pub(super) fn memory<T>(f: impl FnOnce(&mut HashMap<String, Vec<i16>>) -> T) -> T {
    f(lock(&MEMORY).get_or_insert_with(HashMap::new))
}

pub(super) fn steps<T>(f: impl FnOnce(&mut HashMap<String, &'static str>) -> T) -> T {
    f(lock(&STEPS).get_or_insert_with(HashMap::new))
}

pub(super) fn step(id: &str) -> Option<&'static str> {
    steps(|s| s.get(id).copied())
}

/// Dossier d'une réunion ; l'identifiant vient de l'application : chiffres
/// seulement, jamais un chemin.
pub(super) fn dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !meeting::is_valid_id(id) {
        return Err(format!("réunion inconnue : {id}"));
    }
    Ok(root.join(id))
}

pub(super) fn load(root: &Path, id: &str) -> Result<(MeetingRecord, PathBuf), String> {
    meeting::load_meeting(root, id).map_err(|e| {
        if e == "réunion introuvable" {
            "réunion introuvable (supprimée ?)".into()
        } else {
            e
        }
    })
}

/// Relit, modifie et réécrit la fiche, sans croiser une autre modification.
pub(super) fn update<T>(
    root: &Path,
    id: &str,
    f: impl FnOnce(&mut MeetingRecord) -> T,
) -> Result<T, String> {
    let _guard = lock(&RECORDS);
    let (mut record, dir) = load(root, id)?;
    let result = f(&mut record);
    record.save(&dir);
    Ok(result)
}

/// Nombre d'échantillons d'une piste sur disque, d'après les en-têtes des
/// blocs (sans tout relire).
pub(super) fn audio_samples(path: &Path) -> u64 {
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
pub(super) fn stored_samples(record: &MeetingRecord, dir: &Path) -> u64 {
    if record.audio == MeetingAudio::Memoire {
        memory(|m| m.get(&record.id).map_or(0, |a| a.len() as u64))
    } else {
        audio_samples(&dir.join(track(record).file()))
    }
}

pub(super) fn has_audio(record: &MeetingRecord, dir: &Path) -> bool {
    stored_samples(record, dir) > 0
}

pub(super) fn delete_audio(record: &MeetingRecord, dir: &Path) {
    memory(|m| m.remove(&record.id));
    meeting::delete_audio(dir);
}

/// Réécoute possible : audio gardé (l'audio temporaire est effacé après la
/// transcription).
pub(super) fn replay(record: &MeetingRecord, dir: &Path) -> bool {
    record.audio != MeetingAudio::Temporaire && has_audio(record, dir)
}

/// Relit l'audio reçu, une seconde environ à la fois (mémoire ou disque) ;
/// `f` renvoie `false` pour arrêter.
pub(super) fn for_each_block(
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

pub(super) fn remove(record: &MeetingRecord, dir: &Path) {
    memory(|m| m.remove(&record.id));
    if let Err(e) = fs::remove_dir_all(dir) {
        log::warn!("suppression de {} : {e}", dir.display());
    }
}

pub(super) fn save_analysis(dir: &Path, analysis: &diarize::Analysis) -> Result<(), String> {
    meeting::save_analysis(dir, analysis, plain)
}

pub(super) fn load_analysis(dir: &Path) -> Result<diarize::Analysis, String> {
    meeting::load_analysis(dir, plain)
}
