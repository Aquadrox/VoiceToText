use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use vtt_core::SAMPLE_RATE;
use vtt_core::config::MeetingAudio;
use vtt_core::diarize;
use vtt_core::meeting::{self, MeetingKind, MeetingStatus, MeetingSummary, Segment, Track};

use super::storage::{
    Resampler, audio_samples, dir, live, load, lock, memory, save_analysis, step, update,
    voicebook_path,
};
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

/// Serveur LLM jetable (adresse locale : pas de clé) qui répond
/// `reply` à chaque requête, `count` fois.
fn fake_llm(reply: &'static str, count: usize) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length: usize = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = serde_json::json!({
                "choices": [{"message": {"content": reply}}],
                "usage": {"prompt_tokens": 1000, "completion_tokens": 100}
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    url
}

#[test]
fn report_is_written_on_demand_checked_and_exported() {
    let _serial = lock(&SERIAL);
    let root = root("report");
    let id = create(&root, &new(MeetingAudio::Conserve, None)).unwrap();
    stop(&root, &id).unwrap();
    update(&root, &id, |r| {
        r.status = MeetingStatus::Terminee;
        r.title = "Point projet".into();
        r.duration_ms = 120_000;
        r.segments.push(Segment {
            track: Track::Moi,
            start_ms: 12_000,
            end_ms: 15_000,
            text: "Paul envoie le devis vendredi.".into(),
            copied: false,
            speaker: None,
        });
    })
    .unwrap();
    assert!(
        set_action_done(&root, &id, 0, true).is_err(),
        "pas de rapport"
    );
    let reply = r#"{"synthese": "Devis à envoyer.",
        "chapitres": [{"titre": "Devis", "debut": "00:10"}],
        "actions": [{"tache": "Envoyer le devis", "responsable": "Paul", "echeance": "vendredi", "t": "00:12"}]}"#;
    let url = fake_llm(reply, 2);
    let toml = format!(
        "[llm]\nenabled = true\nbase_url = \"{url}\"\nmodel = \"test\"\n[llm.pricing]\ninput_per_million = 1.0\noutput_per_million = 2.0\n"
    );
    report(&root, &id, &toml, "").unwrap();
    let view = detail(&root, &id).unwrap();
    assert!(view.row.report && view.row.step.is_none());
    let summary = view.report_view.unwrap();
    assert_eq!(summary.synthesis, "Devis à envoyer.");
    assert_eq!((summary.actions, summary.actions_done), (1, 0));
    assert_eq!(summary.chapters, ["Devis"]);
    assert_eq!(summary.model, "test");
    assert!((summary.cost.unwrap() - 0.0012).abs() < 1e-9);

    set_action_done(&root, &id, 0, true).unwrap();
    assert!(set_action_done(&root, &id, 3, true).is_err());
    let html = report_html(&root, &id, "18/11/2026", "dark").unwrap();
    assert!(html.contains("data-theme=\"dark\" data-mode=\"app\""));
    assert!(html.contains("data-index=\"0\" checked"));
    assert!(html.contains("Paul envoie le devis vendredi."));
    // Fichier exporté : sans lien avec l'application.
    let file = export(&root, &id, "report_html", "18/11/2026").unwrap();
    assert!(file.contains("data-mode=\"file\"") && !file.contains("class=\"play\""));
    let md = export(&root, &id, "report", "18/11/2026").unwrap();
    assert!(md.contains("- [x] Envoyer le devis — Paul, échéance : vendredi"));

    // Nouvelle rédaction : l'action cochée le reste.
    report(&root, &id, &toml, "").unwrap();
    assert_eq!(
        detail(&root, &id)
            .unwrap()
            .report_view
            .unwrap()
            .actions_done,
        1
    );
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
    assert_eq!(meeting::excerpt("Bonjour", 90), "Bonjour");
    assert_eq!(meeting::excerpt("Ça va très bien", 6), "Ça va…");
}

struct FakeDiarize;

impl diarize::Models for FakeDiarize {
    fn segment(&mut self, window: &[f32]) -> Result<Vec<[f32; diarize::CLASSES]>, vtt_core::Error> {
        let frames = (diarize::WINDOW - diarize::FRAME_SIZE) / diarize::FRAME_STEP + 1;
        let mut local: Vec<i32> = Vec::new();
        Ok((0..frames)
            .map(|f| {
                let mid = f * diarize::FRAME_STEP + diarize::FRAME_SIZE / 2;
                let v = (window[mid] * 10.0).round() as i32;
                let mut row = [0.0; diarize::CLASSES];
                let class = if v <= 0 {
                    0
                } else {
                    let slot = local.iter().position(|&x| x == v).unwrap_or_else(|| {
                        local.push(v);
                        local.len() - 1
                    });
                    1 + slot.min(2)
                };
                row[class] = 1.0;
                row
            })
            .collect())
    }

    fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, vtt_core::Error> {
        let v = (samples[samples.len() / 2] * 10.0).round() as i32;
        let mut e = vec![0.05f32; 3];
        if (1..=3).contains(&v) {
            e[(v - 1) as usize] = 1.0;
        }
        Ok(e)
    }
}

#[test]
fn voicebook_enrolls_recognizes_across_meetings_renames_and_removes() {
    let _serial = lock(&SERIAL);
    let root = root("voicebook");

    // Prépare une analyse vocale à deux voix (0.1 puis 0.2).
    const SECOND: usize = vtt_core::SAMPLE_RATE as usize;
    let mut audio = vec![0.1f32; 8 * SECOND];
    audio.extend(std::iter::repeat_n(0.2f32, 8 * SECOND));
    let mut analysis = diarize::Analysis::new(diarize::Options::default().step);
    analysis.add(0, &audio, &mut FakeDiarize).unwrap();
    analysis.finish(&mut FakeDiarize).unwrap();

    // Réunion 1 : on garde voix.bin et on mémorise la voix 0 sous « Marie ».
    let id1 = create(&root, &new(MeetingAudio::Conserve, None)).unwrap();
    stop(&root, &id1).unwrap();
    update(&root, &id1, |r| {
        r.status = MeetingStatus::Terminee;
        r.segments = vec![
            Segment {
                track: Track::Moi,
                start_ms: 500,
                end_ms: 7_500,
                text: "Bonjour à tous.".into(),
                copied: false,
                speaker: None,
            },
            Segment {
                track: Track::Moi,
                start_ms: 8_500,
                end_ms: 15_500,
                text: "Salut.".into(),
                copied: false,
                speaker: None,
            },
        ];
        r.set_speakers(&analysis);
    })
    .unwrap();
    save_analysis(&root.join(&id1), &analysis).unwrap();

    assert!(voicebook_list(&root).unwrap().is_empty());
    assert!(enroll_speaker_voice(&root, &id1, 0, "  ").is_err());

    let marie = enroll_speaker_voice(&root, &id1, 0, "Marie").unwrap();
    assert_eq!(marie.name, "Marie");
    assert_eq!(marie.meetings, 1);
    assert_eq!(voicebook_list(&root).unwrap(), vec![marie.clone()]);
    assert_eq!(detail(&root, &id1).unwrap().speaker_list[0].name, "Marie");

    // `voicebook.bin` dans `root` ne perturbe jamais la liste des réunions.
    assert_eq!(list(&root, "").unwrap().len(), 1);

    // Réunion 2 avec la même voix 0 : reconnue automatiquement par set_speaker_count.
    let id2 = create(&root, &new(MeetingAudio::Conserve, None)).unwrap();
    stop(&root, &id2).unwrap();
    update(&root, &id2, |r| {
        r.status = MeetingStatus::Terminee;
        r.segments = vec![Segment {
            track: Track::Moi,
            start_ms: 500,
            end_ms: 7_500,
            text: "On fait le point.".into(),
            copied: false,
            speaker: None,
        }];
    })
    .unwrap();
    save_analysis(&root.join(&id2), &analysis).unwrap();
    set_speaker_count(&root, &id2, None).unwrap();

    let view2 = detail(&root, &id2).unwrap();
    assert_eq!(view2.speaker_list[0].name, "Marie");
    let rec = view2.speaker_list[0]
        .recognized
        .as_ref()
        .expect("voix reconnue automatiquement");
    assert_eq!(rec.entry_id, marie.id);
    assert!(rec.confidence >= 95);

    // Renommage, oubli en un clic et effacement complet.
    rename_voicebook_entry(&root, &marie.id, "Marie Dupont").unwrap();
    assert_eq!(voicebook_list(&root).unwrap()[0].name, "Marie Dupont");

    assert!(remove_voicebook_entry(&root, &marie.id).unwrap());
    assert!(!remove_voicebook_entry(&root, &marie.id).unwrap());
    assert!(voicebook_list(&root).unwrap().is_empty());
    assert!(!voicebook_path(&root).exists());

    enroll_speaker_voice(&root, &id1, 0, "Marie").unwrap();
    assert!(voicebook_path(&root).exists());
    clear_voicebook(&root);
    assert!(!voicebook_path(&root).exists());
    assert!(voicebook_list(&root).unwrap().is_empty());

    fs::remove_dir_all(&root).ok();
}
