//! Transcription différée, diarisation vocale, compte rendu et rapport complet
//! des réunions sur Android.

use std::path::Path;

use vtt_core::SAMPLE_RATE;
use vtt_core::config::MeetingAudio;
use vtt_core::diarize;
use vtt_core::meeting::{
    self, Chunk, Chunker, HtmlOptions, MeetingRecord, MeetingStatus, MeetingSummary, Output,
    Segment, Track,
};

use super::storage::{
    delete_audio, dir, for_each_block, has_audio, load, load_analysis, now_ms, replay,
    save_analysis, steps, stored_samples, track, update,
};
use crate::{engine, models, settings};

/// Audio envoyé à l'analyse des voix par morceaux de 30 s.
const VOICE_PIECE: usize = SAMPLE_RATE as usize * 30;

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
/// compte rendu si demandé. Bloquant : des minutes pour une longue réunion.
/// Le moteur est pris bloc par bloc, les dictées passent entre deux.
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
    let resume = meeting::ms_to_samples(resume_ms) as u64;
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
        let start_ms = meeting::samples_to_ms(*sent);
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
    let book = super::storage::load_voicebook_or_empty(root);
    let keep = update(root, id, |r| {
        r.set_speakers_with_voicebook(&analysis, Some(&book));
        r.progress = None;
        r.keeps_voices()
    })?;
    if keep {
        save_analysis(&dir, &analysis)?;
    }
    Ok(())
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
    meeting::validate_speaker_count(count)?;
    let dir = dir(root, id)?;
    let analysis = load_analysis(&dir).map_err(|e| {
        log::warn!("analyse des voix de {id} : {e}");
        "analyse des voix non gardée : relancez « Distinguer les intervenants »".to_owned()
    })?;
    let book = super::storage::load_voicebook_or_empty(root);
    update(root, id, |r| {
        r.speaker_count = count;
        r.set_speakers_with_voicebook(&analysis, Some(&book));
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
    meeting::delete_analysis(&dir);
    Ok(())
}

/// Voix mémorisées dans le carnet local (sans vecteurs biométriques).
pub fn voicebook_list(root: &Path) -> Result<Vec<vtt_core::voicebook::VoiceEntryView>, String> {
    let _guard = super::storage::lock(&super::storage::VOICEBOOK);
    Ok(super::storage::load_voicebook_locked(root)?.views())
}

/// Mémorise explicitement la voix d'un intervenant d'une réunion dans le
/// carnet local (opt-in).
pub fn enroll_speaker_voice(
    root: &Path,
    id: &str,
    index: usize,
    name: &str,
) -> Result<vtt_core::voicebook::VoiceEntryView, String> {
    let (record, dir) = load(root, id)?;
    let target_name = if !name.trim().is_empty() {
        name.trim().to_owned()
    } else {
        record
            .names()
            .get(index)
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "donnez d'abord un nom à cet intervenant".to_owned())?
    };
    let analysis = load_analysis(&dir).map_err(|e| {
        log::warn!("analyse des voix de {id} : {e}");
        "empreintes vocales non gardées pour cette réunion".to_owned()
    })?;
    let profiles = analysis.speaker_profiles(&diarize::Options {
        speakers: record.speaker_count,
        ..diarize::Options::default()
    });
    let profile = profiles
        .into_iter()
        .find(|p| p.speaker == index)
        .ok_or_else(|| "intervenant introuvable dans l'analyse vocale".to_owned())?;
    let samples = u32::try_from(profile.samples).unwrap_or(u32::MAX);
    let view = {
        let _guard = super::storage::lock(&super::storage::VOICEBOOK);
        let mut book = super::storage::load_voicebook_locked(root)?;
        let entry = book.enroll(&target_name, &profile.embedding, samples, now_ms())?;
        super::storage::save_voicebook_locked(root, &book)?;
        vtt_core::voicebook::VoiceEntryView::from(&entry)
    };
    update(root, id, |r| {
        let _ = r.rename_speaker(index, &view.name);
    })?;
    log::info!(
        "carnet de voix : voix de « {} » mémorisée depuis la réunion {id}",
        view.name
    );
    Ok(view)
}

/// Renomme une voix mémorisée dans le carnet local.
pub fn rename_voicebook_entry(root: &Path, id: &str, name: &str) -> Result<(), String> {
    let _guard = super::storage::lock(&super::storage::VOICEBOOK);
    let mut book = super::storage::load_voicebook_locked(root)?;
    book.rename(id, name, now_ms())?;
    super::storage::save_voicebook_locked(root, &book)
}

/// Oublie une voix mémorisée en un clic ; renvoie `true` si elle existait.
pub fn remove_voicebook_entry(root: &Path, id: &str) -> Result<bool, String> {
    let _guard = super::storage::lock(&super::storage::VOICEBOOK);
    let mut book = super::storage::load_voicebook_locked(root)?;
    let removed = book.remove(id);
    if removed {
        super::storage::save_voicebook_locked(root, &book)?;
    }
    Ok(removed)
}

/// Efface toutes les voix du carnet local.
pub fn clear_voicebook(root: &Path) {
    let _guard = super::storage::lock(&super::storage::VOICEBOOK);
    vtt_core::voicebook::VoiceBook::delete_file(&super::storage::voicebook_path(root));
    log::info!("carnet de voix effacé");
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

/// Rédige (ou refait) le rapport complet : à la demande seulement.
/// Bloquant : plusieurs requêtes au LLM pour un long enregistrement
/// (avancement dans `progress`).
pub fn report(root: &Path, id: &str, toml: &str, language: &str) -> Result<(), String> {
    let _guard = StepGuard::start(id, "report")?;
    let config = settings::parse(toml)?.0;
    let (record, _) = load(root, id)?;
    if record.segments.is_empty() {
        return Err("transcription vide : rien à analyser".into());
    }
    let cleaner = engine::summarizer(&config)
        .ok_or("nettoyage par LLM désactivé : activez-le dans les réglages")?;
    let transcript = meeting::transcript_text(&record.segments(), &record.names(), false);
    let language = Some(language.trim())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| config.meetings.summary_language.trim().to_owned());
    log::info!(
        "rapport de {id} ({} caractères) via {}",
        transcript.chars().count(),
        config.llm.model
    );
    update(root, id, |r| r.progress = Some(0))?;
    let result = cleaner.report(
        &record.title,
        &transcript,
        record.duration_ms,
        Some(language.as_str()).filter(|l| !l.is_empty()),
        record.conference_summary(),
        &mut |done, total| {
            let percent = (done * 100 / total.max(1)) as u8;
            let _ = update(root, id, |r| r.progress = Some(percent));
        },
    );
    let pricing = &config.llm.pricing;
    match result {
        Ok(drafted) => update(root, id, |r| {
            let mut report = drafted.report;
            if let Some(previous) = &r.report {
                report.keep_done(previous);
            }
            report.at_ms = now_ms();
            report.model = config.llm.model.clone();
            report.cost = drafted.usage.map(|u| pricing.cost(&u));
            report.currency = pricing.currency.clone();
            r.report = Some(report);
            r.report_error = None;
            r.progress = None;
        }),
        Err(e) => {
            let message = e.to_string();
            update(root, id, |r| {
                r.report_error = Some(message.clone());
                r.progress = None;
            })?;
            Err(message)
        }
    }
}

/// Rapport en HTML pour l'application (thème imposé, ▶ et cases reliés à
/// l'application). `date` : date lisible, mise en forme par l'application.
pub fn report_html(root: &Path, id: &str, date: &str, theme: &str) -> Result<String, String> {
    let (record, dir) = load(root, id)?;
    let options = HtmlOptions {
        app: true,
        replay: replay(&record, &dir),
        theme: Some(theme.to_owned()),
        tracks: false,
    };
    record
        .report_html(date, &options)
        .ok_or_else(|| "pas encore de rapport".to_owned())
}

/// Action du plan d'action cochée ou décochée.
pub fn set_action_done(root: &Path, id: &str, index: usize, done: bool) -> Result<(), String> {
    let mut result = Ok(());
    update(root, id, |r| {
        match r
            .report
            .as_mut()
            .and_then(|report| report.actions.get_mut(index))
        {
            Some(action) => action.done = done,
            None => result = Err("action introuvable".to_owned()),
        }
    })?;
    result
}
