//! Vues JSON, édition, suppression, export et réécoute des réunions sur
//! Android, appuyés sur `vtt_core::meeting`.

use std::fs;
use std::path::Path;

use serde::Serialize;
use vtt_core::config::MeetingAudio;
use vtt_core::meeting::{
    self, DEFAULT_EXCERPT_CHARS, ExportOptions, MeetingKind, MeetingStatus, SpeakerSummary,
};

use super::storage::{
    RECORDS, live, load, lock, memory, now_ms, plain, remove, replay, step, track, update,
};
use crate::settings;

const ANDROID_EXPORT: ExportOptions = ExportOptions {
    include_track_names: false,
    summary_heading: true,
    allow_empty_summary: false,
};

/// Ligne de la liste des réunions.
#[derive(Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub status: MeetingStatus,
    /// « transcription », « speakers », « summary » ou « report » pendant le traitement.
    pub step: Option<&'static str>,
    pub progress: Option<u8>,
    pub imported: bool,
    pub conference: bool,
    pub summary: bool,
    pub report: bool,
    /// Intervenants distingués.
    pub speakers: usize,
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
        if done && step(&id).is_none() && meeting::is_expired(record.started_at_ms, now, keep_days)
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
            report: record.report.is_some(),
            speakers: meeting::speaker_count(&record),
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
    pub report_view: Option<ReportView>,
    pub report_error: Option<String>,
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

/// Aperçu du rapport (onglet Rapport) ; le document s'ouvre à part.
#[derive(Debug, Serialize)]
pub struct ReportView {
    pub synthesis: String,
    pub key_points: Vec<String>,
    pub chapters: Vec<String>,
    pub actions: usize,
    pub actions_done: usize,
    pub at_ms: u64,
    pub model: String,
    pub cost: Option<f64>,
    pub currency: String,
}

/// Intervenant distingué (format JSON partagé avec `vtt_core::meeting::SpeakerSummary`).
pub type SpeakerView = SpeakerSummary;

pub fn detail(root: &Path, id: &str) -> Result<Detail, String> {
    let (record, dir) = load(root, id)?;
    let turns = meeting::turns(&record.segments());
    let names = record.names();
    let speaker_list = meeting::speaker_summaries(&record, &turns, DEFAULT_EXCERPT_CHARS);
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
    let replay = replay(&record, &dir);
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
            report: record.report.is_some(),
            speakers: meeting::speaker_count(&record),
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
        report_view: record.report.as_ref().map(|r| ReportView {
            synthesis: r.synthesis.clone(),
            key_points: r.key_points.iter().map(|p| p.text.clone()).collect(),
            chapters: r.chapters.iter().map(|c| c.title.clone()).collect(),
            actions: r.actions.len(),
            actions_done: r.actions.iter().filter(|a| a.done).count(),
            at_ms: r.at_ms,
            model: r.model.clone(),
            cost: r.cost,
            currency: r.currency.clone(),
        }),
        report_error: record.report_error.clone(),
        notes: record.notes.clone(),
        failed_chunks: record.failed_chunks,
        marks: record.marks.clone(),
        replies,
        speaker_names: names,
        speaker_list,
        speaker_count: record.speaker_count,
        speakers_pending: record.speaker_track.is_some() && record.speaker_turns.is_empty(),
        voices_kept: meeting::has_analysis(&dir),
    })
}

pub fn rename(root: &Path, id: &str, title: &str) -> Result<(), String> {
    let title = meeting::validate_title(title)?;
    update(root, id, |r| r.title = title)
}

/// Choix du modèle de compte rendu (réunion ou conférence).
pub fn set_conference(root: &Path, id: &str, conference: bool) -> Result<(), String> {
    update(root, id, |r| r.conference = Some(conference))
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

/// Texte à partager : « markdown » (tout), « summary » (compte rendu),
/// « transcript » (transcription), « report » (rapport en Markdown, sans la
/// transcription) ou « report_html » (rapport en HTML autonome, avec la
/// transcription). `date` : date lisible, mise en forme par l'application.
pub fn export(root: &Path, id: &str, what: &str, date: &str) -> Result<String, String> {
    let (record, _) = load(root, id)?;
    meeting::export_meeting_text(&record, what, date, ANDROID_EXPORT)
}

/// Passage de l'audio conservé, `start_ms..end_ms`, en PCM 16 bits 16 kHz
/// pour la réécoute.
pub fn clip(root: &Path, id: &str, start_ms: u64, end_ms: u64) -> Result<Vec<i16>, String> {
    let (record, dir) = load(root, id)?;
    let (start, end) = (
        meeting::ms_to_samples(start_ms),
        meeting::ms_to_samples(end_ms.max(start_ms)),
    );
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
