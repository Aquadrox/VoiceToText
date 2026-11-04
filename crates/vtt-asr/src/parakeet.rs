//! Transcription avec Parakeet TDT 0.6B v3 (NVIDIA), sur le processeur, via
//! ONNX Runtime. Multilingue (langue détectée), ponctuation et majuscules
//! comprises.
//!
//! ONNX Runtime (`onnxruntime.dll`) n'est pas dans l'installeur : il est
//! téléchargé avec le modèle, vérifié, puis chargé une fois pour toutes.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use parakeet_rs::ort::environment::Environment;
use parakeet_rs::{ExecutionConfig, ParakeetTDT, TimestampMode, Transcriber as _};
use vtt_core::{AudioClip, Error, SAMPLE_RATE, Transcriber, Word};

/// ONNX Runtime ne se charge qu'une fois par processus.
static RUNTIME: OnceLock<Result<Environment, String>> = OnceLock::new();

fn environment(runtime: &Path) -> Result<Environment, Error> {
    RUNTIME
        .get_or_init(|| {
            let started = Instant::now();
            let environment = parakeet_rs::ort::init_from(runtime)
                .map_err(|e| format!("chargement de {} : {e}", runtime.display()))?
                .with_name("voicetotext")
                .build()
                .map_err(|e| format!("ONNX Runtime : {e}"))?;
            log::info!(
                "ONNX Runtime chargé en {} ms",
                started.elapsed().as_millis()
            );
            Ok(environment)
        })
        .clone()
        .map_err(Error::Asr)
}

pub struct ParakeetTranscriber {
    dir: PathBuf,
    config: ExecutionConfig,
    /// `None` : modèle libéré (mémoire), rechargé à la dictée suivante.
    model: Mutex<Option<ParakeetTDT>>,
}

impl ParakeetTranscriber {
    /// `dir` : dossier du modèle (encoder, decoder_joint, vocab.txt) ;
    /// `runtime` : bibliothèque ONNX Runtime. Préchauffe le modèle.
    pub fn load(dir: &Path, runtime: &Path) -> Result<Self, Error> {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);
        let config = ExecutionConfig::new()
            .with_intra_threads(threads)
            .with_environment(environment(runtime)?);
        let transcriber = Self {
            dir: dir.to_owned(),
            config,
            model: Mutex::new(None),
        };
        transcriber.preload()?;
        let started = Instant::now();
        transcriber.transcribe(&AudioClip {
            samples: vec![0.0; SAMPLE_RATE as usize],
        })?;
        log::info!("préchauffage en {} ms", started.elapsed().as_millis());
        Ok(transcriber)
    }

    fn lock(&self) -> MutexGuard<'_, Option<ParakeetTDT>> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ensure_loaded<'a>(
        &self,
        slot: &'a mut Option<ParakeetTDT>,
    ) -> Result<&'a mut ParakeetTDT, Error> {
        if slot.is_none() {
            let started = Instant::now();
            let model = ParakeetTDT::from_pretrained(&self.dir, Some(self.config.clone()))
                .map_err(|e| Error::Asr(format!("chargement de {} : {e}", self.dir.display())))?;
            log::info!(
                "modèle {} chargé en {} ms (processeur)",
                self.dir.display(),
                started.elapsed().as_millis()
            );
            *slot = Some(model);
        }
        Ok(slot.as_mut().expect("modèle chargé ci-dessus"))
    }
}

impl Transcriber for ParakeetTranscriber {
    fn unload(&self) -> bool {
        self.lock().take().is_some()
    }

    fn preload(&self) -> Result<(), Error> {
        let mut slot = self.lock();
        self.ensure_loaded(&mut slot).map(|_| ())
    }

    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error> {
        let mut slot = self.lock();
        let model = self.ensure_loaded(&mut slot)?;
        let result = model
            .transcribe_samples(clip.samples.clone(), SAMPLE_RATE, 1, None)
            .map_err(|e| Error::Asr(e.to_string()))?;
        Ok(result.text.trim().to_owned())
    }

    fn transcribe_words(&self, clip: &AudioClip) -> Result<Vec<Word>, Error> {
        let mut slot = self.lock();
        let model = self.ensure_loaded(&mut slot)?;
        let result = model
            .transcribe_samples(
                clip.samples.clone(),
                SAMPLE_RATE,
                1,
                Some(TimestampMode::Words),
            )
            .map_err(|e| Error::Asr(e.to_string()))?;
        Ok(result
            .tokens
            .into_iter()
            .filter(|t| !t.text.trim().is_empty())
            .map(|t| Word {
                text: t.text.trim().to_owned(),
                start_ms: (t.start.max(0.0) * 1000.0) as u64,
                end_ms: (t.end.max(0.0) * 1000.0) as u64,
            })
            .collect())
    }
}
