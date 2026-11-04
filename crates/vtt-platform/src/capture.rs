//! Capture d'une réunion : le micro et le son du PC (toutes les sorties
//! audio, en « loopback » WASAPI), en deux pistes 16 kHz mono alignées sur
//! l'horloge.
//!
//! Le son du PC n'arrive que quand quelque chose joue : les trous sont
//! comblés par du silence, pour que les deux pistes restent synchrones.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use vtt_core::{SAMPLE_RATE, audio};

/// Marge laissée aux pilotes pour livrer leurs derniers échantillons : on
/// ne lit que jusqu'à « maintenant moins 300 ms ».
const LATENCY: Duration = Duration::from_millis(300);

/// Audio brut d'une source, et ce qui en a déjà été rendu.
struct Source {
    name: String,
    raw: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    rate: u32,
    /// Converti en 16 kHz, pas encore rendu.
    pending: Vec<f32>,
    /// Échantillons 16 kHz déjà rendus (silence ajouté compris).
    emitted: u64,
}

impl Source {
    /// Les `wanted - emitted` échantillons suivants : silence d'abord si la
    /// source n'a rien livré pendant un moment, surplus gardé pour la suite.
    fn drain(&mut self, wanted: u64) -> Vec<f32> {
        let interleaved = std::mem::take(&mut *self.raw.lock().unwrap_or_else(|e| e.into_inner()));
        if !interleaved.is_empty() {
            let mono = audio::downmix(&interleaved, self.channels);
            self.pending
                .extend(audio::resample(&mono, self.rate, SAMPLE_RATE));
        }
        let need = wanted.saturating_sub(self.emitted) as usize;
        self.emitted += need as u64;
        if self.pending.len() >= need {
            return self.pending.drain(..need).collect();
        }
        let mut out = vec![0.0; need - self.pending.len()];
        out.append(&mut self.pending);
        out
    }
}

/// Audio rendu par [`Capture::take`] : même durée pour les deux pistes.
#[derive(Debug, Default)]
pub struct Tracks {
    /// Micro (l'utilisateur).
    pub mic: Vec<f32>,
    /// Son du PC (les autres), toutes sorties mélangées ; vide si non capté.
    pub system: Vec<f32>,
}

struct Shared {
    started: Instant,
    mic: Source,
    system: Vec<Source>,
}

/// Enregistrement en cours. Les flux vivent dans un thread dédié
/// (`cpal::Stream` n'est pas transférable entre threads partout).
pub struct Capture {
    shared: Mutex<Shared>,
    stop: mpsc::Sender<()>,
    /// Sorties dont le son est capté (pour le journal et l'interface).
    pub outputs: Vec<String>,
}

impl Capture {
    /// Ouvre le micro (`None` : celui par défaut) et, si `system`, le son de
    /// chaque sortie audio active. Échoue seulement si le micro est
    /// inutilisable ; une sortie qui refuse est ignorée.
    pub fn start(mic: Option<&str>, system: bool) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let mic = mic.map(str::to_owned);
        thread::Builder::new()
            .name("meeting-capture".into())
            .spawn(move || {
                let opened = open_all(mic.as_deref(), system);
                let (streams, sources) = match opened {
                    Ok(opened) => opened,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if ready_tx.send(Ok(sources)).is_err() {
                    return;
                }
                // Les flux restent ouverts jusqu'à l'arrêt.
                let _ = stop_rx.recv();
                drop(streams);
            })
            .map_err(|e| format!("thread de capture : {e}"))?;
        let (mic, system) = ready_rx
            .recv()
            .map_err(|_| "capture interrompue".to_owned())??;
        let outputs = system.iter().map(|s| s.name.clone()).collect();
        Ok(Self {
            shared: Mutex::new(Shared {
                started: Instant::now(),
                mic,
                system,
            }),
            stop: stop_tx,
            outputs,
        })
    }

    /// Audio arrivé depuis le dernier appel, jusqu'à « maintenant moins la
    /// latence ».
    pub fn take(&self) -> Tracks {
        self.take_until(LATENCY)
    }

    /// Arrête la capture ; renvoie la fin de l'audio.
    pub fn stop(self) -> Tracks {
        // Laisser les pilotes livrer leurs derniers blocs.
        thread::sleep(LATENCY);
        let tracks = self.take_until(LATENCY);
        let _ = self.stop.send(());
        tracks
    }

    fn take_until(&self, margin: Duration) -> Tracks {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = shared.started.elapsed().saturating_sub(margin);
        let wanted = elapsed.as_millis() as u64 * u64::from(SAMPLE_RATE) / 1000;
        let mic = shared.mic.drain(wanted);
        let mut system: Vec<f32> = Vec::new();
        for source in &mut shared.system {
            let samples = source.drain(wanted);
            if system.is_empty() {
                system = samples;
            } else {
                for (mixed, s) in system.iter_mut().zip(samples) {
                    *mixed = (*mixed + s).clamp(-1.0, 1.0);
                }
            }
        }
        Tracks { mic, system }
    }
}

type Opened = (Vec<cpal::Stream>, (Source, Vec<Source>));

fn open_all(mic: Option<&str>, system: bool) -> Result<Opened, String> {
    let host = cpal::default_host();
    let device = mic
        .and_then(|name| {
            let found = host
                .input_devices()
                .ok()
                .and_then(|mut devices| devices.find(|d| d.to_string() == name));
            if found.is_none() {
                log::warn!("micro « {name} » introuvable : micro par défaut utilisé");
            }
            found
        })
        .or_else(|| host.default_input_device())
        .ok_or("aucun micro détecté")?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("configuration du micro : {e}"))?;
    let (mic_stream, mic_source) = open(&device, config.sample_format(), config.config())
        .map_err(|e| format!("ouverture du micro : {e}"))?;
    let mut streams = vec![mic_stream];
    let mut outputs = Vec::new();
    if system {
        for device in host.output_devices().into_iter().flatten() {
            let name = device.to_string();
            // Un flux d'entrée sur une sortie = loopback (WASAPI).
            let opened = device
                .default_output_config()
                .map_err(|e| e.to_string())
                .and_then(|c| open(&device, c.sample_format(), c.config()));
            match opened {
                Ok((stream, source)) => {
                    streams.push(stream);
                    outputs.push(source);
                }
                Err(e) => log::info!("son de « {name} » non capté : {e}"),
            }
        }
        if outputs.is_empty() {
            log::warn!("aucune sortie audio captée : seul le micro sera transcrit");
        }
    }
    Ok((streams, (mic_source, outputs)))
}

fn open(
    device: &cpal::Device,
    format: SampleFormat,
    config: StreamConfig,
) -> Result<(cpal::Stream, Source), String> {
    let raw = Arc::new(Mutex::new(Vec::new()));
    let stream = match format {
        SampleFormat::F32 => build::<f32>(device, config, &raw),
        SampleFormat::I16 => build::<i16>(device, config, &raw),
        SampleFormat::I32 => build::<i32>(device, config, &raw),
        SampleFormat::U16 => build::<u16>(device, config, &raw),
        SampleFormat::I8 => build::<i8>(device, config, &raw),
        SampleFormat::U8 => build::<u8>(device, config, &raw),
        other => return Err(format!("format audio non géré : {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    let source = Source {
        name: device.to_string(),
        raw,
        channels: config.channels as usize,
        rate: config.sample_rate,
        pending: Vec::new(),
        emitted: 0,
    };
    Ok((stream, source))
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    raw: &Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let raw = Arc::clone(raw);
    let name = device.to_string();
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            if let Ok(mut buf) = raw.lock() {
                buf.extend(data.iter().map(|&s| f32::from_sample(s)));
            }
        },
        move |err| log::warn!("flux « {name} » : {err}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(rate: u32, channels: usize) -> Source {
        Source {
            name: "test".into(),
            raw: Arc::new(Mutex::new(Vec::new())),
            channels,
            rate,
            pending: Vec::new(),
            emitted: 0,
        }
    }

    #[test]
    fn a_silent_output_is_padded_to_stay_in_sync() {
        let mut output = source(48_000, 2);
        // Rien ne joue pendant la première seconde.
        assert_eq!(output.drain(16_000), vec![0.0; 16_000]);
        // Puis 0,5 s de son livré : le trou d'avant est comblé, le son est à la fin.
        output.raw.lock().unwrap().extend(vec![0.5; 48_000]);
        let next = output.drain(32_000);
        assert_eq!(next.len(), 16_000);
        assert_eq!(next[0], 0.0);
        assert!((next[15_999] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn early_samples_wait_for_their_turn() {
        let mut mic = source(16_000, 1);
        mic.raw.lock().unwrap().extend(vec![0.1; 20_000]);
        assert_eq!(mic.drain(16_000).len(), 16_000);
        assert_eq!(mic.pending.len(), 4_000, "surplus gardé");
        assert_eq!(mic.drain(20_000), vec![0.1; 4_000]);
    }
}
