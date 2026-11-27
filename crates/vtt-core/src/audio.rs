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

/// Fréquence de coupure du filtre passe-haut micro (Hz) : coupe le décalage
/// continu, le bourdonnement secteur (50/60 Hz), les vibrations de bureau et
/// les souffles graves sans altérer les formants de la voix humaine.
pub const HIGHPASS_CUTOFF_HZ: f32 = 80.0;

/// Seuil RMS minimal d'une trame de 30 ms pour être considérée comme parole
/// active par l'AGC (~ -44 dBFS, au-dessus du seuil de silence). En dessous,
/// le gain reste à 1 pour ne jamais amplifier le souffle ni le bruit de fond.
const AGC_SPEECH_FLOOR_RMS: f32 = 0.006;
/// Niveau cible visé par l'AGC pour la parole active (~ -21 dBFS).
const AGC_TARGET_RMS: f32 = 0.09;
/// Zone morte : une voix déjà bien calibrée n'est pas modifiée.
const AGC_DEADBAND_LOW: f32 = 0.07;
const AGC_DEADBAND_HIGH: f32 = 0.14;
/// Bornes du gain appliqué (+15,5 dB max, -6 dB max).
const AGC_MIN_GAIN: f32 = 0.5;
const AGC_MAX_GAIN: f32 = 6.0;
/// Coude du limiteur doux : en dessous, le signal reste strictement linéaire.
const LIMITER_KNEE: f32 = 0.85;
const LIMITER_CEIL: f32 = 0.98;

/// Filtre passe-haut Butterworth d'ordre 2 (biquad) précédé d'un retrait du
/// décalage continu (DC) pour éviter tout transitoire d'attaque.
///
/// Préserve exactement le nombre d'échantillons (donc les horodatages).
pub fn highpass_filter(samples: &[f32], sample_rate: u32, cutoff_hz: f32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    if sample_rate == 0 || cutoff_hz <= 0.0 || cutoff_hz >= sample_rate as f32 * 0.45 {
        return samples
            .iter()
            .map(|&s| if s.is_finite() { s } else { 0.0 })
            .collect();
    }

    // Retrait de la composante continue (DC) avant le filtre récursif.
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for &s in samples {
        if s.is_finite() {
            sum += f64::from(s);
            count += 1;
        }
    }
    let dc = if count > 0 {
        (sum / count as f64) as f32
    } else {
        0.0
    };

    // Coefficients RBJ Audio EQ Cookbook pour un passe-haut Butterworth (Q = 1/√2).
    let w0 = std::f64::consts::TAU * f64::from(cutoff_hz) / f64::from(sample_rate);
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * std::f64::consts::FRAC_1_SQRT_2);

    let a0 = 1.0 + alpha;
    let b0 = ((1.0 + cos_w0) * 0.5) / a0;
    let b1 = -(1.0 + cos_w0) / a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;

    let mut out = Vec::with_capacity(samples.len());
    let mut z1 = 0.0f64;
    let mut z2 = 0.0f64;
    for &raw in samples {
        let x = if raw.is_finite() {
            f64::from(raw - dc)
        } else {
            0.0
        };
        let y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        out.push(y as f32);
    }
    out
}

/// Limiteur doux : linéaire jusqu'à [`LIMITER_KNEE`], puis compression douce
/// asymptotique vers [`LIMITER_CEIL`] pour éviter tout écrêtage sur les pics.
fn soft_limit(x: f32) -> f32 {
    let abs = x.abs();
    if abs <= LIMITER_KNEE {
        return x;
    }
    let headroom = LIMITER_CEIL - LIMITER_KNEE;
    let shaped = LIMITER_KNEE + headroom * ((abs - LIMITER_KNEE) / headroom).tanh();
    shaped.copysign(x)
}

/// Normalisation douce du niveau de parole (AGC) avec garde anti-bruit et
/// limiteur de pics.
///
/// - Si aucune trame de 30 ms ne dépasse le plancher vocal, le signal est
///   renvoyé tel quel (le silence ou le souffle n'est jamais amplifié).
/// - Le niveau de parole est estimé au 75e percentile des trames actives pour
///   ignorer à la fois les pauses et un clic isolé.
/// - Une voix déjà dans la zone confortable reste intacte.
pub fn normalize_speech_gain(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let frame = (sample_rate as usize * 30 / 1000).max(1);
    let mut active: Vec<f32> = samples
        .chunks(frame)
        .map(rms)
        .filter(|&level| level >= AGC_SPEECH_FLOOR_RMS)
        .collect();
    if active.is_empty() {
        return samples.to_vec();
    }
    active.sort_by(f32::total_cmp);
    let speech_rms = active[(active.len() - 1) * 3 / 4];
    if (AGC_DEADBAND_LOW..=AGC_DEADBAND_HIGH).contains(&speech_rms) {
        return samples.iter().map(|&s| soft_limit(s)).collect();
    }
    let gain = (AGC_TARGET_RMS / speech_rms).clamp(AGC_MIN_GAIN, AGC_MAX_GAIN);
    samples.iter().map(|&s| soft_limit(s * gain)).collect()
}

/// Prétraitement audio complet avant transcription locale (Whisper, Parakeet,
/// Vosk) : passe-haut 80 Hz puis normalisation douce du niveau de parole.
///
/// N'ajoute ni ne retire aucun échantillon : les horodatages mot par mot et
/// de réunion restent exacts.
pub fn preprocess_for_asr(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let filtered = highpass_filter(samples, sample_rate, HIGHPASS_CUTOFF_HZ);
    normalize_speech_gain(&filtered, sample_rate)
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

    fn sine(freq_hz: f32, amp: f32, rate: u32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (i as f32 * freq_hz * std::f32::consts::TAU / rate as f32).sin())
            .collect()
    }

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

    #[test]
    fn highpass_removes_dc_and_low_rumble_while_keeping_speech_band() {
        let rate = 16_000;
        // Décalage continu pur : complètement supprimé sans transitoire.
        let dc = vec![0.25f32; rate as usize];
        let dc_out = highpass_filter(&dc, rate, HIGHPASS_CUTOFF_HZ);
        assert!(rms(&dc_out) < 1e-6, "DC résiduel : {}", rms(&dc_out));

        // Grondement 30 Hz fortement atténué (~ -17 dB sur la partie établie).
        let rumble = sine(30.0, 0.2, rate, rate as usize);
        let rumble_out = highpass_filter(&rumble, rate, HIGHPASS_CUTOFF_HZ);
        let rumble_ratio = rms(&rumble_out[4000..]) / rms(&rumble[4000..]);
        assert!(rumble_ratio < 0.2, "atténuation 30 Hz : {rumble_ratio}");

        // Voix 440 Hz quasiment intacte (> 98 %).
        let voice = sine(440.0, 0.2, rate, rate as usize);
        let voice_out = highpass_filter(&voice, rate, HIGHPASS_CUTOFF_HZ);
        let voice_ratio = rms(&voice_out[4000..]) / rms(&voice[4000..]);
        assert!(voice_ratio > 0.98, "conservation 440 Hz : {voice_ratio}");
    }

    #[test]
    fn preprocess_boosts_quiet_speech_without_boosting_silence_or_clipping() {
        let rate = 16_000;
        // Bruit de fond très faible (RMS ~ 0,002) : ne doit jamais être amplifié.
        let hiss = sine(440.0, 0.003, rate, rate as usize);
        let hiss_out = preprocess_for_asr(&hiss, rate);
        assert!(
            rms(&hiss_out) < 0.004,
            "bruit amplifié à tort : {}",
            rms(&hiss_out)
        );

        // Micro faible (parole à 0,02 d'amplitude → ~0,014 RMS) + un clic isolé fort :
        // la parole est remontée vers ~0,09 RMS et le clic reste doux sous 0,98.
        let mut quiet = sine(440.0, 0.02, rate, rate as usize);
        quiet[4000] = 0.9;
        let out = preprocess_for_asr(&quiet, rate);
        assert_eq!(out.len(), quiet.len());
        let boosted_rms = rms(&out[6000..]);
        assert!(
            boosted_rms > 0.07 && boosted_rms < 0.11,
            "RMS attendu proche de 0,09, obtenu {boosted_rms}"
        );
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= LIMITER_CEIL + 1e-4, "écrêtage dépassé : {peak}");

        // Voix déjà bien calibrée (0,14 amplitude ≈ 0,099 RMS) : inchangée.
        let good = sine(440.0, 0.14, rate, rate as usize);
        let good_out = preprocess_for_asr(&good, rate);
        let ratio = rms(&good_out[4000..]) / rms(&good[4000..]);
        assert!(
            (ratio - 1.0).abs() < 0.03,
            "zone morte non respectée : {ratio}"
        );
    }

    #[test]
    fn preprocess_sanitizes_non_finite_samples() {
        let mut clip = sine(440.0, 0.1, 16_000, 1600);
        clip[10] = f32::NAN;
        clip[20] = f32::INFINITY;
        let out = preprocess_for_asr(&clip, 16_000);
        assert_eq!(out.len(), clip.len());
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
