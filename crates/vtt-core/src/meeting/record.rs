//! `meeting.json` : fiche d'une réunion (titre, état, transcription, compte
//! rendu, intervenants), au même format sur le PC et sur le téléphone.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::report::MeetingReport;
use super::{NameGuess, Segment, Track, guess_names, same_person, with_speakers, without_echo};
use crate::config::MeetingAudio;
use crate::diarize;
use crate::voicebook::{VoiceBook, VoiceRecognition};

/// Nom de la fiche dans le dossier de la réunion.
pub const RECORD_FILE: &str = "meeting.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    Enregistrement,
    Transcription,
    Terminee,
    /// L'application s'est arrêtée pendant l'enregistrement, sans audio
    /// pour reprendre.
    Interrompue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub text: String,
    pub at_ms: u64,
    pub model: String,
    pub cost: Option<f64>,
    pub currency: String,
}

/// Ce qui a été enregistré.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingKind {
    /// Appel : « Moi » (micro) et « Eux » (son du PC). Sur le téléphone :
    /// réunion au micro.
    #[default]
    Reunion,
    /// Conférence, cours, webinaire : micro, son du PC ou les deux.
    Conference,
    /// Fichier audio ou vidéo importé.
    Fichier,
}

/// Ce que l'utilisateur veut obtenir d'une conférence ou d'un fichier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    #[default]
    LesDeux,
    Transcription,
    /// Compte rendu seul : la transcription est effacée une fois résumée.
    CompteRendu,
}

/// Contenu de `meeting.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub id: String,
    pub title: String,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub status: MeetingStatus,
    pub audio: MeetingAudio,
    /// Le son du PC a été capté (piste « Eux »).
    pub system: bool,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub summary: Option<MeetingSummary>,
    #[serde(default)]
    pub summary_error: Option<String>,
    /// Blocs dont la transcription a échoué.
    #[serde(default)]
    pub failed_chunks: u32,
    /// Remarques (son du PC non capté, audio non chiffrable…).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Écho écarté de la piste « Moi » (appel sur haut-parleurs).
    #[serde(default = "enabled")]
    pub echo_filter: bool,
    /// Blocs du micro écartés comme écho.
    #[serde(default)]
    pub echo_chunks: u32,
    #[serde(default)]
    pub kind: MeetingKind,
    #[serde(default)]
    pub output: Output,
    /// Le micro a été capté (piste « Moi »).
    #[serde(default = "enabled")]
    pub mic: bool,
    /// Fichier importé (nom seulement : il n'est ni copié ni modifié).
    #[serde(default)]
    pub source: Option<String>,
    /// Chemin du fichier importé, pour réécouter un passage (s'il existe
    /// toujours).
    #[serde(default)]
    pub source_path: Option<String>,
    /// Lecture du fichier importé (pourcentage), pendant l'import.
    #[serde(default)]
    pub progress: Option<u8>,
    /// Piste dont les intervenants sont distingués (diarisation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_track: Option<Track>,
    /// Tours de parole des intervenants sur cette piste.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_turns: Vec<diarize::Turn>,
    /// Noms donnés aux intervenants (vide : « Intervenant N »).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_names: Vec<String>,
    /// Noms trouvés quand un intervenant se présente (« je m'appelle
    /// Marie »), utilisés tant que l'utilisateur n'en a pas donné.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_guesses: Vec<Option<NameGuess>>,
    /// Intervenants reconnus automatiquement via le carnet de voix local
    /// (opt-in explicite), utilisés tant que l'utilisateur n'a pas renommé.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_recognized: Vec<Option<VoiceRecognition>>,
    /// Nombre d'intervenants imposé (`None` : déduit des voix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_count: Option<usize>,
    /// Intervenants en cours d'identification.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub identifying: bool,
    /// Moments marqués pendant l'enregistrement (ms depuis le début).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<u64>,
    /// Modèle de compte rendu choisi : conférence (plan, idées clés) ou
    /// réunion (décisions, actions). `None` : d'après le genre (conférence
    /// pour une conférence ou un fichier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference: Option<bool>,
    /// Rapport complet (à la demande) : chapitres, analyse, plan d'action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<MeetingReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_error: Option<String>,
}

fn enabled() -> bool {
    true
}

impl MeetingRecord {
    /// Fiche vide d'un enregistrement qui commence.
    pub fn new(id: String, title: String, started_at_ms: u64, kind: MeetingKind) -> Self {
        Self {
            id,
            title,
            started_at_ms,
            duration_ms: 0,
            status: MeetingStatus::Enregistrement,
            audio: MeetingAudio::Temporaire,
            system: false,
            segments: Vec::new(),
            summary: None,
            summary_error: None,
            failed_chunks: 0,
            notes: Vec::new(),
            echo_filter: false,
            echo_chunks: 0,
            kind,
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

    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(RECORD_FILE);
        let json = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Écrit la fiche (fichier temporaire puis renommage : jamais à moitié
    /// écrite). Une erreur est seulement journalisée.
    pub fn save(&self, dir: &Path) {
        let result = serde_json::to_string_pretty(self)
            .map_err(|e| e.to_string())
            .and_then(|json| {
                let tmp = dir.join("meeting.json.tmp");
                fs::write(&tmp, json)
                    .and_then(|()| fs::rename(&tmp, dir.join(RECORD_FILE)))
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::error!("enregistrement de la réunion {} : {e}", self.id);
        }
    }

    /// Segments affichés : sans l'écho si le filtre est actif, répartis
    /// entre les intervenants s'ils ont été distingués.
    pub fn segments(&self) -> Vec<Segment> {
        let segments = if self.echo_filter {
            without_echo(&self.segments)
        } else {
            self.segments.clone()
        };
        match self.speaker_track {
            Some(track) if !self.speaker_turns.is_empty() => {
                with_speakers(&segments, track, &self.speaker_turns)
            }
            _ => segments,
        }
    }

    /// Compte rendu de conférence plutôt que de réunion.
    pub fn conference_summary(&self) -> bool {
        self.conference.unwrap_or(self.kind != MeetingKind::Reunion)
    }

    /// Un passage peut être réécouté : audio conservé, ou fichier importé
    /// toujours présent.
    pub fn replayable(&self, dir: &Path) -> bool {
        let kept = || {
            self.audio == MeetingAudio::Conserve
                && (dir.join(Track::Moi.file()).is_file() || dir.join(Track::Eux.file()).is_file())
        };
        match self.kind {
            MeetingKind::Fichier => {
                self.source_path
                    .as_deref()
                    .is_some_and(|p| Path::new(p).is_file())
                    || kept()
            }
            _ => kept(),
        }
    }

    /// Piste dont on distingue les intervenants : « Eux » pour un appel
    /// (« Moi », c'est l'utilisateur), la source unique d'une conférence ou
    /// d'un fichier (le son du PC si les deux sont captés).
    pub fn voice_track(&self) -> Option<Track> {
        match self.kind {
            MeetingKind::Reunion => self.system.then_some(Track::Eux),
            MeetingKind::Conference if self.system => Some(Track::Eux),
            MeetingKind::Conference => self.mic.then_some(Track::Moi),
            MeetingKind::Fichier => Some(Track::Eux),
        }
    }

    /// Les empreintes vocales peuvent être enregistrées (chiffrées) : pas
    /// pour une réunion « en mémoire seulement », qui promet de ne rien
    /// écrire de sensible sur le disque (un fichier importé y est déjà).
    pub fn keeps_voices(&self) -> bool {
        self.kind == MeetingKind::Fichier || self.audio != MeetingAudio::Memoire
    }

    /// Tours de parole d'après l'analyse des voix (nombre imposé ou non).
    pub fn set_speakers(&mut self, analysis: &diarize::Analysis) {
        self.set_speakers_with_voicebook(analysis, None);
    }

    /// Tours de parole d'après l'analyse des voix (nombre imposé ou non),
    /// en combinant optionnellement le carnet de voix local (opt-in) et la
    /// détection des présentations (« je m'appelle… »).
    ///
    /// Priorité des noms :
    /// 1. Nom saisi manuellement par l'utilisateur (`speaker_names`)
    /// 2. Voix reconnue dans le carnet (`speaker_recognized`)
    /// 3. Nom détecté dans une présentation (`speaker_guesses`)
    pub fn set_speakers_with_voicebook(
        &mut self,
        analysis: &diarize::Analysis,
        voicebook: Option<&VoiceBook>,
    ) {
        self.speaker_track = self.voice_track();
        let (turns, profiles) = analysis.turns_and_profiles(&diarize::Options {
            speakers: self.speaker_count,
            ..diarize::Options::default()
        });
        self.speaker_turns = turns;
        self.speaker_recognized.clear();
        if let Some(book) = voicebook.filter(|b| !b.is_empty()) {
            for matched in book.match_speakers(&profiles) {
                if self.given_name(matched.speaker).is_some() {
                    continue;
                }
                if self
                    .speaker_names
                    .iter()
                    .any(|n| same_person(n, &matched.name))
                {
                    continue;
                }
                if self.speaker_recognized.len() <= matched.speaker {
                    self.speaker_recognized.resize(matched.speaker + 1, None);
                }
                self.speaker_recognized[matched.speaker] = Some(matched.to_recognition());
            }
        }
        let known_len = self.speaker_names.len().max(self.speaker_recognized.len());
        let known_names: Vec<String> = (0..known_len)
            .map(|i| {
                self.given_name(i)
                    .or_else(|| self.recognized_voice(i).map(|r| r.name.as_str()))
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        self.speaker_guesses = guess_names(&self.segments(), &known_names);
        let count = self.speaker_turns.iter().map(|t| t.speaker + 1).max();
        let recognized = self.speaker_recognized.iter().flatten().count();
        let found = self.speaker_guesses.iter().flatten().count();
        log::info!(
            "{} : {} intervenant(s) distingué(s), {recognized} reconnu(s) par le carnet, {found} nommé(s) d'après leur présentation",
            self.id,
            count.unwrap_or(0)
        );
    }

    /// Nom donné par l'utilisateur à un intervenant (vide : aucun).
    fn given_name(&self, index: usize) -> Option<&str> {
        self.speaker_names
            .get(index)
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
    }

    /// Voix reconnue dans le carnet local pour un intervenant, s'il n'a pas
    /// reçu de nom manuel de l'utilisateur.
    pub fn recognized_voice(&self, index: usize) -> Option<&VoiceRecognition> {
        if self.given_name(index).is_some() {
            return None;
        }
        self.speaker_recognized.get(index)?.as_ref()
    }

    /// Nom trouvé dans la présentation d'un intervenant, s'il n'en a pas
    /// reçu de l'utilisateur ni été reconnu par le carnet de voix.
    pub fn name_guess(&self, index: usize) -> Option<&NameGuess> {
        if self.given_name(index).is_some() || self.recognized_voice(index).is_some() {
            return None;
        }
        self.speaker_guesses.get(index)?.as_ref()
    }

    /// Noms des intervenants (indice = numéro) : celui donné par
    /// l'utilisateur, sinon la voix reconnue dans le carnet, sinon celui
    /// trouvé dans sa présentation, sinon vide (« Intervenant N »).
    pub fn names(&self) -> Vec<String> {
        let count = self
            .speaker_names
            .len()
            .max(self.speaker_recognized.len())
            .max(self.speaker_guesses.len());
        (0..count)
            .map(|i| {
                self.given_name(i)
                    .or_else(|| self.recognized_voice(i).map(|r| r.name.as_str()))
                    .or_else(|| self.name_guess(i).map(|g| g.name.as_str()))
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    /// Nomme un intervenant (vide : « Intervenant N »). Le nom trouvé dans
    /// sa présentation ou par le carnet de voix est oublié : le choix de
    /// l'utilisateur passe devant, même vide ; un autre intervenant ne garde
    /// pas ce nom s'il le lui avait été proposé.
    pub fn rename_speaker(&mut self, index: usize, name: &str) -> Result<(), String> {
        const MAX_NAME: usize = 60;
        const MAX_SPEAKERS: usize = 50;
        if index >= MAX_SPEAKERS {
            return Err("intervenant inconnu".into());
        }
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if self.speaker_names.len() <= index {
            self.speaker_names.resize(index + 1, String::new());
        }
        if let Some(guess) = self.speaker_guesses.get_mut(index) {
            *guess = None;
        }
        for guess in &mut self.speaker_guesses {
            if guess.as_ref().is_some_and(|g| same_person(&g.name, &name)) {
                *guess = None;
            }
        }
        if let Some(rec) = self.speaker_recognized.get_mut(index) {
            *rec = None;
        }
        for rec in &mut self.speaker_recognized {
            if rec.as_ref().is_some_and(|r| same_person(&r.name, &name)) {
                *rec = None;
            }
        }
        while self.speaker_recognized.last().is_some_and(Option::is_none) {
            self.speaker_recognized.pop();
        }
        self.speaker_names[index] = name;
        while self.speaker_names.last().is_some_and(String::is_empty) {
            self.speaker_names.pop();
        }
        Ok(())
    }

    /// Ne plus distinguer les intervenants : tours, noms et nombre oubliés
    /// (l'analyse des voix, à effacer à part, est dans un fichier).
    pub fn forget_speakers(&mut self) {
        self.speaker_track = None;
        self.speaker_turns.clear();
        self.speaker_names.clear();
        self.speaker_guesses.clear();
        self.speaker_recognized.clear();
        self.speaker_count = None;
    }

    /// Fin de la transcription déjà faite, par piste (reprise).
    pub fn transcribed_until(&self, track: Track) -> u64 {
        self.segments
            .iter()
            .filter(|s| s.track == track)
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> MeetingRecord {
        let mut record = MeetingRecord::new(
            "1793790000000".into(),
            "Point projet".into(),
            1_793_790_000_000,
            MeetingKind::Reunion,
        );
        record.system = true;
        record.status = MeetingStatus::Transcription;
        record.segments = vec![
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
        ];
        record
    }

    #[test]
    fn record_round_trips_and_resumes_after_each_track() {
        let dir = std::env::temp_dir().join(format!("vtt-record-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut original = record();
        original.marks = vec![4_000];
        original.save(&dir);
        let loaded = MeetingRecord::load(&dir).unwrap();
        assert_eq!(loaded.segments, record().segments);
        assert_eq!(loaded.status, MeetingStatus::Transcription);
        assert_eq!(loaded.marks, [4_000]);
        assert_eq!(loaded.transcribed_until(Track::Moi), 7_000);
        assert_eq!(loaded.transcribed_until(Track::Eux), 9_500);
        // Champs facultatifs absents du fichier tant qu'ils sont vides
        // (fichiers relus par une version précédente).
        record().save(&dir);
        let json = fs::read_to_string(dir.join(RECORD_FILE)).unwrap();
        assert!(!json.contains("speaker") && !json.contains("marks"));
        assert!(!json.contains("conference"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn records_of_an_older_version_are_read() {
        let json = r#"{"id":"1","title":"Appel","started_at_ms":1,"duration_ms":2,
            "status":"terminee","audio":"temporaire","system":true,"segments":[]}"#;
        let record: MeetingRecord = serde_json::from_str(json).unwrap();
        assert!(record.echo_filter && record.mic && record.marks.is_empty());
        assert_eq!(record.kind, MeetingKind::Reunion);
        assert!(!record.conference_summary());
    }

    #[test]
    fn summary_kind_follows_the_choice_or_the_kind() {
        let mut record = record();
        assert!(!record.conference_summary());
        record.kind = MeetingKind::Fichier;
        assert!(record.conference_summary(), "fichier : conférence");
        record.conference = Some(false);
        assert!(!record.conference_summary(), "réunion choisie");
    }

    #[test]
    fn introduced_speakers_are_named_until_the_user_says_otherwise() {
        let mut record = record();
        record.speaker_track = Some(Track::Eux);
        record.segments.push(Segment {
            track: Track::Eux,
            start_ms: 10_000,
            end_ms: 14_000,
            text: "Moi c'est Paul.".into(),
            copied: false,
            speaker: None,
        });
        record.speaker_turns = vec![
            diarize::Turn {
                start_ms: 2_000,
                end_ms: 9_500,
                speaker: 0,
            },
            diarize::Turn {
                start_ms: 10_000,
                end_ms: 14_000,
                speaker: 1,
            },
        ];
        record.speaker_guesses = guess_names(&record.segments(), &record.speaker_names);
        assert_eq!(record.names(), ["", "Paul"]);
        assert_eq!(record.name_guess(1).unwrap().quote, "Moi c'est Paul.");
        // Gardé dans la fiche.
        let json = serde_json::to_string(&record).unwrap();
        let read: MeetingRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(read.speaker_guesses, record.speaker_guesses);
        // L'utilisateur donne « Paul » à l'autre voix : la proposition tombe.
        record.rename_speaker(0, " Paul ").unwrap();
        assert_eq!(record.names(), ["Paul", ""]);
        assert!(record.name_guess(1).is_none());
        // Effacer le nom revient à « Intervenant N », sans la proposition.
        record.rename_speaker(0, "").unwrap();
        assert!(record.names().iter().all(String::is_empty));
        assert!(record.speaker_names.is_empty());
        assert!(record.rename_speaker(50, "X").is_err());
        record.forget_speakers();
        assert!(record.speaker_guesses.is_empty() && record.speaker_track.is_none());
    }

    #[test]
    fn voices_come_from_the_other_side_or_the_only_source() {
        let mut record = record();
        assert_eq!(record.voice_track(), Some(Track::Eux));
        record.system = false;
        assert_eq!(record.voice_track(), None, "appel sans le son du PC");
        record.kind = MeetingKind::Conference;
        assert_eq!(
            record.voice_track(),
            Some(Track::Moi),
            "conférence au micro"
        );
        record.system = true;
        assert_eq!(record.voice_track(), Some(Track::Eux));
        record.kind = MeetingKind::Fichier;
        assert_eq!(record.voice_track(), Some(Track::Eux));
    }

    struct FakeDiarize;

    impl diarize::Models for FakeDiarize {
        fn segment(
            &mut self,
            window: &[f32],
        ) -> Result<Vec<[f32; diarize::CLASSES]>, crate::Error> {
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

        fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, crate::Error> {
            let v = (samples[samples.len() / 2] * 10.0).round() as i32;
            let mut e = vec![0.05f32; 3];
            if (1..=3).contains(&v) {
                e[(v - 1) as usize] = 1.0;
            }
            Ok(e)
        }
    }

    #[test]
    fn voicebook_combines_with_name_guesses_and_respects_manual_overrides() {
        const SECOND: usize = crate::SAMPLE_RATE as usize;
        // Deux voix sur la piste « Eux » : voix 1 (0..8s) connue dans le carnet
        // (« Marie »), voix 2 (8..16s) inconnue qui se présente (« Moi c'est Paul. »).
        let mut audio = vec![0.1f32; 8 * SECOND];
        audio.extend(std::iter::repeat_n(0.2f32, 8 * SECOND));
        let mut analysis = diarize::Analysis::new(diarize::Options::default().step);
        analysis.add(0, &audio, &mut FakeDiarize).unwrap();
        analysis.finish(&mut FakeDiarize).unwrap();

        let mut book = VoiceBook::default();
        let marie = book.enroll("Marie", &[1.0, 0.05, 0.05], 4, 1_000).unwrap();

        let mut record = MeetingRecord::new("42".into(), "Point".into(), 100, MeetingKind::Reunion);
        record.system = true;
        record.segments = vec![
            Segment {
                track: Track::Eux,
                start_ms: 500,
                end_ms: 7_500,
                text: "Bonjour tout le monde, on commence.".into(),
                copied: false,
                speaker: None,
            },
            Segment {
                track: Track::Eux,
                start_ms: 8_500,
                end_ms: 15_500,
                text: "Moi c'est Paul, ravi de vous retrouver.".into(),
                copied: false,
                speaker: None,
            },
        ];

        record.set_speakers_with_voicebook(&analysis, Some(&book));
        assert_eq!(record.names(), ["Marie", "Paul"]);
        let rec0 = record.recognized_voice(0).expect("voix 0 reconnue");
        assert_eq!(rec0.entry_id, marie.id);
        assert_eq!(rec0.name, "Marie");
        assert!(rec0.confidence >= 95);
        assert!(record.name_guess(0).is_none());
        assert!(record.recognized_voice(1).is_none());
        assert_eq!(record.name_guess(1).unwrap().name, "Paul");

        // Sérialisation JSON aller-retour.
        let json = serde_json::to_string(&record).unwrap();
        let reloaded: MeetingRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded.speaker_recognized, record.speaker_recognized);

        // Si l'utilisateur attribue « Marie » à l'autre intervenant, la
        // reconnaissance automatique de l'intervenant 0 est effacée.
        record.rename_speaker(1, "Marie").unwrap();
        assert_eq!(record.names(), ["", "Marie"]);
        assert!(record.recognized_voice(0).is_none());
        assert!(record.speaker_recognized.is_empty());

        // Nouvelle application avec nom manuel déjà posé sur 1 : ne réattribue
        // pas « Marie » à 0.
        record.set_speakers_with_voicebook(&analysis, Some(&book));
        assert!(record.recognized_voice(0).is_none());
        assert_eq!(record.names(), ["", "Marie"]);

        // Effacer le nom manuel puis réappliquer : Marie revient, et renommer 0
        // en vide l'oublie sans bloquer 1.
        record.rename_speaker(1, "").unwrap();
        record.set_speakers_with_voicebook(&analysis, Some(&book));
        assert_eq!(record.names(), ["Marie", "Paul"]);
        record.rename_speaker(0, "").unwrap();
        assert!(record.recognized_voice(0).is_none());
        assert_eq!(record.names(), ["", "Paul"]);

        // Oublier les intervenants efface aussi speaker_recognized.
        record.set_speakers_with_voicebook(&analysis, Some(&book));
        assert!(!record.speaker_recognized.is_empty());
        record.forget_speakers();
        assert!(record.speaker_recognized.is_empty());
    }
}
