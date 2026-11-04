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
    Stop(Sender<AudioClip>),
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

    /// Ferme le micro et renvoie l'audio en 16 kHz mono (vide si rien n'était enregistré).
    pub fn stop(&self) -> AudioClip {
        let (reply, response) = mpsc::channel();
        if self.tx.send(Command::Stop(reply)).is_err() {
            return AudioClip::default();
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
            Command::Stop(reply) => {
                let clip = active.take().map(finish).unwrap_or_default();
                level.store(0f32.to_bits(), Ordering::Relaxed);
                let _ = reply.send(clip);
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

fn finish(active: Active) -> AudioClip {
    // Fermer le flux d'abord pour que le callback ne touche plus au tampon.
    let Active {
        _stream,
        buffer,
        channels,
        sample_rate,
    } = active;
    drop(_stream);
    let interleaved = std::mem::take(&mut *buffer.lock().unwrap_or_else(|e| e.into_inner()));
    let mono = audio::downmix(&interleaved, channels);
    AudioClip {
        samples: audio::resample(&mono, sample_rate, SAMPLE_RATE),
    }
}
