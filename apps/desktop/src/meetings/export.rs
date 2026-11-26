//! Export Markdown/texte, contexte des questions LLM et plages d'extraits
//! audio pour les réunions PC.

use vtt_core::meeting::{self, ExportOptions, MeetingRecord};

/// Contexte donné au LLM pour une question : au plus ce nombre de caractères.
const MAX_CONTEXT_CHARS: usize = 40_000;

/// Extrait réécouté : un peu avant le passage, puis 30 s.
const CLIP_BEFORE_MS: u64 = 2_000;
const CLIP_MS: u64 = 30_000;

const DESKTOP_EXPORT: ExportOptions = ExportOptions {
    include_track_names: true,
    summary_heading: false,
    allow_empty_summary: true,
};

pub(super) fn meeting_date(record: &MeetingRecord) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(record.started_at_ms) * 1_000_000)
        .ok()
        .and_then(|d| {
            d.format(time::macros::format_description!(
                "[day]/[month]/[year] [hour]:[minute]"
            ))
            .ok()
        })
        .unwrap_or_default()
}

pub(super) fn markdown(record: &MeetingRecord) -> String {
    let date = meeting_date(record);
    meeting::export_meeting_text(record, "markdown", &date, DESKTOP_EXPORT).unwrap_or_default()
}

pub(super) fn report_html(
    record: &MeetingRecord,
    app: bool,
    replay: bool,
    theme: Option<String>,
) -> Result<String, String> {
    let date = meeting_date(record);
    let options = meeting::HtmlOptions {
        app,
        replay: app && replay,
        theme: theme.filter(|t| t == "light" || t == "dark"),
        tracks: true,
    };
    record
        .report_html(&date, &options)
        .ok_or_else(|| "aucun rapport complet rédigé pour cet enregistrement".to_owned())
}

pub(super) fn select_copy_text(record: &MeetingRecord, part: &str) -> String {
    let date = if part == "summary" || part == "transcript" {
        String::new()
    } else {
        meeting_date(record)
    };
    meeting::export_meeting_text(record, part, &date, DESKTOP_EXPORT).unwrap_or_default()
}

/// Nom de fichier sûr d'après le titre.
pub(super) fn file_name(title: &str, extension: &str) -> String {
    meeting::safe_file_name(title, extension)
}

pub(super) fn clip_sample_range(start_ms: u64) -> (usize, usize) {
    let from = meeting::ms_to_samples(start_ms.saturating_sub(CLIP_BEFORE_MS));
    let to = meeting::ms_to_samples(start_ms.saturating_add(CLIP_MS));
    (from, to)
}

/// Rassemble les extraits pertinents (ou la transcription entière pour un seul
/// enregistrement si elle tient sous `MAX_CONTEXT_CHARS`) et le nombre de
/// sources retenues (sans index sémantique).
#[cfg(test)]
pub(super) fn build_ask_context(
    records: &[MeetingRecord],
    question: &str,
    single_record: bool,
) -> (String, usize) {
    let rag = build_ask_context_semantic(records, &[], question, None, single_record);
    (rag.context, rag.sources)
}

/// Version RAG hybride (classement lexical BM25-lite + sémantique RRF +
/// diversité inter-réunions + inclusion du rapport complet).
pub(super) fn build_ask_context_semantic(
    records: &[MeetingRecord],
    indices: &[Option<vtt_core::semantic::MeetingIndex>],
    question: &str,
    query_vec: Option<&[f32]>,
    single_record: bool,
) -> vtt_core::semantic::RagContext {
    let refs: Vec<Option<&vtt_core::semantic::MeetingIndex>> =
        indices.iter().map(Option::as_ref).collect();
    vtt_core::semantic::build_rag_context(
        records,
        &refs,
        question,
        query_vec,
        single_record,
        MAX_CONTEXT_CHARS,
    )
}
