//! Création, réception audio au fil de l'eau, marquage, arrêt et reprise après
//! interruption d'une réunion sur Android.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use vtt_core::audio;
use vtt_core::config::MeetingAudio;
use vtt_core::meeting::{self, MeetingKind, MeetingRecord, MeetingStatus, Output, Track};

use super::storage::{
    Live, RECORDS, Resampler, delete_audio, has_audio, live, load, lock, memory, now_ms, plain,
    step, stored_samples, track, update,
};

/// Durée d'un enregistrement en cours écrite dans `meeting.json`.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Avancement de la lecture d'un fichier importé : 0 à 100 %.
const MAX_PERCENT: u8 = 100;

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
    let duration_ms = meeting::samples_to_ms(total);
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
    let at = live(|l| l.get(id).map(|e| meeting::samples_to_ms(e.samples)))
        .ok_or("aucun enregistrement en cours")?;
    update(root, id, |r| {
        r.marks.push(at);
        r.marks.sort_unstable();
    })?;
    Ok(at)
}

/// Fin de l'enregistrement ou de la lecture du fichier : la réunion attend
/// sa transcription ([`super::process`]).
pub fn stop(root: &Path, id: &str) -> Result<(), String> {
    let received = live(|l| l.remove(id)).map(|e| e.samples);
    let _guard = lock(&RECORDS);
    let (mut record, dir) = load(root, id)?;
    let samples = received.unwrap_or_else(|| stored_samples(&record, &dir));
    record.duration_ms = meeting::samples_to_ms(samples);
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
                        r.duration_ms = meeting::samples_to_ms(stored_samples(r, &dir));
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

/// Réunions qui reçoivent encore de l'audio (enregistrement ou import).
pub fn receiving() -> HashSet<String> {
    live(|l| l.keys().cloned().collect())
}
