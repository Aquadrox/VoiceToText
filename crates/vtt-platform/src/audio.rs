//! Capture du micro par défaut.
//!
//! Le flux n'est ouvert que pendant l'enregistrement (pas d'icône « micro
//! utilisé » en permanence). Il vit dans un thread dédié car `cpal::Stream`
//! n'est pas transférable entre threads sur toutes les plateformes.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use vtt_core::{AudioClip, SAMPLE_RATE, audio};

enum Command {
    Start(Sender<Result<(), String>>),
    Stop(Sender<AudioClip>),
}

/// Pilote le thread audio ; clonable et utilisable depuis n'importe quel thread.
#[derive(Clone)]
pub struct Recorder {
    tx: Sender<Command>,
}

impl Recorder {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx))
            .expect("création du thread audio");
        Self { tx }
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
}

struct Active {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    sample_rate: u32,
}

fn run(rx: Receiver<Command>) {
    let mut active: Option<Active> = None;
    for command in rx {
        match command {
            Command::Start(reply) => {
                let result = if active.is_some() {
                    Ok(())
                } else {
                    open().map(|a| active = Some(a))
                };
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let clip = active.take().map(finish).unwrap_or_default();
                let _ = reply.send(clip);
            }
        }
    }
}

fn open() -> Result<Active, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("aucun micro détecté")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("configuration du micro : {e}"))?;
    let config = supported.config();
    let buffer = Arc::new(Mutex::new(Vec::with_capacity(
        config.sample_rate as usize * config.channels as usize * 30,
    )));

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, &buffer),
        SampleFormat::I16 => build::<i16>(&device, config, &buffer),
        SampleFormat::I32 => build::<i32>(&device, config, &buffer),
        SampleFormat::U16 => build::<u16>(&device, config, &buffer),
        SampleFormat::I8 => build::<i8>(&device, config, &buffer),
        SampleFormat::U8 => build::<u8>(&device, config, &buffer),
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
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            if let Ok(mut buf) = buffer.lock() {
                buf.extend(data.iter().map(|&s| f32::from_sample(s)));
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
