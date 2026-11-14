//! Lecture des sons courts (début, fin, annulation de dictée) sur la sortie
//! audio par défaut, via cpal (ALSA, donc PipeWire ou PulseAudio sous Linux).
//!
//! Un thread dédié garde le flux de sortie (`cpal::Stream` ne se partage pas
//! entre threads) et le ferme dès la fin du son : la carte son n'est pas
//! tenue éveillée entre deux dictées (batterie du Steam Deck).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// Marge après la fin du son avant de fermer le flux (tampons de la carte).
const TAIL: Duration = Duration::from_millis(250);

/// Joue un son WAV en mémoire sans attendre la fin (le son précédent est
/// interrompu).
pub fn play_sound(wav: &'static [u8]) {
    static PLAYER: OnceLock<Option<Sender<&'static [u8]>>> = OnceLock::new();
    let player = PLAYER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("sound".into())
            .spawn(move || run(rx))
            .map_err(|e| log::warn!("thread des sons : {e}"))
            .ok()?;
        Some(tx)
    });
    if let Some(player) = player {
        let _ = player.send(wav);
    }
}

fn run(rx: Receiver<&'static [u8]>) {
    let mut active: Option<(cpal::Stream, Instant)> = None;
    loop {
        let received = match &active {
            Some((_, end)) => rx.recv_timeout(end.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(wav) => {
                // Le son précédent s'arrête net.
                active = None;
                match start(wav) {
                    Ok(playing) => active = Some(playing),
                    Err(e) => log::warn!("lecture du son impossible : {e}"),
                }
            }
            Err(RecvTimeoutError::Timeout) => active = None,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn start(wav: &[u8]) -> Result<(cpal::Stream, Instant), String> {
    let (rate, mono) = decode_wav(wav).ok_or("WAV non pris en charge")?;
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("aucune sortie audio")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("configuration de la sortie : {e}"))?;
    let config = supported.config();
    let samples = vtt_core::audio::resample(&mono, rate, config.sample_rate);
    let duration = Duration::from_secs_f64(samples.len() as f64 / config.sample_rate as f64);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, samples),
        SampleFormat::I16 => build::<i16>(&device, config, samples),
        SampleFormat::I32 => build::<i32>(&device, config, samples),
        SampleFormat::U16 => build::<u16>(&device, config, samples),
        SampleFormat::U8 => build::<u8>(&device, config, samples),
        other => return Err(format!("format audio non géré : {other}")),
    }
    .map_err(|e| format!("ouverture de la sortie : {e}"))?;
    stream.play().map_err(|e| format!("lecture : {e}"))?;
    Ok((stream, Instant::now() + duration + TAIL))
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    samples: Vec<f32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let samples = Arc::new(samples);
    let mut position = 0usize;
    device.build_output_stream::<T, _, _>(
        config,
        move |out: &mut [T], _| {
            for frame in out.chunks_mut(channels.max(1)) {
                let value = samples.get(position).copied().unwrap_or(0.0);
                position += 1;
                frame.fill(T::from_sample(value));
            }
        },
        |err| log::warn!("flux de sortie : {err}"),
        None,
    )
}

/// WAV PCM 16 bits (mono ou stéréo) → fréquence et échantillons mono.
fn decode_wav(wav: &[u8]) -> Option<(u32, Vec<f32>)> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut at = 12;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().ok()?) as usize;
        let body = wav.get(at + 8..(at + 8).checked_add(len)?.min(wav.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                format = Some((u16_at(0), u16_at(2), rate, u16_at(14)));
            }
            b"data" => {
                let (kind, channels, rate, bits) = format?;
                if kind != 1 || bits != 16 || channels == 0 || rate == 0 {
                    return None;
                }
                let frame = 2 * channels as usize;
                let mono = body
                    .chunks_exact(frame)
                    .map(|f| {
                        let sum: f32 = f
                            .chunks_exact(2)
                            .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                            .sum();
                        sum / channels as f32
                    })
                    .collect();
                return Some((rate, mono));
            }
            _ => {}
        }
        // Les blocs sont alignés sur 2 octets.
        at += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_generated_tones() {
        let wav = vtt_core::audio::tone_wav(&[(440.0, 100)], 0.5);
        let (rate, samples) = decode_wav(&wav).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(samples.len(), 2205);
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!((0.45..=0.51).contains(&peak), "{peak}");
    }

    #[test]
    fn rejects_other_files() {
        assert!(decode_wav(b"").is_none());
        assert!(decode_wav(b"RIFF\0\0\0\0WAVEdata\0\0\0\0").is_none());
        assert!(decode_wav(&[0u8; 64]).is_none());
    }
}
