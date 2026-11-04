//! Fait passer un fichier WAV par le pipeline complet, sans micro ni clavier.
//!
//! cargo run --release -p vtt-asr --example transcribe_wav -- <modèle.bin> <fichier.wav>
//!
//! Parakeet et Vosk : dossier du modèle et bibliothèque native (ONNX Runtime,
//! libvosk) en plus :
//!
//! … -- parakeet <dossier> <fichier.wav> <libonnxruntime.so>
//! … -- vosk <dossier> <fichier.wav> <libvosk.so>

use std::time::Instant;

use vtt_asr::{ParakeetTranscriber, VoskTranscriber, WhisperTranscriber};
use vtt_core::{AudioClip, Error, Pipeline, SAMPLE_RATE, TextSink, Transcriber, audio};

struct Stdout;

impl TextSink for Stdout {
    fn insert(&self, text: &str) -> Result<(), Error> {
        println!("{text}");
        Ok(())
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let engine = match args.first().map(String::as_str) {
        Some(name @ ("parakeet" | "vosk")) => {
            let name = name.to_owned();
            args.remove(0);
            name
        }
        _ => "whisper".to_owned(),
    };
    let (Some(model), Some(wav)) = (args.first().cloned(), args.get(1).cloned()) else {
        eprintln!("usage : transcribe_wav [parakeet|vosk] <modèle> <fichier.wav> [bibliothèque]");
        std::process::exit(2);
    };
    let library = args.get(2).cloned().unwrap_or_default();

    let mut reader = hound::WavReader::open(&wav).expect("lecture du WAV");
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / scale)
                .collect()
        }
    };
    let mono = audio::downmix(&interleaved, spec.channels as usize);
    let clip = AudioClip {
        samples: audio::resample(&mono, spec.sample_rate, SAMPLE_RATE),
    };

    let started = Instant::now();
    let transcriber: Box<dyn Transcriber> = match engine.as_str() {
        "parakeet" => Box::new(
            ParakeetTranscriber::load(model.as_ref(), library.as_ref()).expect("chargement"),
        ),
        "vosk" => {
            Box::new(VoskTranscriber::load(model.as_ref(), library.as_ref()).expect("chargement"))
        }
        _ => {
            Box::new(WhisperTranscriber::load(model.as_ref(), "fr", true, &[]).expect("chargement"))
        }
    };
    eprintln!("chargement : {} ms", started.elapsed().as_millis());

    // VTT_WORDS=1 : mots horodatés (capture commentée).
    if std::env::var_os("VTT_WORDS").is_some() {
        for word in transcriber.transcribe_words(&clip).expect("transcription") {
            println!("{:>6} {:>6}  {}", word.start_ms, word.end_ms, word.text);
        }
        return;
    }
    let pipeline = Pipeline::new(transcriber, None, Box::new(Stdout));
    let started = Instant::now();
    let outcome = pipeline
        .run(&clip, &vtt_core::Target::default())
        .expect("transcription");
    eprintln!(
        "{} ms d'audio transcrits en {} ms : {outcome:?}",
        clip.duration_ms(),
        started.elapsed().as_millis()
    );
}
