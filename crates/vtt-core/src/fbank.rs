//! Bancs de filtres mel « façon Kaldi » (fbank) : l'entrée qu'attendent les
//! modèles d'empreinte vocale (WeSpeaker, 3D-Speaker).
//!
//! Mêmes réglages que Kaldi / `kaldi-native-fbank` par défaut : trames de
//! 25 ms toutes les 10 ms, sans bord rembourré, fenêtre de Povey (ou de
//! Hamming, celle de l'entraînement de WeSpeaker), pré-accentuation 0,97,
//! 80 filtres de 20 Hz à 8 kHz, logarithme de l'énergie. Résultats vérifiés
//! contre `kaldi-native-fbank` (écart < 10⁻⁴). Pas de dépendance : la FFT
//! (512 points) est écrite ici.

use std::f32::consts::PI;

use crate::pipeline::SAMPLE_RATE;

/// Nombre de filtres mel (dimension des caractéristiques).
pub const MEL_BINS: usize = 80;
/// Longueur d'une trame : 25 ms.
const FRAME_LENGTH: usize = SAMPLE_RATE as usize / 40;
/// Décalage entre deux trames : 10 ms.
const FRAME_SHIFT: usize = SAMPLE_RATE as usize / 100;
/// Trame complétée par des zéros jusqu'à une puissance de deux.
const FFT_SIZE: usize = 512;
const PREEMPHASIS: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;

/// Échelle mel de Kaldi.
fn mel(freq: f32) -> f32 {
    1127.0 * (1.0 + freq / 700.0).ln()
}

/// Fenêtre appliquée à chaque trame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Kaldi par défaut (3D-Speaker).
    Povey,
    /// WeSpeaker.
    Hamming,
}

/// Calcule les caractéristiques fbank d'un signal 16 kHz mono.
pub struct Fbank {
    window: Vec<f32>,
    /// Pour chaque filtre : premier indice de fréquence et poids.
    banks: Vec<(usize, Vec<f32>)>,
    fft: Fft,
}

impl Fbank {
    pub fn new(window: Window) -> Self {
        let window = (0..FRAME_LENGTH)
            .map(|i| {
                let cos = (2.0 * PI * i as f32 / (FRAME_LENGTH - 1) as f32).cos();
                match window {
                    // Un Hann élevé à la puissance 0,85.
                    Window::Povey => (0.5 - 0.5 * cos).powf(0.85),
                    Window::Hamming => 0.54 - 0.46 * cos,
                }
            })
            .collect();

        let fft_bins = FFT_SIZE / 2;
        let bin_width = SAMPLE_RATE as f32 / FFT_SIZE as f32;
        let mel_low = mel(LOW_FREQ);
        let mel_high = mel(SAMPLE_RATE as f32 / 2.0);
        let delta = (mel_high - mel_low) / (MEL_BINS + 1) as f32;
        let banks = (0..MEL_BINS)
            .map(|bin| {
                let left = mel_low + bin as f32 * delta;
                let center = left + delta;
                let right = center + delta;
                let mut first = None;
                let mut weights = Vec::new();
                for i in 0..fft_bins {
                    let m = mel(bin_width * i as f32);
                    if m > left && m < right {
                        first.get_or_insert(i);
                        weights.push(if m <= center {
                            (m - left) / (center - left)
                        } else {
                            (right - m) / (right - center)
                        });
                    }
                }
                (first.unwrap_or(0), weights)
            })
            .collect();

        Self {
            window,
            banks,
            fft: Fft::new(FFT_SIZE),
        }
    }

    /// Une ligne de [`MEL_BINS`] valeurs par trame de 10 ms. `scale`
    /// multiplie d'abord les échantillons (32768 pour les modèles entraînés
    /// sur des entiers 16 bits, 1 sinon).
    pub fn compute(&self, samples: &[f32], scale: f32) -> Vec<[f32; MEL_BINS]> {
        if samples.len() < FRAME_LENGTH {
            return Vec::new();
        }
        let frames = 1 + (samples.len() - FRAME_LENGTH) / FRAME_SHIFT;
        let mut re = vec![0.0f32; FFT_SIZE];
        let mut im = vec![0.0f32; FFT_SIZE];
        let mut power = vec![0.0f32; FFT_SIZE / 2 + 1];
        let mut out = Vec::with_capacity(frames);
        for f in 0..frames {
            let frame = &samples[f * FRAME_SHIFT..f * FRAME_SHIFT + FRAME_LENGTH];
            let mean = frame.iter().sum::<f32>() * scale / FRAME_LENGTH as f32;
            for (r, &s) in re.iter_mut().zip(frame) {
                *r = s * scale - mean;
            }
            for i in (1..FRAME_LENGTH).rev() {
                re[i] -= PREEMPHASIS * re[i - 1];
            }
            re[0] -= PREEMPHASIS * re[0];
            for (r, w) in re.iter_mut().zip(&self.window) {
                *r *= w;
            }
            re[FRAME_LENGTH..].fill(0.0);
            im.fill(0.0);
            self.fft.run(&mut re, &mut im);
            for (k, p) in power.iter_mut().enumerate() {
                *p = re[k] * re[k] + im[k] * im[k];
            }
            let mut row = [0.0f32; MEL_BINS];
            for (value, (first, weights)) in row.iter_mut().zip(&self.banks) {
                let energy: f32 = weights
                    .iter()
                    .zip(&power[*first..])
                    .map(|(w, p)| w * p)
                    .sum();
                *value = energy.max(f32::EPSILON).ln();
            }
            out.push(row);
        }
        out
    }
}

/// Retire la moyenne de chaque dimension (normalisation par énoncé).
pub fn subtract_mean(features: &mut [[f32; MEL_BINS]]) {
    if features.is_empty() {
        return;
    }
    let mut mean = [0.0f32; MEL_BINS];
    for row in features.iter() {
        for (m, v) in mean.iter_mut().zip(row) {
            *m += v;
        }
    }
    let n = features.len() as f32;
    for row in features.iter_mut() {
        for (v, m) in row.iter_mut().zip(&mean) {
            *v -= m / n;
        }
    }
}

/// FFT complexe en place, radix 2 (taille : puissance de deux).
struct Fft {
    cos: Vec<f32>,
    sin: Vec<f32>,
    reversed: Vec<usize>,
}

impl Fft {
    fn new(size: usize) -> Self {
        assert!(size.is_power_of_two());
        let bits = size.trailing_zeros();
        let reversed = (0..size)
            .map(|i| i.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let (cos, sin) = (0..size / 2)
            .map(|k| {
                let angle = -2.0 * std::f64::consts::PI * k as f64 / size as f64;
                (angle.cos() as f32, angle.sin() as f32)
            })
            .unzip();
        Self { cos, sin, reversed }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        for i in 0..n {
            let j = self.reversed[i];
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let stride = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * stride], self.sin[k * stride]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_the_direct_transform() {
        let n = 16;
        let fft = Fft::new(n);
        let signal: Vec<f32> = (0..n).map(|i| ((i * 7 % 5) as f32) - 2.0).collect();
        let (mut re, mut im) = (signal.clone(), vec![0.0; n]);
        fft.run(&mut re, &mut im);
        for k in 0..n {
            let (mut er, mut ei) = (0.0f64, 0.0f64);
            for (t, &x) in signal.iter().enumerate() {
                let angle = -2.0 * std::f64::consts::PI * (k * t) as f64 / n as f64;
                er += x as f64 * angle.cos();
                ei += x as f64 * angle.sin();
            }
            assert!((re[k] as f64 - er).abs() < 1e-4, "re[{k}]");
            assert!((im[k] as f64 - ei).abs() < 1e-4, "im[{k}]");
        }
    }

    #[test]
    fn frames_every_ten_milliseconds_without_padding() {
        let fbank = Fbank::new(Window::Povey);
        assert!(fbank.compute(&[0.0; 399], 1.0).is_empty());
        assert_eq!(fbank.compute(&[0.0; 400], 1.0).len(), 1);
        // 1 s : 1 + (16000 - 400) / 160 = 98 trames.
        assert_eq!(fbank.compute(&[0.0; 16_000], 1.0).len(), 98);
    }

    #[test]
    fn a_tone_lights_up_the_filter_of_its_frequency() {
        let tone: Vec<f32> = (0..16_000)
            .map(|i| (2.0 * PI * 1000.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        let features = Fbank::new(Window::Hamming).compute(&tone, 32768.0);
        let row = features[10];
        let loudest = (0..MEL_BINS)
            .max_by(|&a, &b| row[a].total_cmp(&row[b]))
            .unwrap();
        // 1 kHz ≈ 1000 mel ; filtres larges de 34,7 mel à partir de 31,7 mel
        // (20 Hz) : centre du filtre 27 à 1002 mel.
        assert_eq!(loudest, 27);
    }

    #[test]
    fn mean_subtraction_centers_each_dimension() {
        let mut features = vec![[1.0; MEL_BINS], [3.0; MEL_BINS]];
        subtract_mean(&mut features);
        assert_eq!(features[0][0], -1.0);
        assert_eq!(features[1][79], 1.0);
    }
}
