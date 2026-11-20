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
/// sources retenues.
pub(super) fn build_ask_context(
    records: &[MeetingRecord],
    question: &str,
    single_record: bool,
) -> (String, usize) {
    let keywords = vtt_core::search::keywords(question);
    let mut context = String::new();
    let mut sources = 0;
    for record in records {
        let segments = record.segments();
        let mut lines =
            vtt_core::search::excerpts(&record.title, &segments, &keywords, &record.names());
        // Un seul enregistrement : tout, s'il tient.
        if single_record {
            let all = vtt_core::search::excerpts(
                &record.title,
                &segments,
                &[String::new()],
                &record.names(),
            );
            if all.iter().map(String::len).sum::<usize>() <= MAX_CONTEXT_CHARS {
                lines = all;
            }
        }
        if lines.is_empty() {
            continue;
        }
        sources += 1;
        for line in lines {
            if context.len() + line.len() > MAX_CONTEXT_CHARS {
                break;
            }
            context.push_str(&line);
            context.push('\n');
        }
        if context.len() >= MAX_CONTEXT_CHARS {
            break;
        }
    }
    (context, sources)
}
