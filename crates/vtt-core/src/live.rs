//! Dictée longue en continu (mode mains libres) : découpage incrémental de
//! l'audio aux pauses, assemblage des tranches transcrites et aperçu glissant.
//!
//! La session ne dépend ni de l'OS ni de la capture micro : PC (`apps/desktop`)
//! et Android (`vtt-android`) lui envoient les échantillons 16 kHz mono au fil
//! de l'eau, transcrivent chaque bloc prêt via [`crate::Pipeline::transcribe_chunk`],
//! affichent [`LiveSession::preview`], puis lancent un unique nettoyage LLM et
//! une unique insertion à l'arrêt via [`crate::Pipeline::finalize_raw`].

use crate::audio;
use crate::meeting::{Chunk, Chunker};
use crate::pipeline::{AudioClip, RawDictation, SAMPLE_RATE, SILENCE_RMS};
use crate::text;

/// Durée minimale d'une tranche en dictée longue : assez courte pour donner un
/// retour visuel rapide après une phrase, assez longue pour garder le contexte
/// acoustique de Whisper / Parakeet / Vosk.
pub const LIVE_MIN_CHUNK_MS: u64 = 3_500;

/// Durée maximale avant coupure forcée au moment le plus calme si l'utilisateur
/// parle en continu sans pause.
pub const LIVE_MAX_CHUNK_MS: u64 = 9_000;

/// Longueur d'aperçu par défaut (en caractères) pour la pastille PC et la barre
/// d'état du clavier Android.
pub const DEFAULT_PREVIEW_CHARS: usize = 90;

/// Session de prévisualisation en continu pour une dictée longue.
#[derive(Debug)]
pub struct LiveSession {
    chunker: Chunker,
    segments: Vec<String>,
    total_samples: u64,
    emitted_chunks: usize,
    asr_ms: u64,
}

impl Default for LiveSession {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveSession {
    pub fn new() -> Self {
        Self::with_bounds(LIVE_MIN_CHUNK_MS, LIVE_MAX_CHUNK_MS)
    }

    /// Session avec bornes personnalisées (tests ou réglages spécifiques).
    pub fn with_bounds(min_chunk_ms: u64, max_chunk_ms: u64) -> Self {
        Self {
            chunker: Chunker::with_bounds(min_chunk_ms, max_chunk_ms),
            segments: Vec::new(),
            total_samples: 0,
            emitted_chunks: 0,
            asr_ms: 0,
        }
    }

    /// Ajoute des échantillons 16 kHz mono et renvoie les tranches non
    /// silencieuses prêtes à être transcrites.
    ///
    /// Utilise le seuil de silence de la dictée (plus sensible que celui des
    /// réunions) pour ne pas écarter les micros peu sensibles.
    pub fn feed(&mut self, samples: &[f32]) -> Vec<AudioClip> {
        self.total_samples = self.total_samples.saturating_add(samples.len() as u64);
        let chunks = self.chunker.feed(samples);
        self.emitted_chunks += chunks.len();
        chunks
            .into_iter()
            .filter(|chunk| !is_silent_chunk(chunk))
            .map(|chunk| AudioClip {
                samples: chunk.samples,
            })
            .collect()
    }

    /// Termine le flux et renvoie l'éventuelle dernière tranche non silencieuse
    /// (au moins 300 ms).
    pub fn finish(&mut self) -> Option<AudioClip> {
        let chunk = self.chunker.finish()?;
        self.emitted_chunks += 1;
        if is_silent_chunk(&chunk) {
            return None;
        }
        Some(AudioClip {
            samples: chunk.samples,
        })
    }

    /// Enregistre le texte brut d'une tranche transcrite et sa durée ASR.
    /// Renvoie `true` si l'aperçu a changé (texte non vide après filtrage des
    /// hallucinations).
    pub fn push_text(&mut self, chunk_text: &str, asr_ms: u64) -> bool {
        self.asr_ms = self.asr_ms.saturating_add(asr_ms);
        let cleaned = text::strip_hallucinations(chunk_text);
        if cleaned.is_empty() {
            return false;
        }
        self.segments.push(cleaned);
        true
    }

    /// Durée totale d'audio reçu depuis le début de la session, en ms.
    pub fn duration_ms(&self) -> u64 {
        self.total_samples * 1000 / u64::from(SAMPLE_RATE)
    }

    /// Temps ASR cumulé sur l'ensemble des tranches transcrites, en ms.
    pub fn total_asr_ms(&self) -> u64 {
        self.asr_ms
    }

    /// Nombre total de tranches découpées (silencieuses comprises).
    pub fn emitted_chunks(&self) -> usize {
        self.emitted_chunks
    }

    /// Nombre de tranches ayant produit du texte exploitable.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Texte brut complet assemblé depuis toutes les tranches transcrites.
    pub fn full_raw(&self) -> String {
        join_live_segments(&self.segments)
    }

    /// Aperçu borné des derniers mots transcrits, prêt à afficher dans la
    /// pastille PC ou la barre d'état Android.
    pub fn preview(&self, max_chars: usize) -> String {
        preview_tail(&self.full_raw(), max_chars)
    }

    /// Résumé prêt pour [`crate::Pipeline::finalize_raw`].
    pub fn to_raw(&self) -> RawDictation {
        RawDictation {
            raw: self.full_raw(),
            duration_ms: self.duration_ms(),
            asr_ms: self.total_asr_ms(),
        }
    }
}

fn is_silent_chunk(chunk: &Chunk) -> bool {
    audio::is_silent(&chunk.samples, SAMPLE_RATE, SILENCE_RMS)
}

/// Assemble les tranches brutes consécutives avec un espacement propre (pas de
/// double espace, ni d'espace parasite avant une virgule/point ou après une
/// apostrophe/parenthèse ouvrante).
pub fn join_live_segments(segments: &[String]) -> String {
    let mut out = String::new();
    for segment in segments {
        let part = segment.trim();
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() && needs_space_between(&out, part) {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn needs_space_between(left: &str, right: &str) -> bool {
    let no_space_after = left.ends_with(['\'', '’', '(', '[', '\n']);
    let no_space_before = right.starts_with([',', '.', '…', ')', ']']);
    !(no_space_after || no_space_before)
}

/// Extrait la fin d'un texte brut sur au plus `max_chars` caractères Unicode,
/// en normalisant les retours à la ligne / espaces multiples et en préférant
/// couper sur une frontière de mot avec un préfixe « … ».
pub fn preview_tail(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return String::new();
    }
    let total = normalized.chars().count();
    if total <= max_chars {
        return normalized;
    }
    if max_chars <= 2 {
        return normalized
            .chars()
            .skip(total - max_chars)
            .collect::<String>();
    }
    // Budget hors préfixe « …  » (2 caractères : ellipse + espace).
    let budget = max_chars - 2;
    let start_char = total - budget;
    let byte_start = normalized
        .char_indices()
        .nth(start_char)
        .map_or(normalized.len(), |(idx, _)| idx);
    let raw_tail = &normalized[byte_start..];
    // Si on coupe au milieu d'un mot (caractère juste avant non-espace et
    // premier caractère non-espace), avancer au mot suivant tant qu'on garde
    // au moins la moitié du budget visible.
    let prev_char = normalized[..byte_start].chars().next_back();
    let trimmed = if prev_char.is_some_and(|c| !c.is_whitespace())
        && raw_tail.starts_with(|c: char| !c.is_whitespace())
        && let Some(space_idx) = raw_tail.find(' ')
    {
        let candidate = raw_tail[space_idx + 1..].trim_start();
        if !candidate.is_empty() && candidate.chars().count() * 2 >= budget {
            candidate
        } else {
            raw_tail.trim_start()
        }
    } else {
        raw_tail.trim_start()
    };
    format!("… {trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = SAMPLE_RATE as usize;

    fn tone(seconds: f32, amp: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| if i % 2 == 0 { amp } else { -amp })
            .collect()
    }

    fn speech(seconds: f32) -> Vec<f32> {
        tone(seconds, 0.25)
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    #[test]
    fn splits_on_first_pause_after_live_min_bound() {
        let mut session = LiveSession::new();
        let mut audio = speech(2.0);
        audio.extend(silence(0.5)); // trop tôt (< 3,5 s)
        audio.extend(speech(2.0));
        audio.extend(silence(0.5)); // première pause après 3,5 s
        audio.extend(speech(1.5));

        let clips = session.feed(&audio);
        assert_eq!(clips.len(), 1);
        let dur = clips[0].duration_ms();
        assert!(
            (4_500..=5_000).contains(&dur),
            "coupé dans la pause : {dur} ms"
        );
        let tail = session.finish().expect("fin de dictée attendue");
        assert!(tail.duration_ms() >= 1_400);
        assert_eq!(session.duration_ms(), 6_500);
        assert_eq!(session.emitted_chunks(), 2);
    }

    #[test]
    fn continuous_speech_cuts_before_live_max_bound() {
        let mut session = LiveSession::new();
        let clips = session.feed(&speech(20.0));
        assert_eq!(clips.len(), 2);
        for clip in &clips {
            let dur = clip.duration_ms();
            assert!((6_000..=LIVE_MAX_CHUNK_MS).contains(&dur), "{dur} ms");
        }
    }

    #[test]
    fn quiet_speech_above_dictation_threshold_is_not_dropped() {
        // Amplitude 0.008 : sous le seuil des réunions (0.01) mais au-dessus du
        // seuil de dictée (0.005). Les micros peu sensibles doivent passer.
        let mut session = LiveSession::new();
        let mut audio = tone(4.0, 0.008);
        audio.extend(silence(0.5));
        let clips = session.feed(&audio);
        assert_eq!(clips.len(), 1);
    }

    #[test]
    fn silent_chunks_are_ignored() {
        let mut session = LiveSession::new();
        assert!(session.feed(&silence(10.0)).is_empty());
        assert!(session.emitted_chunks() >= 1);
        assert!(session.finish().is_none());
    }

    #[test]
    fn push_text_filters_hallucinations_and_tracks_asr_ms() {
        let mut session = LiveSession::new();
        assert!(!session.push_text("Sous-titrage ST' 501", 120));
        assert!(session.is_empty());
        assert_eq!(session.total_asr_ms(), 120);

        assert!(session.push_text("Bonjour tout le monde.", 180));
        assert!(session.push_text("  Voici la suite. ", 210));
        assert_eq!(session.segment_count(), 2);
        assert_eq!(session.total_asr_ms(), 510);
        assert_eq!(session.full_raw(), "Bonjour tout le monde. Voici la suite.");
    }

    #[test]
    fn join_live_segments_handles_punctuation_and_apostrophes() {
        let segs = vec![
            "Bonjour".to_owned(),
            ", voici l'".to_owned(),
            "exemple".to_owned(),
            ".".to_owned(),
            "   ".to_owned(),
            "À bientôt !".to_owned(),
        ];
        assert_eq!(
            join_live_segments(&segs),
            "Bonjour, voici l'exemple. À bientôt !"
        );
    }

    #[test]
    fn preview_tail_respects_budget_utf8_and_word_boundaries() {
        assert_eq!(preview_tail("", 40), "");
        assert_eq!(preview_tail("   \n\t  ", 40), "");
        assert_eq!(preview_tail("Bonjour à tous.", 0), "");
        assert_eq!(preview_tail("Bonjour   à\ntous.", 40), "Bonjour à tous.");

        let long = "Bonjour tout le monde, ceci est une dictée longue avec des accents éàçù pour tester l'aperçu.";
        let cut = preview_tail(long, 42);
        assert!(cut.starts_with("… "), "got: {cut}");
        assert!(
            cut.chars().count() <= 42,
            "len={}: {cut}",
            cut.chars().count()
        );
        assert!(cut.ends_with("pour tester l'aperçu."), "got: {cut}");
        // Pas de mot tronqué juste après « …  ».
        assert!(
            cut == "… des accents éàçù pour tester l'aperçu."
                || cut == "… accents éàçù pour tester l'aperçu.",
            "got: {cut}"
        );
    }

    #[test]
    fn preview_tail_handles_single_long_token_without_spaces() {
        let token = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let cut = preview_tail(token, 12);
        assert_eq!(cut.chars().count(), 12);
        assert!(cut.starts_with("… "));
    }

    #[test]
    fn to_raw_bundles_text_duration_and_asr_ms() {
        let mut session = LiveSession::new();
        session.feed(&speech(4.0));
        session.push_text("Première phrase.", 300);
        session.feed(&speech(2.0));
        session.finish();
        session.push_text("Deuxième phrase.", 250);

        let raw = session.to_raw();
        assert_eq!(raw.raw, "Première phrase. Deuxième phrase.");
        assert_eq!(raw.duration_ms, 6_000);
        assert_eq!(raw.asr_ms, 550);
    }
}
