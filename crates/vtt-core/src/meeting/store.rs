//! Logique commune aux réunions (PC et Android) : résolution sécurisée des
//! dossiers, conservation, vues d'intervenants, sauvegarde atomique de
//! l'analyse vocale (`voix.bin`) et export texte/Markdown/HTML.
//!
//! Aucune dépendance OS : le chiffrement éventuel (DPAPI/trousseau sur PC,
//! stockage privé chiffré par Android) est passé en fermeture, comme pour
//! l'audio.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{
    HtmlOptions, MeetingRecord, RECORD_FILE, Reply, speaker_label, to_markdown, transcript_text,
};
use crate::diarize;
use crate::voicebook::VoiceRecognition;

/// Un jour, en millisecondes (durée de conservation des réunions).
pub const DAY_MS: u64 = 24 * 3_600 * 1_000;

/// Fichier de l'analyse vocale d'une réunion (`diarize::Analysis`, chiffré sur PC).
pub const ANALYSIS_FILE: &str = "voix.bin";

/// Nombre maximal d'intervenants pouvant être imposés manuellement.
pub const MAX_SPEAKER_COUNT: usize = 20;

/// Longueur par défaut de l'extrait de la première réplique d'un intervenant.
pub const DEFAULT_EXCERPT_CHARS: usize = 90;

/// Identifiant de réunion valide : chiffres ASCII uniquement (horodatage en ms),
/// interdisant toute traversée de chemin (`..`, `/`, `\`).
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

/// Résout le dossier d'une réunion existante sous `root` (refuse tout
/// identifiant invalide et tout dossier sans `meeting.json`).
pub fn meeting_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_valid_id(id) {
        return Err("réunion inconnue".into());
    }
    let dir = root.join(id);
    if dir.join(RECORD_FILE).is_file() {
        Ok(dir)
    } else {
        Err("réunion introuvable".into())
    }
}

/// Charge une fiche de réunion et son dossier depuis `root`.
pub fn load_meeting(root: &Path, id: &str) -> Result<(MeetingRecord, PathBuf), String> {
    let dir = meeting_dir(root, id)?;
    Ok((MeetingRecord::load(&dir)?, dir))
}

/// Nettoie et valide un titre de réunion.
pub fn validate_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("titre vide".into());
    }
    Ok(trimmed.to_owned())
}

/// Vrai si une réunion commencée à `started_at_ms` a dépassé la durée de
/// conservation `keep_days` (0 : illimitée).
pub fn is_expired(started_at_ms: u64, now_ms: u64, keep_days: u32) -> bool {
    keep_days > 0 && now_ms.saturating_sub(started_at_ms) >= u64::from(keep_days) * DAY_MS
}

/// Valide un nombre d'intervenants imposé (`None` : automatique, sinon 1..=20).
pub fn validate_speaker_count(count: Option<usize>) -> Result<(), String> {
    if count.is_some_and(|c| !(1..=MAX_SPEAKER_COUNT).contains(&c)) {
        return Err(format!(
            "nombre d'intervenants : de 1 à {MAX_SPEAKER_COUNT}"
        ));
    }
    Ok(())
}

/// Nombre d'intervenants distincts présents dans `record.speaker_turns`.
pub fn speaker_count(record: &MeetingRecord) -> usize {
    record
        .speaker_turns
        .iter()
        .map(|t| t.speaker + 1)
        .max()
        .unwrap_or(0)
}

/// Début d'un texte, coupé proprement sur au plus `max` caractères Unicode.
pub fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// Synthèse d'un intervenant distingué (commune aux vues PC et Android).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpeakerSummary {
    pub index: usize,
    /// « Intervenant N ».
    pub label: String,
    pub name: String,
    pub talk_ms: u64,
    /// Début de sa première réplique, pour reconnaître la voix.
    pub first: String,
    pub first_ms: u64,
    /// Si `name` vient de sa présentation : la phrase et son horodatage.
    pub found: Option<String>,
    pub found_ms: Option<u64>,
    /// Si `name` vient du carnet de voix local (opt-in) : entrée et confiance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized: Option<VoiceRecognition>,
}

/// Construit la liste triée des intervenants distingués à partir de la fiche
/// et de ses répliques regroupées.
pub fn speaker_summaries(
    record: &MeetingRecord,
    replies: &[Reply],
    max_excerpt: usize,
) -> Vec<SpeakerSummary> {
    let names = record.names();
    let mut speakers: Vec<SpeakerSummary> = Vec::new();
    for reply in replies {
        if let Some(index) = reply.speaker
            && !speakers.iter().any(|s| s.index == index)
        {
            let guess = record.name_guess(index);
            let recognized = record.recognized_voice(index).cloned();
            speakers.push(SpeakerSummary {
                index,
                label: speaker_label(reply.track, Some(index), &[]),
                name: names.get(index).cloned().unwrap_or_default(),
                talk_ms: record
                    .speaker_turns
                    .iter()
                    .filter(|t| t.speaker == index)
                    .map(|t| t.end_ms.saturating_sub(t.start_ms))
                    .sum(),
                first: excerpt(&reply.text, max_excerpt),
                first_ms: reply.start_ms,
                found: guess.map(|g| g.quote.clone()),
                found_ms: guess.map(|g| g.at_ms),
                recognized,
            });
        }
    }
    speakers.sort_by_key(|s| s.index);
    speakers
}

/// Enregistre l'analyse vocale (`voix.bin`) de manière atomique via un fichier
/// temporaire, en passant ses octets par `protect` (chiffrement sur PC,
/// identité sur stockage privé Android).
pub fn save_analysis(
    dir: &Path,
    analysis: &diarize::Analysis,
    protect: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let data = protect(&analysis.to_bytes())?;
    let tmp = dir.join("voix.bin.tmp");
    fs::write(&tmp, data)
        .and_then(|()| fs::rename(&tmp, dir.join(ANALYSIS_FILE)))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Relit l'analyse vocale (`voix.bin`) en passant par `unprotect`.
pub fn load_analysis(
    dir: &Path,
    unprotect: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<diarize::Analysis, String> {
    let path = dir.join(ANALYSIS_FILE);
    let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    diarize::Analysis::from_bytes(&unprotect(&data)?)
}

/// Vrai si `voix.bin` est conservé dans le dossier de la réunion.
pub fn has_analysis(dir: &Path) -> bool {
    dir.join(ANALYSIS_FILE).is_file()
}

/// Supprime `voix.bin` s'il existe.
pub fn delete_analysis(dir: &Path) {
    let _ = fs::remove_file(dir.join(ANALYSIS_FILE));
}

/// Nom de fichier sûr d'après le titre d'une réunion et une extension.
pub fn safe_file_name(title: &str, extension: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let stem = safe.trim().trim_matches('.');
    let stem = if stem.is_empty() { "reunion" } else { stem };
    format!("{stem}.{extension}")
}

/// Options de mise en forme d'un export texte de réunion.
#[derive(Debug, Clone, Copy)]
pub struct ExportOptions {
    /// Inclure les noms de pistes (« Moi » / « Eux ») lorsque les intervenants
    /// ne sont pas distingués (`true` sur PC, `false` sur Android mono-piste).
    pub include_track_names: bool,
    /// Ajouter `# <titre>` au-dessus d'un export « summary » (`true` sur Android).
    pub summary_heading: bool,
    /// Renvoyer une chaîne vide plutôt qu'une erreur si « summary » est absent
    /// (`true` pour la copie rapide PC, `false` pour le partage Android).
    pub allow_empty_summary: bool,
}

/// Exporte une réunion selon `what` : `"markdown"` (défaut), `"summary"`,
/// `"transcript"`, `"report"` ou `"report_html"`.
pub fn export_meeting_text(
    record: &MeetingRecord,
    what: &str,
    date: &str,
    options: ExportOptions,
) -> Result<String, String> {
    let segments = record.segments();
    let names = record.names();
    match what {
        "report" => record
            .report_markdown(date, options.include_track_names)
            .ok_or_else(|| "pas encore de rapport".to_owned()),
        "report_html" => record
            .report_html(date, &HtmlOptions::default())
            .ok_or_else(|| "pas encore de rapport".to_owned()),
        "summary" => match &record.summary {
            Some(summary) if options.summary_heading => {
                Ok(format!("# {}\n\n{}", record.title, summary.text.trim()))
            }
            Some(summary) => Ok(summary.text.clone()),
            None if options.allow_empty_summary => Ok(String::new()),
            None => Err("pas encore de compte rendu".into()),
        },
        "transcript" => Ok(transcript_text(
            &segments,
            &names,
            options.include_track_names,
        )),
        _ => Ok(to_markdown(
            &record.title,
            date,
            record.duration_ms,
            record.summary.as_ref().map(|s| s.text.as_str()),
            &segments,
            &names,
            options.include_track_names,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::meeting::{MeetingKind, MeetingSummary, NameGuess, Segment, Track};

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-meeting-store-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validates_ids_and_prevents_path_traversal() {
        let root = temp_dir("ids");
        assert!(is_valid_id("1793790000000"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("../secret"));
        assert!(!is_valid_id("123/456"));
        assert!(!is_valid_id("abc"));

        assert!(meeting_dir(&root, "../etc").is_err());
        assert!(meeting_dir(&root, "12345").is_err());

        let dir = root.join("12345");
        fs::create_dir_all(&dir).unwrap();
        let rec = MeetingRecord::new("12345".into(), "Point".into(), 12_345, MeetingKind::Reunion);
        rec.save(&dir);

        let (loaded, resolved) = load_meeting(&root, "12345").unwrap();
        assert_eq!(resolved, dir);
        assert_eq!(loaded.title, "Point");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validates_title_expiry_and_speaker_count() {
        assert_eq!(validate_title("  Point projet  ").unwrap(), "Point projet");
        assert!(validate_title("   ").is_err());

        assert!(!is_expired(0, 100 * DAY_MS, 0));
        assert!(!is_expired(1000, 1000 + 29 * DAY_MS, 30));
        assert!(is_expired(1000, 1000 + 30 * DAY_MS, 30));

        assert!(validate_speaker_count(None).is_ok());
        assert!(validate_speaker_count(Some(1)).is_ok());
        assert!(validate_speaker_count(Some(MAX_SPEAKER_COUNT)).is_ok());
        assert!(validate_speaker_count(Some(0)).is_err());
        assert!(validate_speaker_count(Some(MAX_SPEAKER_COUNT + 1)).is_err());
    }

    #[test]
    fn builds_sorted_speaker_summaries_with_utf8_excerpts() {
        assert_eq!(excerpt("Bonjour", 10), "Bonjour");
        assert_eq!(excerpt("Été à Paris ", 6), "Été à…");

        let mut rec = MeetingRecord::new("1".into(), "Point".into(), 1, MeetingKind::Reunion);
        rec.speaker_turns = vec![
            diarize::Turn {
                start_ms: 0,
                end_ms: 3_000,
                speaker: 1,
            },
            diarize::Turn {
                start_ms: 3_000,
                end_ms: 5_500,
                speaker: 0,
            },
            diarize::Turn {
                start_ms: 6_000,
                end_ms: 8_000,
                speaker: 1,
            },
        ];
        rec.speaker_names = vec!["Alice".into()];
        rec.speaker_guesses = vec![
            None,
            Some(NameGuess {
                name: "Paul".into(),
                quote: "Moi c'est Paul.".into(),
                at_ms: 500,
            }),
        ];
        assert_eq!(speaker_count(&rec), 2);

        let replies = vec![
            Reply {
                track: Track::Eux,
                speaker: Some(1),
                start_ms: 500,
                text: "Moi c'est Paul, ravi de vous retrouver.".into(),
            },
            Reply {
                track: Track::Eux,
                speaker: Some(0),
                start_ms: 3_100,
                text: "Bonjour à tous.".into(),
            },
            Reply {
                track: Track::Eux,
                speaker: Some(1),
                start_ms: 6_200,
                text: "Deuxième réplique ignorée pour l'extrait.".into(),
            },
        ];

        let summaries = speaker_summaries(&rec, &replies, 15);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].index, 0);
        assert_eq!(summaries[0].label, "Intervenant 1");
        assert_eq!(summaries[0].name, "Alice");
        assert_eq!(summaries[0].talk_ms, 2_500);
        assert_eq!(summaries[0].first, "Bonjour à tous.");
        assert_eq!(summaries[0].first_ms, 3_100);
        assert_eq!(summaries[0].found, None);
        assert_eq!(summaries[0].recognized, None);

        assert_eq!(summaries[1].index, 1);
        assert_eq!(summaries[1].label, "Intervenant 2");
        assert_eq!(summaries[1].name, "Paul");
        assert_eq!(summaries[1].talk_ms, 5_000);
        assert_eq!(summaries[1].first, "Moi c'est Paul,…");
        assert_eq!(summaries[1].first_ms, 500);
        assert_eq!(summaries[1].found.as_deref(), Some("Moi c'est Paul."));
        assert_eq!(summaries[1].found_ms, Some(500));
        assert_eq!(summaries[1].recognized, None);
        let json = serde_json::to_string(&summaries[0]).unwrap();
        assert!(!json.contains("recognized"));

        // Voix reconnue par le carnet : affichée tant qu'il n'y a pas de nom manuel.
        rec.speaker_names.clear();
        rec.speaker_recognized = vec![Some(VoiceRecognition {
            entry_id: "v-1".into(),
            name: "Claire".into(),
            confidence: 92,
        })];
        let summaries = speaker_summaries(&rec, &replies, 15);
        assert_eq!(summaries[0].name, "Claire");
        assert_eq!(
            summaries[0].recognized.as_ref().map(|r| r.confidence),
            Some(92)
        );
    }

    #[test]
    fn analysis_roundtrips_through_protect_and_deletes_cleanly() {
        let dir = temp_dir("analysis");
        let xor = |data: &[u8]| Ok(data.iter().map(|b| b ^ 0x33).collect());
        let analysis = diarize::Analysis::new(16_000);

        assert!(!has_analysis(&dir));
        save_analysis(&dir, &analysis, xor).unwrap();
        assert!(has_analysis(&dir));
        let loaded = load_analysis(&dir, xor).unwrap();
        assert_eq!(loaded.to_bytes(), analysis.to_bytes());
        delete_analysis(&dir);
        assert!(!has_analysis(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_file_name_and_export_text_cover_desktop_and_android_modes() {
        assert_eq!(
            safe_file_name("Appel: Q4 / Budget?", "md"),
            "Appel- Q4 - Budget-.md"
        );
        assert_eq!(safe_file_name("...", "wav"), "reunion.wav");

        let mut rec = MeetingRecord::new("1".into(), "Point".into(), 1, MeetingKind::Reunion);
        rec.segments.push(Segment {
            track: Track::Moi,
            start_ms: 0,
            end_ms: 2_000,
            text: "Bonjour.".into(),
            copied: false,
            speaker: None,
        });

        let pc_opts = ExportOptions {
            include_track_names: true,
            summary_heading: false,
            allow_empty_summary: true,
        };
        let android_opts = ExportOptions {
            include_track_names: false,
            summary_heading: true,
            allow_empty_summary: false,
        };

        assert_eq!(
            export_meeting_text(&rec, "summary", "19/11", pc_opts).unwrap(),
            ""
        );
        assert!(export_meeting_text(&rec, "summary", "19/11", android_opts).is_err());

        rec.summary = Some(MeetingSummary {
            text: "Décision validée.".into(),
            at_ms: 2,
            model: "test".into(),
            cost: None,
            currency: "€".into(),
        });
        assert_eq!(
            export_meeting_text(&rec, "summary", "19/11", pc_opts).unwrap(),
            "Décision validée."
        );
        assert_eq!(
            export_meeting_text(&rec, "summary", "19/11", android_opts).unwrap(),
            "# Point\n\nDécision validée."
        );
        assert!(
            export_meeting_text(&rec, "transcript", "19/11", pc_opts)
                .unwrap()
                .contains("Moi")
        );
        assert!(
            !export_meeting_text(&rec, "transcript", "19/11", android_opts)
                .unwrap()
                .contains("Moi")
        );
        assert!(export_meeting_text(&rec, "report", "19/11", android_opts).is_err());
    }
}
