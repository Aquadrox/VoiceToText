//! Capture du micro (celui choisi dans les réglages, sinon celui par défaut).
//!
//! Le flux n'est ouvert que pendant l'enregistrement (pas d'icône « micro
//! utilisé » en permanence). Il vit dans un thread dédié car `cpal::Stream`
//! n'est pas transférable entre threads sur toutes les plateformes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use vtt_core::{AudioClip, SAMPLE_RATE, audio};

enum Command {
    Start(Sender<Result<(), String>>),
    /// Lit les nouveaux échantillons captés depuis le dernier appel, en 16 kHz
    /// mono, sans vider le tampon complet (conservé pour `Stop` / « Réessayer »).
    DrainLive(Sender<Vec<f32>>),
    Stop(Sender<AudioClip>),
    /// Ferme le micro et renvoie l'audio complet ainsi que les derniers
    /// échantillons non encore lus par `DrainLive` (16 kHz mono).
    StopWithTail(Sender<(AudioClip, Vec<f32>)>),
    /// Micro à utiliser à partir du prochain enregistrement.
    SetDevice(Option<String>),
}

/// Pilote le thread audio ; clonable et utilisable depuis n'importe quel thread.
#[derive(Clone)]
pub struct Recorder {
    tx: Sender<Command>,
    /// Niveau du dernier bloc capté (f32 entre 0 et 1, stocké en bits).
    level: Arc<AtomicU32>,
}

impl Recorder {
    /// `device` : nom du micro (voir [`input_devices`]) ; `None` ou introuvable :
    /// micro par défaut de Windows.
    pub fn spawn(device: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel();
        let level = Arc::new(AtomicU32::new(0));
        let shared = Arc::clone(&level);
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, device, shared))
            .expect("création du thread audio");
        Self { tx, level }
    }

    /// Niveau du micro entre 0 et 1, pour la pastille (0 hors enregistrement).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Ouvre le micro et commence à accumuler les échantillons.
    pub fn start(&self) -> Result<(), String> {
        let (reply, response) = mpsc::channel();
        self.tx
            .send(Command::Start(reply))
            .map_err(|_| "thread audio arrêté".to_owned())?;
        response
            .recv()
            .map_err(|_| "thread audio arrêté".to_owned())?
    }

    /// Renvoie les nouveaux échantillons captés depuis le précédent appel
    /// (16 kHz mono), sans altérer le tampon complet. Vide hors enregistrement.
    pub fn drain_live(&self) -> Vec<f32> {
        let (reply, response) = mpsc::channel();
        if self.tx.send(Command::DrainLive(reply)).is_err() {
            return Vec::new();
        }
        response.recv().unwrap_or_default()
    }

    /// Ferme le micro et renvoie l'audio en 16 kHz mono (vide si rien n'était enregistré).
    pub fn stop(&self) -> AudioClip {
        let (reply, response) = mpsc::channel();
        if self.tx.send(Command::Stop(reply)).is_err() {
            return AudioClip::default();
        }
        response.recv().unwrap_or_default()
    }

    /// Ferme le micro et renvoie `(audio_complet, reliquat_non_lu)` en 16 kHz
    /// mono, pour la dictée mains libres en continu.
    pub fn stop_with_tail(&self) -> (AudioClip, Vec<f32>) {
        let (reply, response) = mpsc::channel();
        if self.tx.send(Command::StopWithTail(reply)).is_err() {
            return (AudioClip::default(), Vec::new());
        }
        response.recv().unwrap_or_default()
    }

    /// Change de micro ; pris en compte au prochain enregistrement.
    pub fn set_device(&self, device: Option<String>) {
        let _ = self.tx.send(Command::SetDevice(device));
    }
}

struct Active {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    sample_rate: u32,
    /// Position (en échantillons entrelacés) déjà lue par [`Recorder::drain_live`].
    live_cursor: usize,
}

/// Noms des micros disponibles, pour le choix dans les réglages.
pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

fn run(rx: Receiver<Command>, mut device: Option<String>, level: Arc<AtomicU32>) {
    let mut active: Option<Active> = None;
    for command in rx {
        match command {
            Command::Start(reply) => {
                let result = if active.is_some() {
                    Ok(())
                } else {
                    open(device.as_deref(), &level).map(|a| active = Some(a))
                };
                let _ = reply.send(result);
            }
            Command::DrainLive(reply) => {
                let samples = active.as_mut().map(drain_live).unwrap_or_default();
                let _ = reply.send(samples);
            }
            Command::Stop(reply) => {
                let clip = active.take().map(finish).unwrap_or_default();
                level.store(0f32.to_bits(), Ordering::Relaxed);
                let _ = reply.send(clip);
            }
            Command::StopWithTail(reply) => {
                let pair = active.take().map(finish_with_tail).unwrap_or_default();
                level.store(0f32.to_bits(), Ordering::Relaxed);
                let _ = reply.send(pair);
            }
            Command::SetDevice(name) => device = name,
        }
    }
}

fn find_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut devices| devices.find(|d| d.to_string() == name));
        if found.is_some() {
            return found;
        }
        log::warn!("micro « {name} » introuvable : micro par défaut utilisé");
    }
    host.default_input_device()
}

fn open(name: Option<&str>, level: &Arc<AtomicU32>) -> Result<Active, String> {
    let device = find_device(name).ok_or("aucun micro détecté")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("configuration du micro : {e}"))?;
    let config = supported.config();
    let buffer = Arc::new(Mutex::new(Vec::with_capacity(
        config.sample_rate as usize * config.channels as usize * 30,
    )));

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, &buffer, level),
        SampleFormat::I16 => build::<i16>(&device, config, &buffer, level),
        SampleFormat::I32 => build::<i32>(&device, config, &buffer, level),
        SampleFormat::U16 => build::<u16>(&device, config, &buffer, level),
        SampleFormat::I8 => build::<i8>(&device, config, &buffer, level),
        SampleFormat::U8 => build::<u8>(&device, config, &buffer, level),
        other => return Err(format!("format audio non géré : {other}")),
    }
    .map_err(|e| format!("ouverture du micro : {e}"))?;
    stream
        .play()
        .map_err(|e| format!("démarrage du micro : {e}"))?;

    Ok(Active {
        _stream: stream,
        buffer,
        channels: config.channels as usize,
        sample_rate: config.sample_rate,
        live_cursor: 0,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    buffer: &Arc<Mutex<Vec<f32>>>,
    level: &Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    let level = Arc::clone(level);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            if let Ok(mut buf) = buffer.lock() {
                let start = buf.len();
                buf.extend(data.iter().map(|&s| f32::from_sample(s)));
                let value = audio::meter_level(audio::rms(&buf[start..]));
                level.store(value.to_bits(), Ordering::Relaxed);
            }
        },
        |err| log::warn!("flux micro : {err}"),
        None,
    )
}

/// Convertit la tranche `[cursor..]` (alignée sur des trames complètes) en
/// 16 kHz mono et renvoie `(échantillons, nouveau_curseur)`.
fn slice_to_16k(
    interleaved: &[f32],
    cursor: usize,
    channels: usize,
    sample_rate: u32,
) -> (Vec<f32>, usize) {
    let channels = channels.max(1);
    let end = interleaved.len() - (interleaved.len() % channels);
    if cursor >= end {
        return (Vec::new(), cursor);
    }
    let mono = audio::downmix(&interleaved[cursor..end], channels);
    (audio::resample(&mono, sample_rate, SAMPLE_RATE), end)
}

fn drain_live(active: &mut Active) -> Vec<f32> {
    let channels = active.channels.max(1);
    let slice = {
        let buf = active.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let end = buf.len() - (buf.len() % channels);
        if active.live_cursor >= end {
            return Vec::new();
        }
        let copy = buf[active.live_cursor..end].to_vec();
        active.live_cursor = end;
        copy
    };
    let mono = audio::downmix(&slice, channels);
    audio::resample(&mono, active.sample_rate, SAMPLE_RATE)
}

fn finish(active: Active) -> AudioClip {
    finish_with_tail(active).0
}

fn finish_with_tail(active: Active) -> (AudioClip, Vec<f32>) {
    // Fermer le flux d'abord pour que le callback ne touche plus au tampon.
    let Active {
        _stream,
        buffer,
        channels,
        sample_rate,
        live_cursor,
    } = active;
    drop(_stream);
    let interleaved = std::mem::take(&mut *buffer.lock().unwrap_or_else(|e| e.into_inner()));
    let (tail, _) = slice_to_16k(&interleaved, live_cursor, channels, sample_rate);
    let (full, _) = slice_to_16k(&interleaved, 0, channels, sample_rate);
    (AudioClip { samples: full }, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_to_16k_drains_incrementally_on_frame_boundaries() {
        // Stéréo 48 kHz : rapport exact 3:1 vers 16 kHz.
        let mut buf: Vec<f32> = Vec::new();
        for i in 0..30 {
            let v = i as f32 / 30.0;
            buf.push(v);
            buf.push(v);
        }
        // Trame incomplète en fin de tampon : ignorée jusqu'à complétion.
        buf.push(0.9);
        let (first, cursor) = slice_to_16k(&buf, 0, 2, 48_000);
        assert_eq!(cursor, 60);
        assert_eq!(first.len(), 10);

        // Complète la trame + ajoute 29 trames (total 30 nouvelles trames).
        buf.push(0.9);
        for _ in 0..29 {
            buf.push(0.5);
            buf.push(0.5);
        }
        let (second, cursor2) = slice_to_16k(&buf, cursor, 2, 48_000);
        assert_eq!(cursor2, 120);
        assert_eq!(second.len(), 10);

        // Rien de neuf : tranche vide, curseur inchangé.
        let (empty, cursor3) = slice_to_16k(&buf, cursor2, 2, 48_000);
        assert!(empty.is_empty());
        assert_eq!(cursor3, cursor2);
    }
}
