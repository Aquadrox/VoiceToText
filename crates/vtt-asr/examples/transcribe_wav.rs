//! Fait passer un fichier WAV par le pipeline complet, sans micro ni clavier.
//!
//! cargo run --release -p vtt-asr --example transcribe_wav -- <modèle.bin> <fichier.wav>

use std::time::Instant;

use vtt_asr::WhisperTranscriber;
use vtt_core::{AudioClip, Error, Pipeline, SAMPLE_RATE, TextSink, audio};

struct Stdout;

impl TextSink for Stdout {
    fn insert(&self, text: &str) -> Result<(), Error> {
        println!("{text}");
        Ok(())
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(model), Some(wav)) = (args.next(), args.next()) else {
        eprintln!("usage : transcribe_wav <modèle.bin> <fichier.wav>");
        std::process::exit(2);
    };

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
    let transcriber = WhisperTranscriber::load(model.as_ref(), "fr", true).expect("chargement");
    eprintln!("chargement : {} ms", started.elapsed().as_millis());

    let pipeline = Pipeline::new(Box::new(transcriber), None, Box::new(Stdout));
    let started = Instant::now();
    let outcome = pipeline.run(&clip).expect("transcription");
    eprintln!(
        "{} ms d'audio transcrits en {} ms : {outcome:?}",
        clip.duration_ms(),
        started.elapsed().as_millis()
    );
}
