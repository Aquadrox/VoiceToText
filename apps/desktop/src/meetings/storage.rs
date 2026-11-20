//! Stockage, reprise, retranscription, identification vocale et compte rendu
//! des réunions côté PC, appuyés sur `vtt_core::meeting`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

use tauri::{Emitter, Manager};
use vtt_core::config::MeetingAudio;
use vtt_core::meeting::{
    self, Chunker, MeetingKind, MeetingRecord, MeetingStatus, MeetingSummary, Track,
};
use vtt_core::usage::Usage;

use super::{Meetings, lock, now_ms};
use crate::costs::{self, LedgerStore};
use crate::settings;
use crate::speakers::{self, VoiceFeed, VoicesState};

/// Audio à relire pour distinguer les intervenants après coup.
pub(super) enum VoiceSource {
    /// Piste conservée (chiffrée).
    Track(PathBuf),
    /// Fichier importé.
    File(PathBuf),
}

/// Audio où relire les voix après coup : la piste conservée, ou le fichier
/// importé s'il existe toujours.
pub(super) fn voice_source(record: &MeetingRecord, dir: &Path) -> Option<VoiceSource> {
    match record.kind {
        MeetingKind::Fichier => record
            .source_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map(VoiceSource::File),
        _ => {
            let track = record.voice_track()?;
            let path = dir.join(track.file());
            (record.audio == MeetingAudio::Conserve && path.is_file())
                .then_some(VoiceSource::Track(path))
        }
    }
}

/// Résout le dossier d'une réunion par son identifiant horodaté (refuse toute
/// traversée de chemin ou dossier sans enregistrement).
pub(super) fn resolve_meeting_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    meeting::meeting_dir(root, id)
}

impl Meetings {
    pub(super) fn meeting_dir(&self, id: &str) -> Result<PathBuf, String> {
        resolve_meeting_dir(&self.dir, id)
    }

    /// Dernier état connu : en mémoire si elle est en cours, sinon sur disque.
    pub(super) fn load(&self, id: &str) -> Result<(MeetingRecord, PathBuf), String> {
        let dir = self.meeting_dir(id)?;
        if let Some(session) = lock(&self.sessions).get(id) {
            return Ok((lock(&session.record).clone(), dir));
        }
        Ok((MeetingRecord::load(&dir)?, dir))
    }

    pub(super) fn busy(&self, id: &str) -> bool {
        lock(&self.sessions).contains_key(id)
    }

    /// Modifie une réunion terminée (renommage, compte rendu).
    pub(super) fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut MeetingRecord),
    ) -> Result<(), String> {
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

    /// Tous les enregistrements (en cours compris), les plus récents d'abord.
    pub(super) fn all(&self) -> Vec<MeetingRecord> {
        let mut records: Vec<MeetingRecord> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| self.load(&e.file_name().to_string_lossy()).ok())
            .map(|(record, _)| record)
            .collect();
        records.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
        records
    }

    /// Reprend les réunions interrompues par un arrêt de l'application
    /// (audio sur disque : transcription de la suite), et supprime celles qui
    /// ont dépassé la durée de conservation.
    pub fn recover(self: &Arc<Self>) {
        let keep_days = self.config().meetings.keep_days;
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let now = now_ms();
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(mut record) = MeetingRecord::load(&dir) else {
                continue;
            };
            if meeting::is_expired(record.started_at_ms, now, keep_days) {
                let age_days = now.saturating_sub(record.started_at_ms) / meeting::DAY_MS;
                log::info!("réunion {} supprimée ({age_days} jours)", record.id);
                if let Err(e) = fs::remove_dir_all(&dir) {
                    log::warn!("suppression de {} : {e}", dir.display());
                }
                continue;
            }
            if record.identifying
                && !self.busy(&record.id)
                && record.status == MeetingStatus::Terminee
            {
                // Arrêt pendant « Distinguer les intervenants » : à relancer.
                record.identifying = false;
                record.save(&dir);
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
    pub(super) fn transcribe_audio(
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
            .map(|(_, s, _)| meeting::samples_to_ms(s.len() as u64))
            .max()
            .unwrap_or(record.duration_ms);
        record.duration_ms = record.duration_ms.max(duration);
        record.status = MeetingStatus::Transcription;
        record.save(&dir);
        let id = record.id.clone();
        let session = self.new_session(dir, record);
        lock(&self.sessions).insert(id, Arc::clone(&session));
        // Voix : toute la piste, relue bloc par bloc dans un thread à part
        // (la transcription, elle, reprend où elle s'était arrêtée).
        let reader = lock(&session.voices).as_ref().and_then(VoiceFeed::sender);
        if let (Some(mut sender), Some(track)) = (reader, session.voice_track) {
            let path = session.dir.join(track.file());
            let spawned = thread::Builder::new()
                .name("voices-read".into())
                .spawn(move || {
                    let read = meeting::for_each_audio_block(&path, vtt_platform::unprotect, |b| {
                        sender.push(b)
                    });
                    if let Err(e) = read {
                        log::warn!("voix : relecture de {} : {e}", path.display());
                    }
                    sender.flush();
                });
            if let Err(e) = spawned {
                log::warn!("voix : relecture de l'audio : {e}");
            }
        }
        for (track, samples, from_ms) in tracks {
            let from = meeting::ms_to_samples(from_ms);
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
        let transcript = meeting::transcript_text(&record.segments(), &record.names(), true);
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
            record.conference_summary(),
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

    /// Distingue les intervenants d'un enregistrement terminé : d'après
    /// l'analyse conservée, ou en relisant l'audio (dans un thread à part).
    pub(super) fn identify(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let (record, dir) = self.load(id)?;
        if self.busy(id) || record.identifying || record.status != MeetingStatus::Terminee {
            return Err("enregistrement en cours de traitement : attendez la fin".into());
        }
        if record.voice_track().is_none() {
            return Err("aucune piste où distinguer des intervenants".into());
        }
        if let Ok(analysis) = speakers::load_analysis(&dir) {
            return self.update(id, |r| r.set_speakers(&analysis));
        }
        let source = voice_source(&record, &dir)
            .ok_or("audio non conservé : impossible de distinguer les intervenants après coup")?;
        let voices = self.app.state::<VoicesState>();
        let feed = voices.feed().ok_or(
            "modèles pas encore téléchargés : activez « Distinguer les intervenants » \
             dans Réglages → Réunions",
        )?;
        let mut sender = feed.sender().ok_or("analyse des voix indisponible")?;
        self.update(id, |r| r.identifying = true)?;
        let meetings = Arc::clone(self);
        let id = id.to_owned();
        thread::Builder::new()
            .name("voices-identify".into())
            .spawn(move || {
                let read = match &source {
                    VoiceSource::Track(path) => {
                        meeting::for_each_audio_block(path, vtt_platform::unprotect, |b| {
                            sender.push(b)
                        })
                    }
                    VoiceSource::File(path) => {
                        crate::decode::decode(path, |b| sender.push(b), |_| {}).map(|_| ())
                    }
                };
                sender.flush();
                drop(sender);
                let result = read.and_then(|()| feed.finish());
                let saved = match &result {
                    Ok(analysis) if record.keeps_voices() => {
                        speakers::save_analysis(&dir, analysis)
                    }
                    _ => Ok(()),
                };
                if let Err(e) = saved {
                    log::warn!("analyse des voix non enregistrée : {e}");
                }
                let updated = meetings.update(&id, |r| {
                    match &result {
                        Ok(analysis) => r.set_speakers(analysis),
                        Err(e) => r.notes.push(format!("Intervenants non distingués : {e}")),
                    }
                    r.identifying = false;
                });
                if let Err(e) = updated {
                    log::warn!("intervenants de {id} : {e}");
                }
            })
            .map_err(|e| format!("analyse des voix : {e}"))?;
        Ok(())
    }
}
