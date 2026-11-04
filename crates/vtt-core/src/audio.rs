//! Conversions audio : le micro livre son format natif (souvent 48 kHz
//! stéréo), Whisper attend du 16 kHz mono.

/// Moyenne des canaux d'un flux entrelacé.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Rééchantillonnage simple, suffisant pour la voix.
///
/// En sous-échantillonnage, chaque échantillon de sortie est la moyenne de la
/// fenêtre d'entrée qu'il couvre (filtre anti-repliement rudimentaire) ;
/// en sur-échantillonnage, interpolation linéaire.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    if ratio > 1.0 {
        for i in 0..out_len {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize)
                .min(input.len())
                .max(start + 1);
            let window = &input[start..end];
            out.push(window.iter().sum::<f32>() / window.len() as f32);
        }
    } else {
        for i in 0..out_len {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = input.get(idx + 1).copied().unwrap_or(a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// Vrai si aucune fenêtre de 30 ms ne dépasse `threshold` en RMS.
///
/// Évite d'envoyer du silence à Whisper, qui « hallucine » alors des phrases.
pub fn is_silent(samples: &[f32], sample_rate: u32, threshold: f32) -> bool {
    let frame = (sample_rate as usize * 30 / 1000).max(1);
    samples.chunks(frame).all(|chunk| {
        let mean_square = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32;
        mean_square.sqrt() < threshold
    })
}

/// Niveau affiché par la pastille, entre 0 et 1, d'après le RMS d'un bloc :
/// -50 dBFS (silence) → 0, -8 dBFS (voix forte) → 1.
pub fn meter_level(rms: f32) -> f32 {
    const FLOOR_DB: f32 = -50.0;
    const CEIL_DB: f32 = -8.0;
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

/// Moyenne quadratique.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Fréquence des sons de début et de fin de dictée.
const TONE_RATE: u32 = 22_050;

/// Petit son (fichier WAV 16 bits mono en mémoire) : suite de notes
/// `(fréquence en Hz, durée en ms)`, avec fondu pour éviter les clics.
pub fn tone_wav(notes: &[(f32, u32)], volume: f32) -> Vec<u8> {
    let fade = (TONE_RATE / 200) as usize; // 5 ms
    let mut samples: Vec<i16> = Vec::new();
    for &(frequency, ms) in notes {
        let count = (TONE_RATE * ms / 1000) as usize;
        for i in 0..count {
            let envelope = (i.min(count - 1 - i) as f32 / fade as f32).min(1.0);
            let t = i as f32 / TONE_RATE as f32;
            let value = (2.0 * std::f32::consts::PI * frequency * t).sin() * volume * envelope;
            samples.push((value * i16::MAX as f32) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // taille du bloc fmt
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&TONE_RATE.to_le_bytes());
    wav.extend_from_slice(&(TONE_RATE * 2).to_le_bytes()); // octets par seconde
    wav.extend_from_slice(&2u16.to_le_bytes()); // octets par échantillon
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(downmix(&[0.1, 0.2], 1), [0.1, 0.2]);
    }

    #[test]
    fn resample_48k_to_16k_divides_length_by_three() {
        let input = vec![0.3; 4800];
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 1600);
        assert!(out.iter().all(|s| (s - 0.3).abs() < 1e-6));
    }

    #[test]
    fn resample_44100_keeps_duration() {
        let out = resample(&vec![0.0; 44_100], 44_100, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_upsamples_linearly() {
        assert_eq!(resample(&[0.0, 1.0], 8_000, 16_000), [0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn resample_keeps_low_frequency_tone() {
        // 440 Hz à 48 kHz → l'amplitude doit survivre au passage à 16 kHz.
        let input: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        let out = resample(&input, 48_000, 16_000);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.9, "pic {peak}");
    }

    #[test]
    fn silence_detection() {
        assert!(is_silent(&vec![0.001; 16_000], 16_000, 0.005));
        let mut speech = vec![0.0; 16_000];
        speech[8_000..8_480].fill(0.2);
        assert!(!is_silent(&speech, 16_000, 0.005));
    }

    #[test]
    fn meter_level_scale() {
        assert_eq!(meter_level(0.0), 0.0);
        assert_eq!(meter_level(0.001), 0.0, "-60 dBFS : silence");
        assert_eq!(meter_level(1.0), 1.0);
        let speech = meter_level(0.05); // -26 dBFS
        assert!(speech > 0.4 && speech < 0.7, "{speech}");
    }

    #[test]
    fn tone_is_a_valid_wav() {
        let wav = tone_wav(&[(660.0, 60), (880.0, 60)], 0.2);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        assert_eq!(data_len, 2 * 2 * (TONE_RATE as usize * 60 / 1000));
        // Fondu : premier échantillon nul.
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), 0);
    }
}
