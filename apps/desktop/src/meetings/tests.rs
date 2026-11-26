use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use vtt_core::config::MeetingAudio;
use vtt_core::diarize;
use vtt_core::meeting::{
    self, MeetingKind, MeetingRecord, MeetingStatus, MeetingSummary, Output, RECORD_FILE, Segment,
    Track,
};

use super::capture::{collect_fresh_turns, default_title};
use super::export::{build_ask_context, clip_sample_range, file_name, markdown, select_copy_text};
use super::storage::{VoiceSource, resolve_meeting_dir, voice_source};

fn record() -> MeetingRecord {
    MeetingRecord {
        id: "1793790000000".into(),
        title: "Point projet".into(),
        started_at_ms: 1_793_790_000_000,
        duration_ms: 60_000,
        status: MeetingStatus::Transcription,
        audio: MeetingAudio::Temporaire,
        system: true,
        segments: vec![
            Segment {
                track: Track::Moi,
                start_ms: 0,
                end_ms: 7_000,
                text: "Bonjour.".into(),
                copied: false,
                speaker: None,
            },
            Segment {
                track: Track::Eux,
                start_ms: 2_000,
                end_ms: 9_500,
                text: "Salut.".into(),
                copied: true,
                speaker: None,
            },
        ],
        summary: None,
        summary_error: None,
        failed_chunks: 0,
        notes: Vec::new(),
        echo_filter: true,
        echo_chunks: 0,
        kind: MeetingKind::Reunion,
        output: Output::LesDeux,
        mic: true,
        source: None,
        source_path: None,
        progress: None,
        speaker_track: None,
        speaker_turns: Vec::new(),
        speaker_names: Vec::new(),
        speaker_guesses: Vec::new(),
        speaker_recognized: Vec::new(),
        speaker_count: None,
        identifying: false,
        marks: Vec::new(),
        conference: None,
        report: None,
        report_error: None,
    }
}

#[test]
fn record_round_trips_and_resumes_after_each_track() {
    let dir = std::env::temp_dir().join(format!("vtt-meetings-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    record().save(&dir);
    let loaded = MeetingRecord::load(&dir).unwrap();
    assert_eq!(loaded.segments, record().segments);
    assert_eq!(loaded.status, MeetingStatus::Transcription);
    assert_eq!(loaded.transcribed_until(Track::Moi), 7_000);
    assert_eq!(loaded.transcribed_until(Track::Eux), 9_500);
    assert!(markdown(&loaded).contains("**Moi** [00:00] : Bonjour."));
    // Fichier d'une version précédente : sans les champs des
    // intervenants, relu tel quel.
    let json = fs::read_to_string(dir.join(RECORD_FILE)).unwrap();
    assert!(!json.contains("speaker"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn speakers_replace_the_other_track_label() {
    let mut record = record();
    record.speaker_track = Some(Track::Eux);
    record.speaker_turns = vec![diarize::Turn {
        start_ms: 2_000,
        end_ms: 9_500,
        speaker: 0,
    }];
    record.speaker_names = vec!["Paul".into()];
    assert!(markdown(&record).contains("**Paul** [00:02] : Salut."));
    assert!(markdown(&record).contains("**Moi** [00:00] : Bonjour."));
    let json = serde_json::to_string(&record).unwrap();
    let read: MeetingRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(read.speaker_turns, record.speaker_turns);
    assert_eq!(read.speaker_names, ["Paul"]);
}

#[test]
fn long_replies_are_cut_on_characters() {
    assert_eq!(meeting::excerpt("Bonjour", 90), "Bonjour");
    assert_eq!(meeting::excerpt("Ça va très bien", 6), "Ça va…");
}

#[test]
fn file_names_drop_forbidden_characters() {
    assert_eq!(
        file_name("Point : budget / Q4?", "md"),
        "Point - budget - Q4-.md"
    );
    assert_eq!(file_name(" Réunion. ", "wav"), "Réunion.wav");
}

fn temp_meeting_dir(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vtt-meetings-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn resolve_meeting_dir_rejects_traversal_and_missing_records() {
    let root = temp_meeting_dir("resolve");
    assert!(resolve_meeting_dir(&root, "").is_err());
    assert!(resolve_meeting_dir(&root, "../autre").is_err());
    assert!(resolve_meeting_dir(&root, "123/456").is_err());
    assert!(resolve_meeting_dir(&root, "123abc").is_err());
    assert!(resolve_meeting_dir(&root, "999999").is_err());

    // Dossier existant sans meeting.json : toujours introuvable.
    let empty = root.join("111111");
    fs::create_dir_all(&empty).unwrap();
    assert!(resolve_meeting_dir(&root, "111111").is_err());

    let valid = root.join("1793790000000");
    fs::create_dir_all(&valid).unwrap();
    record().save(&valid);
    assert_eq!(resolve_meeting_dir(&root, "1793790000000").unwrap(), valid);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn voice_source_resolves_imported_files_and_kept_tracks_only() {
    let dir = temp_meeting_dir("voice-source");

    let mut imported = record();
    imported.kind = MeetingKind::Fichier;
    let audio_file = dir.join("enregistrement.wav");
    imported.source_path = Some(audio_file.display().to_string());
    assert!(voice_source(&imported, &dir).is_none());

    fs::write(&audio_file, b"RIFF").unwrap();
    assert!(matches!(
        voice_source(&imported, &dir),
        Some(VoiceSource::File(p)) if p == audio_file
    ));

    let mut meeting_rec = record();
    let track_path = dir.join(Track::Eux.file());
    fs::write(&track_path, b"audio").unwrap();
    // Temporaire : ne doit pas réutiliser la piste.
    meeting_rec.audio = MeetingAudio::Temporaire;
    assert!(voice_source(&meeting_rec, &dir).is_none());

    // Conservé : piste existante trouvée.
    meeting_rec.audio = MeetingAudio::Conserve;
    assert!(matches!(
        voice_source(&meeting_rec, &dir),
        Some(VoiceSource::Track(p)) if p == track_path
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn collect_fresh_turns_returns_only_uncopied_and_marks_all_copied() {
    let mut rec = record();
    // Segment 0 (Moi) non copié, segment 1 (Eux) déjà copié.
    let first = collect_fresh_turns(&mut rec);
    assert_eq!(first, "Moi : Bonjour.");
    assert!(rec.segments.iter().all(|s| s.copied));
    assert!(collect_fresh_turns(&mut rec).is_empty());

    // Nouvelle réplique avec nom d'intervenant reconnu.
    rec.speaker_track = Some(Track::Eux);
    rec.speaker_turns = vec![diarize::Turn {
        start_ms: 10_000,
        end_ms: 14_000,
        speaker: 0,
    }];
    rec.speaker_names = vec!["Paul".into()];
    rec.segments.push(Segment {
        track: Track::Eux,
        start_ms: 10_000,
        end_ms: 14_000,
        text: "Merci pour le point.".into(),
        copied: false,
        speaker: None,
    });
    assert_eq!(collect_fresh_turns(&mut rec), "Paul : Merci pour le point.");
}

#[test]
fn build_ask_context_filters_across_meetings_and_keeps_full_single_record() {
    let mut budget = record();
    budget.title = "Budget Q4".into();
    budget.segments = vec![Segment {
        track: Track::Moi,
        start_ms: 0,
        end_ms: 5_000,
        text: "Le budget marketing est validé à dix mille euros.".into(),
        copied: false,
        speaker: None,
    }];

    let mut hiring = record();
    hiring.id = "1793790009999".into();
    hiring.title = "Recrutement".into();
    hiring.segments = vec![Segment {
        track: Track::Eux,
        start_ms: 0,
        end_ms: 5_000,
        text: "Deux développeurs rejoignent l'équipe en janvier.".into(),
        copied: false,
        speaker: None,
    }];

    let all = [budget.clone(), hiring];
    let (ctx, sources) = build_ask_context(&all, "Quel est le budget ?", false);
    assert_eq!(sources, 1);
    assert!(ctx.contains("budget marketing"), "{ctx}");
    assert!(!ctx.contains("développeurs"), "{ctx}");

    let (empty, zero) = build_ask_context(&all, "xylophone", false);
    assert_eq!(zero, 0);
    assert!(empty.is_empty());

    // Sur un seul enregistrement court, toute la transcription est incluse.
    let (single, single_sources) = build_ask_context(&[budget], "résumé global", true);
    assert_eq!(single_sources, 1);
    assert!(single.contains("budget marketing"), "{single}");
}

#[test]
fn select_copy_text_clip_range_and_default_title_work() {
    let mut rec = record();
    assert_eq!(select_copy_text(&rec, "summary"), "");
    assert!(select_copy_text(&rec, "transcript").contains("Bonjour."));
    assert!(select_copy_text(&rec, "all").contains("# Point projet"));

    rec.summary = Some(MeetingSummary {
        text: "Synthèse finale.".into(),
        at_ms: 1,
        model: "test".into(),
        cost: None,
        currency: "€".into(),
    });
    assert_eq!(select_copy_text(&rec, "summary"), "Synthèse finale.");

    let rate = vtt_core::SAMPLE_RATE as usize;
    assert_eq!(clip_sample_range(0), (0, 30 * rate));
    assert_eq!(clip_sample_range(5_000), (3 * rate, 35 * rate));

    let title = default_title("Réunion");
    assert!(title.starts_with("Réunion du "), "{title}");
    assert!(title.contains(" à "), "{title}");
}

#[test]
fn resolve_enroll_name_prefers_input_then_known_name_and_rejects_empty() {
    use super::commands::resolve_enroll_name;
    use vtt_core::voicebook::VoiceRecognition;

    let mut rec = record();
    assert!(resolve_enroll_name(&rec, 0, None).is_err());
    assert!(resolve_enroll_name(&rec, 0, Some("   ")).is_err());
    assert_eq!(
        resolve_enroll_name(&rec, 0, Some("  Marie  ")).unwrap(),
        "Marie"
    );

    rec.speaker_recognized = vec![Some(VoiceRecognition {
        entry_id: "v-1".into(),
        name: "Claire".into(),
        confidence: 91,
    })];
    assert_eq!(resolve_enroll_name(&rec, 0, None).unwrap(), "Claire");
    assert_eq!(
        resolve_enroll_name(&rec, 0, Some("Claire Dupont")).unwrap(),
        "Claire Dupont"
    );
}

#[test]
fn search_records_includes_report_and_transcript_hits() {
    use super::commands::search_records;
    use super::export::report_html;
    use vtt_core::meeting::MeetingReport;

    let mut rec = record();
    rec.segments.push(Segment {
        track: Track::Moi,
        start_ms: 15_000,
        end_ms: 20_000,
        text: "Le budget marketing est validé.".into(),
        copied: false,
        speaker: None,
    });
    let mut report = MeetingReport::from_json(
        &serde_json::json!({
            "synthese": "Validation du budget marketing pour le trimestre.",
            "chapitres": [{"titre": "Budget", "debut": "00:00", "resume": "Décision validée."}],
            "actions": [{"tache": "Finaliser le budget", "responsable": "Julie", "t": "00:15"}]
        }),
        60_000,
    );
    report.finish(60_000);
    rec.report = Some(report);

    let dir = temp_meeting_dir("search-report");
    let hits = search_records(&[rec.clone()], &dir, "budget");
    assert!(
        hits.iter()
            .any(|h| h.source == "report" && h.section.as_deref() == Some("Synthèse"))
    );
    assert!(
        hits.iter()
            .any(|h| h.source == "report" && h.section.as_deref() == Some("Action"))
    );
    assert!(
        hits.iter()
            .any(|h| h.source == "transcript" && h.start_ms == Some(15_000))
    );

    // Rendu HTML interactif (application) et autonome (export).
    let interactive = report_html(&rec, true, true, Some("dark".into())).unwrap();
    assert!(interactive.contains("data-mode=\"app\""));
    assert!(interactive.contains("data-theme=\"dark\""));
    let standalone = report_html(&rec, false, false, None).unwrap();
    assert!(!standalone.contains("data-mode=\"app\""));
    let _ = fs::remove_dir_all(&dir);
}
