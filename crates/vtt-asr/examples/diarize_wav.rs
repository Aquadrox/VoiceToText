//! Essai de diarisation sur un fichier WAV : qui parle quand.
//!
//! cargo run --release -p vtt-asr --example diarize_wav -- \
//!     <segmentation.onnx> <empreinte.onnx> <libonnxruntime.so> <fichier.wav> \
//!     [--speakers N] [--threshold 0.6] [--step 2]
//!
//! Sortie : un tour de parole par ligne (début, fin, locuteur), puis la
//! durée de calcul.

use std::path::Path;
use std::time::Instant;

use vtt_asr::Diarizer;
use vtt_core::diarize::Options;
use vtt_core::{SAMPLE_RATE, audio};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut options = Options::default();
    let mut flag = |name: &str| {
        let i = args.iter().position(|a| a == name)?;
        let value = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        value
    };
    if let Some(n) = flag("--speakers") {
        options.speakers = Some(n.parse().expect("--speakers : un nombre"));
    }
    if let Some(t) = flag("--threshold") {
        options.threshold = t.parse().expect("--threshold : un nombre");
    }
    if let Some(s) = flag("--step") {
        let seconds: f32 = s.parse().expect("--step : des secondes");
        options.step = (seconds * SAMPLE_RATE as f32) as usize;
    }
    let [segmentation, embedding, runtime, wav] = args.as_slice() else {
        eprintln!(
            "usage : diarize_wav <segmentation.onnx> <empreinte.onnx> <libonnxruntime> <fichier.wav> [--speakers N] [--threshold X] [--step S]"
        );
        std::process::exit(2);
    };

    let mut reader = hound::WavReader::open(wav).expect("lecture du WAV");
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
    let samples = audio::resample(&mono, spec.sample_rate, SAMPLE_RATE);
    let duration = samples.len() as f32 / SAMPLE_RATE as f32;

    let started = Instant::now();
    let mut diarizer = Diarizer::load(
        Path::new(segmentation),
        Path::new(embedding),
        Path::new(runtime),
    )
    .expect("chargement des modèles");
    let loaded = started.elapsed();
    let started = Instant::now();
    let turns = diarizer
        .diarize(&samples, &options, &mut |_| {})
        .expect("diarisation");
    let elapsed = started.elapsed();

    for t in &turns {
        println!(
            "{:8.2} → {:8.2}  locuteur {}",
            t.start_ms as f32 / 1000.0,
            t.end_ms as f32 / 1000.0,
            t.speaker + 1
        );
    }
    let speakers = turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0);
    eprintln!(
        "{speakers} locuteur(s), {} tours ; audio {duration:.1} s, calcul {:.2} s (×{:.1} temps réel), chargement {} ms",
        turns.len(),
        elapsed.as_secs_f32(),
        duration / elapsed.as_secs_f32(),
        loaded.as_millis()
    );
}
