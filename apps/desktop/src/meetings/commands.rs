//! Commandes Tauri exposées à la fenêtre principale pour la page Réunions.

use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use vtt_core::calls::{self, Policy};
use vtt_core::config::{MEETING_KEEP_DAYS, MeetingAudio, MeetingsConfig, TRANSLATION_LANGUAGES};
use vtt_core::meeting::{
    self, DEFAULT_EXCERPT_CHARS, MeetingKind, MeetingRecord, MeetingStatus, Output, Track,
};
use vtt_core::voicebook::{VoiceEntryView, VoiceRecognition};

use super::export::{build_ask_context, clip_sample_range, file_name, markdown, select_copy_text};
use super::storage::voice_source;
use super::{MeetingState, MeetingsState, lock, now_ms};
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};
use crate::speakers::{self, VoicesState};

#[derive(Serialize)]
pub struct MeetingRow {
    id: String,
    title: String,
    started_at_ms: u64,
    duration_ms: u64,
    status: MeetingStatus,
    segments: usize,
    has_summary: bool,
    audio_kept: bool,
    kind: MeetingKind,
    progress: Option<u8>,
    identifying: bool,
}

#[derive(Serialize)]
pub struct MeetingsView {
    recording: Option<String>,
    elapsed_ms: u64,
    meetings: Vec<MeetingRow>,
    settings: MeetingsConfig,
    keep_days: &'static [u32],
    llm_enabled: bool,
    /// Services d'appel reconnus et le choix pour chacun.
    providers: Vec<ProviderView>,
    languages: &'static [&'static str],
}

#[derive(Serialize)]
pub struct ProviderView {
    key: &'static str,
    label: &'static str,
    browser: bool,
    policy: Policy,
}

#[tauri::command]
pub fn meetings_view(
    meetings: State<'_, MeetingsState>,
    config: State<'_, SharedConfig>,
) -> MeetingsView {
    let mut rows: Vec<MeetingRow> = fs::read_dir(&meetings.dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            let (record, dir) = meetings.load(&id).ok()?;
            Some(MeetingRow {
                audio_kept: record.audio == MeetingAudio::Conserve
                    && dir.join(Track::Moi.file()).is_file(),
                id: record.id,
                title: record.title,
                started_at_ms: record.started_at_ms,
                duration_ms: record.duration_ms,
                status: record.status,
                segments: record.segments.len(),
                has_summary: record.summary.is_some(),
                kind: record.kind,
                progress: record.progress,
                identifying: record.identifying,
            })
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
    let state = meetings.state();
    let active = lock(&meetings.active).as_ref().map(|a| a.session.id());
    let config = settings::read(&config);
    MeetingsView {
        recording: active,
        elapsed_ms: state.elapsed_ms,
        meetings: rows,
        settings: config.meetings.clone(),
        keep_days: MEETING_KEEP_DAYS,
        llm_enabled: config.llm.enabled,
        providers: calls::providers()
            .map(|p| ProviderView {
                key: p.key,
                label: p.label,
                browser: p.browser,
                policy: config.meetings.policy(p.key),
            })
            .collect(),
        languages: TRANSLATION_LANGUAGES,
    }
}

#[derive(Serialize)]
pub struct Turn {
    track: Track,
    speaker: Option<usize>,
    /// Nom affiché pour un intervenant (vide : le nom de la piste, choisi
    /// par la page selon le type d'enregistrement).
    who: String,
    start_ms: u64,
    text: String,
}

/// Intervenant distingué, pour le reconnaître et le nommer.
#[derive(Serialize)]
pub struct SpeakerView {
    index: usize,
    label: String,
    name: String,
    talk_ms: u64,
    /// Première réplique (début).
    first: String,
    /// `name` vient de sa présentation : la phrase, et quand.
    found: Option<String>,
    found_ms: Option<u64>,
    /// `name` vient du carnet de voix local (opt-in) : entrée et confiance.
    recognized: Option<VoiceRecognition>,
}

#[derive(Serialize)]
pub struct MeetingDetail {
    record: MeetingRecord,
    turns: Vec<Turn>,
    busy: bool,
    audio_kept: bool,
    /// Un passage peut être réécouté.
    replay: bool,
    speakers: Vec<SpeakerView>,
    /// « Distinguer les intervenants » possible (modèles, audio ou analyse
    /// disponibles).
    can_identify: bool,
    /// Analyse des voix conservée : nombre d'intervenants modifiable.
    voices_kept: bool,
}

#[tauri::command]
pub fn meeting_detail(
    meetings: State<'_, MeetingsState>,
    voices: State<'_, VoicesState>,
    id: String,
) -> Result<MeetingDetail, String> {
    let (record, dir) = meetings.load(&id)?;
    let replies = meeting::turns(&record.segments());
    let names = record.names();
    let speakers = meeting::speaker_summaries(&record, &replies, DEFAULT_EXCERPT_CHARS)
        .into_iter()
        .map(|s| SpeakerView {
            index: s.index,
            label: s.label,
            name: s.name,
            talk_ms: s.talk_ms,
            first: s.first,
            found: s.found,
            found_ms: s.found_ms,
            recognized: s.recognized,
        })
        .collect();
    let turns = replies
        .into_iter()
        .map(|r| Turn {
            who: r
                .speaker
                .map(|_| meeting::speaker_label(r.track, r.speaker, &names))
                .unwrap_or_default(),
            track: r.track,
            speaker: r.speaker,
            start_ms: r.start_ms,
            text: r.text,
        })
        .collect();
    let busy = meetings.busy(&id);
    let voices_kept = speakers::analysis_kept(&dir);
    Ok(MeetingDetail {
        audio_kept: dir.join(Track::Moi.file()).is_file() && record.audio == MeetingAudio::Conserve,
        can_identify: !busy
            && !record.identifying
            && record.status == MeetingStatus::Terminee
            && record.speaker_turns.is_empty()
            && voices.installed()
            && (voices_kept || voice_source(&record, &dir).is_some()),
        voices_kept,
        busy,
        replay: record.replayable(&dir),
        speakers,
        record,
        turns,
    })
}

#[derive(Serialize)]
pub struct OfferView {
    label: &'static str,
    browser: bool,
}

#[tauri::command]
pub fn offer_view(meetings: State<'_, MeetingsState>) -> Option<OfferView> {
    lock(&meetings.offer).map(|(p, _)| OfferView {
        label: p.label,
        browser: p.browser,
    })
}

/// `choice` : « enregistrer », « plus-tard », « toujours », « jamais ».
#[tauri::command]
pub fn offer_answer(meetings: State<'_, MeetingsState>, choice: String) -> Result<(), String> {
    meetings.answer_offer(&choice)
}

#[tauri::command]
pub fn meeting_state(meetings: State<'_, MeetingsState>) -> MeetingState {
    meetings.state()
}

#[tauri::command]
pub fn start_meeting(meetings: State<'_, MeetingsState>) -> Result<String, String> {
    meetings.start(None)
}

#[tauri::command]
pub fn start_conference(
    meetings: State<'_, MeetingsState>,
    mic: bool,
    system: bool,
    output: Output,
) -> Result<String, String> {
    meetings.start_conference(mic, system, output)
}

/// Choisit un fichier audio ou vidéo et lance sa transcription (`None` :
/// annulé).
#[tauri::command(async)]
pub fn import_recording(
    app: AppHandle,
    meetings: State<'_, MeetingsState>,
    output: Output,
) -> Result<Option<String>, String> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Transcrire un fichier audio ou vidéo")
        .add_filter("Audio et vidéo", crate::decode::EXTENSIONS)
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    meetings.import(path, output).map(Some)
}

/// Passage trouvé par la recherche.
#[derive(Serialize)]
pub struct SearchResult {
    id: String,
    title: String,
    kind: MeetingKind,
    started_at_ms: u64,
    start_ms: u64,
    parts: vtt_core::search::Parts,
    replay: bool,
}

/// Au plus ce nombre de passages trouvés.
const MAX_RESULTS: usize = 200;

#[tauri::command(async)]
pub fn search_meetings(meetings: State<'_, MeetingsState>, query: String) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for record in meetings.all() {
        let replay = record.replayable(&meetings.dir.join(&record.id));
        for hit in vtt_core::search::find(&record.segments(), &query) {
            results.push(SearchResult {
                id: record.id.clone(),
                title: record.title.clone(),
                kind: record.kind,
                started_at_ms: record.started_at_ms,
                start_ms: hit.start_ms,
                parts: hit.parts,
                replay,
            });
            if results.len() >= MAX_RESULTS {
                return results;
            }
        }
    }
    results
}

#[derive(Serialize)]
pub struct Answer {
    text: String,
    cost: Option<f64>,
    currency: String,
    /// Enregistrements consultés.
    sources: usize,
}

/// Question au LLM sur un enregistrement (`id`) ou sur tous.
#[tauri::command(async)]
pub fn ask_meetings(
    meetings: State<'_, MeetingsState>,
    question: String,
    id: Option<String>,
) -> Result<Answer, String> {
    let question = question.trim();
    if question.is_empty() {
        return Err("question vide".into());
    }
    let config = meetings.config();
    let cleaner = settings::summarizer(&config)
        .ok_or("nettoyage LLM désactivé : activez-le dans les réglages")?;
    let records = match &id {
        Some(id) => vec![meetings.load(id)?.0],
        None => meetings.all(),
    };
    let (context, sources) = build_ask_context(&records, question, id.is_some());
    if context.is_empty() {
        return Err(
            "aucun passage ne correspond aux mots de la question : reformulez avec les mots \
             employés pendant l'enregistrement"
                .into(),
        );
    }
    log::info!(
        "question sur {sources} enregistrement(s), {} caractères de contexte",
        context.len()
    );
    let cleaned = cleaner
        .answer(question, &context)
        .map_err(|e| e.to_string())?;
    let pricing = &config.llm.pricing;
    if let Some(usage) = &cleaned.usage {
        costs::record(&meetings.app.state::<LedgerStore>(), None, usage, pricing);
    }
    Ok(Answer {
        text: cleaned.text,
        cost: cleaned.usage.map(|u| pricing.cost(&u)),
        currency: pricing.currency.clone(),
        sources,
    })
}

/// Extrait audio (WAV en `data:` URL) qui commence peu avant `start_ms` :
/// audio conservé (les deux pistes mêlées) ou fichier importé d'origine.
#[tauri::command(async)]
pub fn meeting_clip(
    meetings: State<'_, MeetingsState>,
    id: String,
    start_ms: u64,
) -> Result<String, String> {
    let (record, dir) = meetings.load(&id)?;
    if !record.replayable(&dir) {
        return Err("audio non conservé pour cet enregistrement".into());
    }
    let (from, to) = clip_sample_range(start_ms);
    let samples = match (&record.kind, &record.source_path) {
        (MeetingKind::Fichier, Some(path)) => {
            let mut read = Vec::new();
            let mut position = 0usize;
            crate::decode::decode(
                Path::new(path),
                |block| {
                    let start = from.saturating_sub(position).min(block.len());
                    let end = to.saturating_sub(position).min(block.len());
                    read.extend_from_slice(&block[start..end]);
                    position += block.len();
                    position < to
                },
                |_| {},
            )?;
            read
        }
        _ => {
            let mut mixed: Vec<f32> = Vec::new();
            for track in [Track::Moi, Track::Eux] {
                let part = meeting::read_audio_range(
                    &dir.join(track.file()),
                    vtt_platform::unprotect,
                    from,
                    to,
                )?;
                if mixed.len() < part.len() {
                    mixed.resize(part.len(), 0.0);
                }
                for (m, s) in mixed.iter_mut().zip(part) {
                    *m = (*m + s).clamp(-1.0, 1.0);
                }
            }
            mixed
        }
    };
    if samples.is_empty() {
        return Err("rien à réécouter à ce moment".into());
    }
    Ok(vtt_llm::data_url(
        "audio/wav",
        &meeting::wav_bytes(&samples),
    ))
}

#[tauri::command]
pub fn stop_meeting(meetings: State<'_, MeetingsState>) -> Result<(), String> {
    meetings.stop()
}

#[tauri::command]
pub fn rename_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    title: String,
) -> Result<(), String> {
    let title = meeting::validate_title(&title)
        .ok()
        .filter(|t| t.chars().count() <= 200)
        .ok_or("titre vide ou trop long (200 caractères au plus)")?;
    meetings.update(&id, |r| r.title = title)
}

#[tauri::command(async)]
pub fn summarize_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    language: Option<String>,
) -> Result<(), String> {
    meetings.summarize(&id, language)
}

#[tauri::command]
pub fn identify_speakers(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    meetings.identify(&id)
}

/// Nombre d'intervenants imposé (`None` : déduit des voix), recalculé
/// d'après l'analyse conservée.
#[tauri::command]
pub fn set_speaker_count(
    meetings: State<'_, MeetingsState>,
    id: String,
    count: Option<usize>,
) -> Result<(), String> {
    meeting::validate_speaker_count(count)?;
    let dir = meetings.meeting_dir(&id)?;
    let analysis = speakers::load_analysis(&dir).map_err(|e| {
        log::warn!("analyse des voix de {id} : {e}");
        "analyse des voix non conservée : relancez « Distinguer les intervenants »".to_owned()
    })?;
    let book = meetings.voicebook.load_or_empty();
    meetings.update(&id, |r| {
        r.speaker_count = count;
        r.set_speakers_with_voicebook(&analysis, Some(&book));
    })
}

/// Nom d'un intervenant (vide : « Intervenant N »).
#[tauri::command]
pub fn rename_speaker(
    meetings: State<'_, MeetingsState>,
    id: String,
    index: usize,
    name: String,
) -> Result<(), String> {
    let mut result = Ok(());
    meetings.update(&id, |r| result = r.rename_speaker(index, &name))?;
    result
}

/// Ne plus distinguer les intervenants : tours, noms et empreintes vocales
/// effacés.
#[tauri::command]
pub fn forget_speakers(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    let dir = meetings.meeting_dir(&id)?;
    meetings.update(&id, MeetingRecord::forget_speakers)?;
    speakers::delete_analysis(&dir);
    Ok(())
}

#[tauri::command]
pub fn voices_view(voices: State<'_, VoicesState>) -> speakers::VoicesView {
    voices.view()
}

#[tauri::command]
pub fn download_voices(voices: State<'_, VoicesState>) {
    voices.download();
}

/// Détermine le nom à mémoriser dans le carnet pour un intervenant : celui
/// saisi/transmis s'il n'est pas vide, sinon le nom déjà connu sur la fiche.
pub(super) fn resolve_enroll_name(
    record: &MeetingRecord,
    index: usize,
    provided: Option<&str>,
) -> Result<String, String> {
    if let Some(name) = provided.map(str::trim).filter(|n| !n.is_empty()) {
        return Ok(name.to_owned());
    }
    record
        .names()
        .get(index)
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "donnez d'abord un nom à cet intervenant".to_owned())
}

/// Voix mémorisées dans le carnet local (sans vecteurs biométriques).
#[tauri::command]
pub fn voicebook_view(meetings: State<'_, MeetingsState>) -> Result<Vec<VoiceEntryView>, String> {
    meetings.voicebook.views()
}

/// Mémorise explicitement la voix d'un intervenant d'une réunion dans le
/// carnet local chiffré (opt-in).
#[tauri::command]
pub fn enroll_speaker_voice(
    meetings: State<'_, MeetingsState>,
    id: String,
    index: usize,
    name: Option<String>,
) -> Result<VoiceEntryView, String> {
    let (record, dir) = meetings.load(&id)?;
    let target_name = resolve_enroll_name(&record, index, name.as_deref())?;
    let analysis = speakers::load_analysis(&dir).map_err(|e| {
        log::warn!("analyse des voix de {id} : {e}");
        "empreintes vocales non conservées pour cette réunion".to_owned()
    })?;
    let entry = meetings.voicebook.enroll_from_analysis(
        &analysis,
        record.speaker_count,
        index,
        &target_name,
        now_ms(),
    )?;
    meetings.update(&id, |r| {
        let _ = r.rename_speaker(index, &entry.name);
    })?;
    log::info!(
        "carnet de voix : voix de « {} » mémorisée depuis la réunion {id}",
        entry.name
    );
    let _ = meetings.app.emit("voicebook", ());
    Ok(entry)
}

/// Renomme une voix du carnet local.
#[tauri::command]
pub fn rename_voicebook_entry(
    meetings: State<'_, MeetingsState>,
    id: String,
    name: String,
) -> Result<(), String> {
    meetings.voicebook.rename(&id, &name, now_ms())?;
    let _ = meetings.app.emit("voicebook", ());
    Ok(())
}

/// Oublie une voix du carnet local en un clic.
#[tauri::command]
pub fn remove_voicebook_entry(
    meetings: State<'_, MeetingsState>,
    id: String,
) -> Result<(), String> {
    if meetings.voicebook.remove(&id)? {
        let _ = meetings.app.emit("voicebook", ());
    }
    Ok(())
}

/// Efface toutes les voix du carnet local.
#[tauri::command]
pub fn clear_voicebook(meetings: State<'_, MeetingsState>) {
    meetings.voicebook.clear();
    log::info!("carnet de voix effacé");
    let _ = meetings.app.emit("voicebook", ());
}

#[tauri::command]
pub fn delete_meeting(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    if meetings.busy(&id) || meetings.load(&id).is_ok_and(|(r, _)| r.identifying) {
        return Err("réunion en cours de traitement : attendez la fin".into());
    }
    let dir = meetings.meeting_dir(&id)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("suppression : {e}"))?;
    log::info!("réunion {id} supprimée");
    let _ = meetings.app.emit("meetings", ());
    Ok(())
}

/// Nouvelle transcription de tout l'audio conservé (après un changement de
/// modèle, par exemple).
#[tauri::command]
pub fn retranscribe_meeting(meetings: State<'_, MeetingsState>, id: String) -> Result<(), String> {
    if meetings.busy(&id) {
        return Err("réunion déjà en cours de transcription".into());
    }
    let (record, dir) = meetings.load(&id)?;
    if record.identifying {
        return Err("intervenants en cours d'identification : attendez la fin".into());
    }
    if !dir.join(Track::Moi.file()).is_file() {
        return Err("audio non conservé pour cette réunion".into());
    }
    meetings.inner().transcribe_audio(record, dir, true)
}

#[tauri::command]
pub fn copy_meeting(
    meetings: State<'_, MeetingsState>,
    id: String,
    part: String,
) -> Result<(), String> {
    let (record, _) = meetings.load(&id)?;
    let text = select_copy_text(&record, &part);
    vtt_platform::copy_text(&text)
}

#[tauri::command(async)]
pub fn export_meeting(
    app: AppHandle,
    meetings: State<'_, MeetingsState>,
    id: String,
) -> Result<Option<String>, String> {
    let (record, _) = meetings.load(&id)?;
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Exporter la réunion")
        .add_filter("Markdown", &["md"])
        .set_file_name(file_name(&record.title, "md"))
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, markdown(&record)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// Exporte l'audio conservé d'une piste en WAV (déchiffré).
#[tauri::command(async)]
pub fn export_meeting_audio(
    app: AppHandle,
    meetings: State<'_, MeetingsState>,
    id: String,
    track: Track,
) -> Result<Option<String>, String> {
    let (record, dir) = meetings.load(&id)?;
    let samples = meeting::read_audio(&dir.join(track.file()), vtt_platform::unprotect)?;
    if samples.is_empty() {
        return Err("aucun audio conservé pour cette piste".into());
    }
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Exporter l'audio")
        .add_filter("WAV", &["wav"])
        .set_file_name(file_name(
            &format!("{} - {}", record.title, track.label()),
            "wav",
        ))
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, meeting::wav_bytes(&samples))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub fn save_meeting_settings(
    app: AppHandle,
    paths: State<'_, Paths>,
    meetings: State<'_, MeetingsState>,
    settings: MeetingsConfig,
) -> Result<(), String> {
    let (mut config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    settings.validate()?;
    config.meetings = MeetingsConfig {
        // Le prompt se choisit dans la page Prompts.
        summary_prompt: config.meetings.summary_prompt.clone(),
        summary_language: settings.summary_language.trim().to_owned(),
        ..settings
    };
    paths.write_config(&config)?;
    crate::app_settings::apply(&app, &paths, &config);
    if config.meetings.diarize {
        app.state::<VoicesState>().download();
    }
    // Durée de conservation raccourcie : appliquée tout de suite.
    meetings.recover();
    log::info!("réglages des réunions appliqués");
    Ok(())
}
