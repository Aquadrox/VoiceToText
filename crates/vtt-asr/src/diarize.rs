//! Diarisation (qui parle quand) avec ONNX Runtime : segmentation pyannote
//! 3.0 et modèle d'empreinte vocale (WeSpeaker ou 3D-Speaker, format
//! sherpa-onnx). L'algorithme est dans [`vtt_core::diarize`].

use std::path::Path;
use std::time::Instant;

use parakeet_rs::ort::session::Session;
use parakeet_rs::ort::value::Tensor;
use vtt_core::Error;
use vtt_core::diarize::{self, CLASSES, Models, Options, Turn, WINDOW};
use vtt_core::fbank::{self, Fbank, MEL_BINS, Window};

use crate::parakeet::environment;

fn ort_error(context: &str) -> impl Fn(parakeet_rs::ort::Error) -> Error + '_ {
    move |e| Error::Asr(format!("{context} : {e}"))
}

pub struct Diarizer {
    segmentation: Session,
    embedding: Session,
    fbank: Fbank,
    /// Échantillons multipliés par 32768 avant les fbank (modèle entraîné
    /// sur des entiers 16 bits).
    scale: f32,
    /// Moyenne des fbank retirée avant l'empreinte.
    subtract_mean: bool,
}

impl Diarizer {
    /// `segmentation` : `segmentation-3.0` en ONNX ; `embedding` : modèle
    /// d'empreinte (sherpa-onnx) ; `runtime` : bibliothèque ONNX Runtime.
    pub fn load(segmentation: &Path, embedding: &Path, runtime: &Path) -> Result<Self, Error> {
        let environment = environment(runtime)?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(4))
            .unwrap_or(2);
        let open = |path: &Path| -> Result<Session, Error> {
            let context = path.display().to_string();
            Session::builder(&environment)
                .map_err(ort_error(&context))?
                .with_intra_threads(threads)
                .map_err(|e| Error::Asr(format!("{context} : {e}")))?
                .commit_from_file(path)
                .map_err(ort_error(&context))
        };
        let started = Instant::now();
        let segmentation = open(segmentation)?;
        let embedding = open(embedding)?;
        let meta = |key: &str| {
            embedding
                .metadata()
                .ok()
                .and_then(|m| m.custom(key))
                .unwrap_or_default()
        };
        let scale = if meta("normalize_samples") == "0" {
            32768.0
        } else {
            1.0
        };
        // Réglages de l'entraînement de chaque famille de modèles (WeSpeaker :
        // Hamming et moyenne retirée ; 3D-Speaker : Povey et moyenne
        // retirée). Mesuré : sans moyenne retirée, WeSpeaker distingue bien
        // moins deux voix.
        let wespeaker = meta("framework") == "wespeaker";
        let subtract_mean = wespeaker || meta("feature_normalize_type") == "global-mean";
        let window = if wespeaker {
            Window::Hamming
        } else {
            Window::Povey
        };
        log::info!(
            "diarisation : modèles chargés en {} ms ({})",
            started.elapsed().as_millis(),
            meta("framework")
        );
        Ok(Self {
            segmentation,
            embedding,
            fbank: Fbank::new(window),
            scale,
            subtract_mean,
        })
    }

    /// Tours de parole de `audio` (16 kHz mono).
    pub fn diarize(
        &mut self,
        audio: &[f32],
        options: &Options,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Vec<Turn>, Error> {
        diarize::diarize(audio, self, options, progress)
    }
}

impl Models for Diarizer {
    fn segment(&mut self, window: &[f32]) -> Result<Vec<[f32; CLASSES]>, Error> {
        debug_assert_eq!(window.len(), WINDOW);
        let input = Tensor::from_array(([1usize, 1, window.len()], window.to_vec()))
            .map_err(ort_error("segmentation"))?;
        let outputs = self
            .segmentation
            .run(parakeet_rs::ort::inputs![input])
            .map_err(ort_error("segmentation"))?;
        let (shape, scores) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_error("segmentation"))?;
        if shape.len() != 3 || shape[2] as usize != CLASSES {
            return Err(Error::Asr(format!(
                "segmentation : sortie {shape:?} inattendue"
            )));
        }
        Ok(scores
            .chunks_exact(CLASSES)
            .map(|row| row.try_into().expect("ligne de CLASSES valeurs"))
            .collect())
    }

    fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, Error> {
        let mut features = self.fbank.compute(samples, self.scale);
        if self.subtract_mean {
            fbank::subtract_mean(&mut features);
        }
        let frames = features.len();
        let flat: Vec<f32> = features.into_iter().flatten().collect();
        let input = Tensor::from_array(([1usize, frames, MEL_BINS], flat))
            .map_err(ort_error("empreinte vocale"))?;
        let outputs = self
            .embedding
            .run(parakeet_rs::ort::inputs![input])
            .map_err(ort_error("empreinte vocale"))?;
        let (_, embedding) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_error("empreinte vocale"))?;
        Ok(embedding.to_vec())
    }
}
