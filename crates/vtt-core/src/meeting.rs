//! Réunions : découpage de l'audio en blocs aux pauses, transcription
//! horodatée « Moi / Eux », compte rendu Markdown, audio chiffré sur disque.
//!
//! Tout ce qui ne dépend pas de l'OS (capture, chiffrement, fenêtres) vit
//! ici et se teste sans micro.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::audio;
use crate::pipeline::SAMPLE_RATE;

/// Qui parle : le micro (l'utilisateur) ou le son du PC (les autres).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Moi,
    Eux,
}

impl Track {
    pub fn label(self) -> &'static str {
        match self {
            Track::Moi => "Moi",
            Track::Eux => "Eux",
        }
    }

    /// Nom du fichier audio de la piste.
    pub fn file(self) -> &'static str {
        match self {
            Track::Moi => "micro.audio",
            Track::Eux => "systeme.audio",
        }
    }
}

/// Bloc d'audio à transcrire, en 16 kHz mono.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub start_ms: u64,
    pub samples: Vec<f32>,
}

impl Chunk {
    pub fn end_ms(&self) -> u64 {
        self.start_ms + samples_to_ms(self.samples.len() as u64)
    }

    /// Aucune parole probable : inutile de le transcrire (Whisper
    /// « hallucine » sur le silence).
    pub fn is_silent(&self) -> bool {
        audio::is_silent(&self.samples, SAMPLE_RATE, SILENCE_RMS)
    }
}

fn samples_to_ms(samples: u64) -> u64 {
    samples * 1000 / u64::from(SAMPLE_RATE)
}

fn ms_to_samples(ms: u64) -> usize {
    (ms * u64::from(SAMPLE_RATE) / 1000) as usize
}

/// Un bloc fait au moins 6 s (contexte suffisant pour bien transcrire)...
const MIN_CHUNK_MS: u64 = 6_000;
/// ... et au plus 15 s : au-delà, il est coupé au moment le plus calme.
const MAX_CHUNK_MS: u64 = 15_000;
/// Analyse par trames de 20 ms.
const FRAME_MS: u64 = 20;
/// Une pause de 300 ms entre deux phrases permet de couper sans couper un mot.
const PAUSE_FRAMES: usize = 15;
/// En dessous (RMS), une trame est considérée comme une pause.
const PAUSE_RMS: f32 = 0.01;
/// Même seuil que la dictée : un bloc entièrement sous ce niveau est ignoré.
const SILENCE_RMS: f32 = 0.01;
/// Reste plus court à la fin : ignoré.
const MIN_TAIL_MS: u64 = 300;

/// Découpe un flux continu en blocs de 6 à 15 s, aux pauses.
#[derive(Debug, Default)]
pub struct Chunker {
    buffer: Vec<f32>,
    /// Position (en échantillons) du début de `buffer` depuis le début.
    start: u64,
}

impl Chunker {
    /// Reprise après coup (transcription d'un enregistrement interrompu) :
    /// le premier bloc commence à `start_ms`.
    pub fn starting_at(start_ms: u64) -> Self {
        Self {
            buffer: Vec::new(),
            start: ms_to_samples(start_ms) as u64,
        }
    }

    /// Ajoute des échantillons ; renvoie les blocs complets.
    pub fn feed(&mut self, samples: &[f32]) -> Vec<Chunk> {
        self.buffer.extend_from_slice(samples);
        let mut chunks = Vec::new();
        while let Some(cut) = self.cut_point() {
            chunks.push(self.split(cut));
        }
        chunks
    }

    /// Fin du flux : le reste, s'il n'est pas trop court.
    pub fn finish(&mut self) -> Option<Chunk> {
        let len = self.buffer.len();
        if samples_to_ms(len as u64) < MIN_TAIL_MS {
            self.start += len as u64;
            self.buffer.clear();
            return None;
        }
        Some(self.split(len))
    }

    fn split(&mut self, cut: usize) -> Chunk {
        let rest = self.buffer.split_off(cut);
        let samples = std::mem::replace(&mut self.buffer, rest);
        let chunk = Chunk {
            start_ms: samples_to_ms(self.start),
            samples,
        };
        self.start += cut as u64;
        chunk
    }

    /// Où couper le tampon, s'il contient un bloc complet.
    fn cut_point(&self) -> Option<usize> {
        let frame = ms_to_samples(FRAME_MS);
        let min = ms_to_samples(MIN_CHUNK_MS);
        let max = ms_to_samples(MAX_CHUNK_MS);
        if self.buffer.len() < min {
            return None;
        }
        // Première pause assez longue après le minimum.
        let limit = self.buffer.len().min(max);
        let mut quiet = 0;
        let mut position = min;
        while position + frame <= limit {
            if audio::rms(&self.buffer[position..position + frame]) < PAUSE_RMS {
                quiet += 1;
                if quiet == PAUSE_FRAMES {
                    // Milieu de la pause.
                    return Some(position + frame - PAUSE_FRAMES * frame / 2);
                }
            } else {
                quiet = 0;
            }
            position += frame;
        }
        if self.buffer.len() < max {
            return None;
        }
        // Pas de pause : la trame la plus calme des 3 dernières secondes.
        let from = max - ms_to_samples(3_000);
        let quietest = (from..max - frame)
            .step_by(frame)
            .min_by(|&a, &b| {
                audio::rms(&self.buffer[a..a + frame])
                    .total_cmp(&audio::rms(&self.buffer[b..b + frame]))
            })
            .unwrap_or(max);
        Some(quietest + frame / 2)
    }
}

/// Phrase transcrite, horodatée depuis le début de la réunion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub track: Track,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Déjà copiée par « Copier les nouvelles répliques ».
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub copied: bool,
}

/// Les blocs des deux pistes arrivent dans le désordre : ordre chronologique.
pub fn sort_segments(segments: &mut [Segment]) {
    segments.sort_by_key(|s| (s.start_ms, s.track == Track::Eux));
}

/// `12:34`, ou `1:02:03` au-delà d'une heure.
pub fn timestamp(ms: u64) -> String {
    let seconds = ms / 1000;
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Au-delà de ce silence, une même personne commence une nouvelle réplique.
const TURN_GAP_MS: u64 = 10_000;

/// Répliques : segments consécutifs et rapprochés d'une même piste réunis.
pub fn turns(segments: &[Segment]) -> Vec<(Track, u64, String)> {
    let mut turns: Vec<(Track, u64, String)> = Vec::new();
    let mut last_end = 0;
    for segment in segments {
        match turns.last_mut() {
            Some((track, _, text))
                if *track == segment.track && segment.start_ms <= last_end + TURN_GAP_MS =>
            {
                text.push(' ');
                text.push_str(&segment.text);
            }
            _ => turns.push((segment.track, segment.start_ms, segment.text.clone())),
        }
        last_end = segment.end_ms;
    }
    turns
}

/// Transcription lisible, une réplique par ligne : `[12:34] Moi : …`.
pub fn transcript_text(segments: &[Segment]) -> String {
    turns(segments)
        .into_iter()
        .map(|(track, start, text)| format!("[{}] {} : {text}", timestamp(start), track.label()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Fichier Markdown d'une réunion : en-tête, compte rendu, transcription.
pub fn to_markdown(
    title: &str,
    date: &str,
    duration_ms: u64,
    summary: Option<&str>,
    segments: &[Segment],
) -> String {
    let mut md = format!("# {title}\n\n{date} · durée {}\n\n", timestamp(duration_ms));
    if let Some(summary) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        md.push_str("## Compte rendu\n\n");
        md.push_str(summary);
        md.push_str("\n\n");
    }
    md.push_str("## Transcription\n\n");
    for (track, start, text) in turns(segments) {
        md.push_str(&format!(
            "**{}** [{}] : {text}\n\n",
            track.label(),
            timestamp(start)
        ));
    }
    md
}

/// Trames de 20 ms pour la comparaison des deux pistes.
const ECHO_FRAME: usize = SAMPLE_RATE as usize / 50;
/// Son du PC gardé pour comparaison (en trames) : une minute.
const ECHO_HISTORY: usize = 3_000;
/// Retard du micro sur le son du PC exploré : 0 à 600 ms.
const ECHO_MAX_LAG: usize = 30;
/// Niveau (dBFS) sous lequel une trame est du silence.
const ECHO_FLOOR_DB: f32 = -50.0;
/// Corrélation minimale des deux pistes pour parler d'écho (au casque, le
/// micro n'entend pas les autres : pas de corrélation).
const ECHO_MIN_CORRELATION: f32 = 0.5;
/// Une trame est « locale » si le micro dépasse l'écho attendu de 9 dB.
const ECHO_LOCAL_DB: f32 = 9.0;
/// Bloc d'écho : au plus 20 % de trames locales.
const ECHO_MAX_LOCAL: f32 = 0.2;

fn db(rms: f32) -> f32 {
    20.0 * rms.max(1e-5).log10()
}

fn frame_levels(samples: &[f32]) -> Vec<f32> {
    samples
        .as_chunks::<ECHO_FRAME>()
        .0
        .iter()
        .map(|f| db(audio::rms(f)))
        .collect()
}

/// Appel sur haut-parleurs : le micro réentend les autres. Compare un bloc
/// du micro au son du PC du même moment : si le micro ne fait que suivre le
/// son du PC (même enveloppe, un peu plus tard, plus faible), c'est de
/// l'écho et le bloc n'est pas transcrit comme « Moi ».
#[derive(Debug, Default)]
pub struct EchoGate {
    /// Niveaux (dB) des trames du son du PC ; `frames[0]` est la trame `first`.
    frames: std::collections::VecDeque<f32>,
    first: usize,
    /// Fin du dernier bloc reçu, pas encore une trame complète.
    partial: Vec<f32>,
}

impl EchoGate {
    /// Son du PC, dans l'ordre et sans trou depuis le début.
    pub fn feed_system(&mut self, samples: &[f32]) {
        self.partial.extend_from_slice(samples);
        let whole = self.partial.len() / ECHO_FRAME * ECHO_FRAME;
        let levels = frame_levels(&self.partial[..whole]);
        self.partial.drain(..whole);
        self.frames.extend(levels);
        while self.frames.len() > ECHO_HISTORY {
            self.frames.pop_front();
            self.first += 1;
        }
    }

    fn system_level(&self, frame: usize) -> Option<f32> {
        frame
            .checked_sub(self.first)
            .and_then(|i| self.frames.get(i))
            .copied()
    }

    /// Le bloc du micro n'est que de l'écho du son du PC.
    pub fn is_echo(&self, chunk: &Chunk) -> bool {
        let mic = frame_levels(&chunk.samples);
        let start = (chunk.start_ms / 20) as usize;
        let active: Vec<usize> = (0..mic.len()).filter(|&i| mic[i] > ECHO_FLOOR_DB).collect();
        if active.len() < 10 {
            return false;
        }
        // Retard qui aligne le mieux les deux enveloppes.
        let mut best: Option<(f32, usize)> = None;
        for lag in 0..=ECHO_MAX_LAG {
            let pairs: Vec<(f32, f32)> = active
                .iter()
                .filter_map(|&i| {
                    let system = self.system_level((start + i).checked_sub(lag)?)?;
                    Some((mic[i], system))
                })
                .collect();
            if pairs.len() < active.len() * 3 / 4 {
                continue;
            }
            let correlation = pearson(&pairs);
            if best.is_none_or(|(c, _)| correlation > c) {
                best = Some((correlation, lag));
            }
        }
        let Some((correlation, lag)) = best else {
            return false;
        };
        if correlation < ECHO_MIN_CORRELATION {
            return false;
        }
        let pairs: Vec<(f32, f32)> = active
            .iter()
            .filter_map(|&i| Some((mic[i], self.system_level((start + i).checked_sub(lag)?)?)))
            .collect();
        // Atténuation habituelle haut-parleur → micro (médiane).
        let mut gaps: Vec<f32> = pairs
            .iter()
            .filter(|(_, s)| *s > ECHO_FLOOR_DB)
            .map(|(m, s)| m - s)
            .collect();
        if gaps.is_empty() {
            return false;
        }
        gaps.sort_by(f32::total_cmp);
        let gain = gaps[gaps.len() / 2];
        let local = pairs
            .iter()
            .filter(|(m, s)| *m > s + gain + ECHO_LOCAL_DB)
            .count();
        (local as f32) / (pairs.len() as f32) <= ECHO_MAX_LOCAL
    }
}

fn pearson(pairs: &[(f32, f32)]) -> f32 {
    let n = pairs.len() as f32;
    let (mx, my) = pairs
        .iter()
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x / n, b + y / n));
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        cov += (x - mx) * (y - my);
        vx += (x - mx).powi(2);
        vy += (y - my).powi(2);
    }
    if vx <= f32::EPSILON || vy <= f32::EPSILON {
        return 0.0;
    }
    cov / (vx.sqrt() * vy.sqrt())
}

/// Mots comparables (minuscules, au moins deux caractères).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

/// Fenêtre autour d'une réplique « Moi » où chercher l'original de l'écho.
const ECHO_TEXT_WINDOW_MS: u64 = 1_500;
/// Part des mots retrouvés chez « Eux » au-delà de laquelle c'est de l'écho.
const ECHO_TEXT_SHARE: f32 = 0.6;

/// Retire les segments « Moi » qui répètent ce que « Eux » disaient au même
/// moment (écho que le signal n'a pas suffi à écarter).
pub fn without_echo(segments: &[Segment]) -> Vec<Segment> {
    segments
        .iter()
        .filter(|segment| {
            if segment.track != Track::Moi {
                return true;
            }
            let mine = words(&segment.text);
            if mine.len() < 3 {
                return true;
            }
            let theirs: std::collections::HashSet<String> = segments
                .iter()
                .filter(|o| {
                    o.track == Track::Eux
                        && o.start_ms <= segment.end_ms + ECHO_TEXT_WINDOW_MS
                        && o.end_ms + ECHO_TEXT_WINDOW_MS >= segment.start_ms
                })
                .flat_map(|o| words(&o.text))
                .collect();
            let shared = mine.iter().filter(|w| theirs.contains(*w)).count();
            (shared as f32) / (mine.len() as f32) < ECHO_TEXT_SHARE
        })
        .cloned()
        .collect()
}

/// Chiffrement d'un bloc d'audio (Windows : DPAPI, lié à la session).
pub type Protect = fn(&[u8]) -> Result<Vec<u8>, String>;

/// Audio d'une piste sur disque : suite de blocs chiffrés séparément
/// (`longueur u32 LE` + bloc), pour écrire au fil de l'eau et ne perdre que
/// le dernier bloc en cas de coupure. Chaque bloc : PCM 16 bits, 16 kHz mono.
pub fn append_audio(path: &Path, samples: &[f32], protect: Protect) -> Result<(), String> {
    if samples.is_empty() {
        return Ok(());
    }
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes())
        .collect();
    let block = protect(&pcm)?;
    let len = u32::try_from(block.len()).map_err(|_| "bloc audio trop grand".to_owned())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&len.to_le_bytes())
        .and_then(|()| file.write_all(&block))
        .map_err(|e| format!("écriture de {} : {e}", path.display()))
}

/// Relit l'audio d'une piste ; un dernier bloc tronqué (coupure pendant
/// l'écriture) est ignoré.
pub fn read_audio(path: &Path, unprotect: Protect) -> Result<Vec<f32>, String> {
    read_audio_range(path, unprotect, 0, usize::MAX)
}

/// Extrait d'une piste, échantillons `start..end` : la lecture s'arrête dès
/// la fin de l'extrait (inutile de tout déchiffrer pour un passage du début).
pub fn read_audio_range(
    path: &Path,
    unprotect: Protect,
    start: usize,
    end: usize,
) -> Result<Vec<f32>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut samples = Vec::new();
    // Position (en échantillons) du début du prochain bloc.
    let mut position = 0usize;
    while position < end {
        let mut len = [0u8; 4];
        if file.read_exact(&mut len).is_err() {
            break;
        }
        let mut block = vec![0u8; u32::from_le_bytes(len) as usize];
        if file.read_exact(&mut block).is_err() {
            log::warn!("{} : dernier bloc incomplet ignoré", path.display());
            break;
        }
        let pcm = unprotect(&block)?;
        let block_samples = pcm.len() / 2;
        let from = start.saturating_sub(position).min(block_samples);
        let to = end.saturating_sub(position).min(block_samples);
        samples.extend(
            pcm.as_chunks::<2>().0[from..to]
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / f32::from(i16::MAX)),
        );
        position += block_samples;
    }
    Ok(samples)
}

/// Fichier WAV (16 bits, 16 kHz mono) pour exporter l'audio d'une piste.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    wav
}

/// Supprime les fichiers audio d'une réunion.
pub fn delete_audio(dir: &Path) {
    for track in [Track::Moi, Track::Eux] {
        let path = dir.join(track.file());
        if let Err(e) = fs::remove_file(&path)
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("suppression de {} : {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = SAMPLE_RATE as usize;

    /// `seconds` de « parole » (bruit d'amplitude 0,3).
    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    #[test]
    fn chunks_are_cut_at_the_first_pause_after_six_seconds() {
        let mut chunker = Chunker::default();
        let mut audio = speech(4.0);
        audio.extend(silence(0.5)); // trop tôt : pas de coupure
        audio.extend(speech(3.0));
        audio.extend(silence(0.5)); // première pause après 6 s
        audio.extend(speech(2.0));
        let chunks = chunker.feed(&audio);
        assert_eq!(chunks.len(), 1);
        let end = chunks[0].end_ms();
        assert!(
            (7_500..=8_000).contains(&end),
            "coupé dans la pause : {end} ms"
        );
        let tail = chunker.finish().unwrap();
        assert_eq!(tail.start_ms, end, "aucun trou entre deux blocs");
        assert!(!tail.is_silent());
    }

    #[test]
    fn long_speech_without_pause_is_cut_at_fifteen_seconds() {
        let mut chunker = Chunker::default();
        let chunks = chunker.feed(&speech(40.0));
        assert_eq!(chunks.len(), 3);
        for chunk in &chunks {
            let ms = chunk.end_ms() - chunk.start_ms;
            assert!((12_000..=15_000).contains(&ms), "{ms} ms");
        }
        assert_eq!(chunks[1].start_ms, chunks[0].end_ms());
        assert_eq!(chunks[2].start_ms, chunks[1].end_ms());
    }

    #[test]
    fn fed_in_small_pieces_gives_the_same_chunks() {
        let mut audio = speech(7.0);
        audio.extend(silence(1.0));
        audio.extend(speech(7.0));
        let whole = Chunker::default().feed(&audio);
        let mut chunker = Chunker::default();
        let pieces: Vec<Chunk> = audio
            .chunks(RATE / 10)
            .flat_map(|p| chunker.feed(p))
            .collect();
        assert_eq!(pieces, whole);
    }

    #[test]
    fn silence_and_short_tails_are_skipped() {
        let mut chunker = Chunker::default();
        let chunks = chunker.feed(&silence(10.0));
        assert!(chunks.iter().all(Chunk::is_silent));
        let mut chunker = Chunker::starting_at(60_000);
        chunker.feed(&speech(0.2));
        assert_eq!(chunker.finish(), None);
        chunker.feed(&speech(1.0));
        assert_eq!(chunker.finish().unwrap().start_ms, 60_200);
    }

    fn segment(track: Track, start_ms: u64, text: &str) -> Segment {
        Segment {
            track,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
            copied: false,
        }
    }

    #[test]
    fn transcript_merges_turns_in_time_order() {
        let mut segments = vec![
            segment(Track::Eux, 9_000, "Très bien."),
            segment(Track::Moi, 0, "Bonjour à tous."),
            segment(Track::Moi, 4_000, "On commence ?"),
            segment(Track::Eux, 3_661_000, "Au revoir."),
        ];
        sort_segments(&mut segments);
        assert_eq!(
            transcript_text(&segments),
            "[00:00] Moi : Bonjour à tous. On commence ?\n[00:09] Eux : Très bien.\n\
             [1:01:01] Eux : Au revoir."
        );
        let md = to_markdown(
            "Point hebdo",
            "4 nov. 2026",
            3_662_000,
            Some("Rien."),
            &segments,
        );
        assert!(md.starts_with("# Point hebdo\n\n4 nov. 2026 · durée 1:01:02"));
        assert!(md.contains("## Compte rendu\n\nRien."));
        assert!(md.contains("**Eux** [00:09] : Très bien."));
    }

    /// Enveloppe de parole pseudo-aléatoire (syllabes de 100 à 300 ms).
    fn bursts(seconds: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as f32 / 65_536.0
        };
        let mut out = Vec::new();
        while out.len() < (seconds * RATE as f32) as usize {
            let len = (0.1 + 0.2 * next()) * RATE as f32;
            let amplitude = if next() < 0.35 {
                0.0
            } else {
                0.05 + 0.4 * next()
            };
            for i in 0..len as usize {
                out.push(if i % 2 == 0 { amplitude } else { -amplitude });
            }
        }
        out.truncate((seconds * RATE as f32) as usize);
        out
    }

    fn delayed(samples: &[f32], ms: usize, gain: f32) -> Vec<f32> {
        let lag = RATE * ms / 1000;
        let mut out = vec![0.0; lag];
        out.extend(samples.iter().map(|s| s * gain));
        out.truncate(samples.len());
        out
    }

    #[test]
    fn speaker_bleed_is_echo_but_own_speech_is_not() {
        let system = bursts(10.0, 7);
        let mut gate = EchoGate::default();
        for piece in system.chunks(RATE / 3) {
            gate.feed_system(piece);
        }
        let chunk = |samples: Vec<f32>| Chunk {
            start_ms: 0,
            samples,
        };
        // Haut-parleurs : le micro entend les autres 120 ms plus tard, 4× plus bas.
        assert!(gate.is_echo(&chunk(delayed(&system, 120, 0.25))));
        // Ma propre voix, sans rapport avec le son du PC.
        assert!(!gate.is_echo(&chunk(bursts(10.0, 99))));
        // Je parle par-dessus l'écho : gardé.
        let mut both = delayed(&system, 120, 0.1);
        for (sample, own) in both.iter_mut().zip(bursts(10.0, 99)) {
            *sample += own;
        }
        assert!(!gate.is_echo(&chunk(both)));
        // Au casque : son du PC silencieux pour le micro.
        let silent_pc = EchoGate::default();
        assert!(!silent_pc.is_echo(&chunk(bursts(10.0, 99))));
    }

    #[test]
    fn text_echo_of_their_words_is_dropped() {
        let segments = vec![
            segment(Track::Eux, 0, "Le budget est validé à cinq mille euros."),
            segment(Track::Moi, 500, "budget est validé à cinq mille euros"),
            segment(Track::Moi, 1_000, "Parfait, je préviens Julie."),
            segment(Track::Moi, 60_000, "Le budget est validé, donc."),
        ];
        let kept = without_echo(&segments);
        let texts: Vec<&str> = kept.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Le budget est validé à cinq mille euros.",
                "Parfait, je préviens Julie.",
                "Le budget est validé, donc."
            ]
        );
    }

    #[test]
    fn audio_round_trips_through_protected_blocks() {
        fn xor(data: &[u8]) -> Result<Vec<u8>, String> {
            Ok(data.iter().map(|b| b ^ 0x5a).collect())
        }
        let dir = std::env::temp_dir().join(format!("vtt-meeting-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(Track::Moi.file());
        let _ = fs::remove_file(&path);
        append_audio(&path, &[0.5, -0.5], xor).unwrap();
        append_audio(&path, &[0.25], xor).unwrap();
        // Coupure pendant l'écriture : bloc tronqué à la fin.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[200, 0, 0, 0, 1, 2]).unwrap();
        let samples = read_audio(&path, xor).unwrap();
        assert_eq!(samples.len(), 3);
        assert!((samples[0] - 0.5).abs() < 1e-3 && (samples[2] - 0.25).abs() < 1e-3);
        assert!(
            !fs::read(&path)
                .unwrap()
                .windows(2)
                .any(|w| w == 16383i16.to_le_bytes())
        );
        assert_eq!(
            read_audio(&dir.join("absent"), xor).unwrap(),
            Vec::<f32>::new()
        );
        // Extrait à cheval sur deux blocs.
        let part = read_audio_range(&path, xor, 1, 3).unwrap();
        assert_eq!(part.len(), 2);
        assert!((part[0] + 0.5).abs() < 1e-3 && (part[1] - 0.25).abs() < 1e-3);
        assert_eq!(&wav_bytes(&samples)[..4], b"RIFF");
        delete_audio(&dir);
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
