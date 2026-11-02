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
}
