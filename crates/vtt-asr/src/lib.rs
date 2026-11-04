//! Transcription locale : whisper.cpp (Whisper), ONNX Runtime (Parakeet),
//! Kaldi (Vosk).

#[cfg(feature = "parakeet")]
mod parakeet;
#[cfg(feature = "vosk")]
mod vosk;

#[cfg(feature = "parakeet")]
pub use parakeet::ParakeetTranscriber;
#[cfg(feature = "vosk")]
pub use vosk::VoskTranscriber;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use vtt_core::{AudioClip, Error, SAMPLE_RATE, Transcriber, Word};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// whisper.cpp refuse les entrées de moins d'une seconde : on complète par du silence.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize * 11 / 10;

pub struct WhisperTranscriber {
    model: PathBuf,
    use_gpu: bool,
    language: String,
    /// Mots du dictionnaire, donnés à Whisper comme contexte pour l'orthographe.
    /// Modifiable à chaud depuis la fenêtre principale.
    initial_prompt: Mutex<Option<String>>,
    n_threads: i32,
    // Un seul état réutilisé : en créer un par dictée réallouerait les tampons
    // GPU. `None` : modèle déchargé pour libérer la mémoire graphique (l'état
    // détient le contexte : le libérer libère tout).
    state: Mutex<Option<WhisperState>>,
}

impl WhisperTranscriber {
    /// Charge le modèle puis fait une transcription à vide pour que la
    /// première vraie dictée ne paie pas l'initialisation du GPU.
    pub fn load(
        model: &Path,
        language: &str,
        use_gpu: bool,
        dictionary: &[String],
    ) -> Result<Self, Error> {
        whisper_rs::install_logging_hooks();
        #[cfg(feature = "vulkan")]
        for device in whisper_rs::vulkan::list_devices() {
            log::info!(
                "GPU Vulkan {} : {} ({} Mo)",
                device.id,
                device.name,
                device.vram.total / (1024 * 1024)
            );
        }
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8) as i32)
            .unwrap_or(4);
        let transcriber = Self {
            model: model.to_owned(),
            use_gpu,
            language: language.to_owned(),
            initial_prompt: Mutex::new(vocabulary_prompt(dictionary)),
            n_threads,
            state: Mutex::new(None),
        };
        transcriber.preload()?;

        let started = Instant::now();
        transcriber.transcribe(&AudioClip::default())?;
        log::info!("préchauffage en {} ms", started.elapsed().as_millis());
        Ok(transcriber)
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, Option<WhisperState>>, Error> {
        self.state
            .lock()
            .map_err(|_| Error::Asr("état Whisper empoisonné".into()))
    }

    /// Charge le modèle dans `slot` s'il est vide.
    fn ensure_loaded<'a>(
        &self,
        slot: &'a mut Option<WhisperState>,
    ) -> Result<&'a mut WhisperState, Error> {
        if slot.is_none() {
            let started = Instant::now();
            let mut params = WhisperContextParameters::default();
            params.use_gpu(self.use_gpu);
            let path = self.model.to_str().ok_or_else(|| {
                Error::Asr(format!("chemin non UTF-8 : {}", self.model.display()))
            })?;
            let ctx = WhisperContext::new_with_params(path, params)
                .map_err(|e| Error::Asr(format!("chargement de {} : {e}", self.model.display())))?;
            let state = ctx
                .create_state()
                .map_err(|e| Error::Asr(format!("création de l'état : {e}")))?;
            log::info!(
                "modèle {} chargé en {} ms (GPU demandé : {})",
                self.model.display(),
                started.elapsed().as_millis(),
                self.use_gpu
            );
            *slot = Some(state);
        }
        Ok(slot.as_mut().expect("modèle chargé ci-dessus"))
    }
}

fn vocabulary_prompt(dictionary: &[String]) -> Option<String> {
    let words: Vec<&str> = dictionary
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .collect();
    (!words.is_empty()).then(|| words.join(", "))
}

impl Transcriber for WhisperTranscriber {
    fn set_vocabulary(&self, words: &[String]) {
        *self
            .initial_prompt
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = vocabulary_prompt(words);
    }

    fn unload(&self) -> bool {
        // Attend la fin d'une transcription en cours.
        let mut slot = self.state.lock().unwrap_or_else(|e| e.into_inner());
        slot.take().is_some()
    }

    fn preload(&self) -> Result<(), Error> {
        let mut slot = self.lock_state()?;
        self.ensure_loaded(&mut slot).map(|_| ())
    }

    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error> {
        let mut text = String::new();
        self.run(clip, false, |segment, _, _| text.push_str(segment))?;
        Ok(text.trim().to_owned())
    }

    /// Un segment par mot (`max_len` 1, coupure aux mots), horodaté.
    fn transcribe_words(&self, clip: &AudioClip) -> Result<Vec<Word>, Error> {
        let mut words = Vec::new();
        self.run(clip, true, |segment, start_ms, end_ms| {
            let text = segment.trim();
            if !text.is_empty() {
                words.push(Word {
                    text: text.to_owned(),
                    start_ms,
                    end_ms,
                });
            }
        })?;
        Ok(words)
    }
}

impl WhisperTranscriber {
    /// Transcrit `clip` ; `on_segment` reçoit chaque segment et ses bornes
    /// (ms). `words` : un segment par mot.
    fn run(
        &self,
        clip: &AudioClip,
        words: bool,
        mut on_segment: impl FnMut(&str, u64, u64),
    ) -> Result<(), Error> {
        let prompt = self
            .initial_prompt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_translate(false);
        params.set_n_threads(self.n_threads);
        // Chaque dictée est indépendante : pas de contexte hérité de la précédente.
        params.set_no_context(true);
        params.set_no_timestamps(!words);
        if words {
            params.set_token_timestamps(true);
            params.set_max_len(1);
            params.set_split_on_word(true);
        }
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(prompt) = &prompt {
            params.set_initial_prompt(prompt);
        }

        let mut samples = clip.samples.clone();
        if samples.len() < MIN_SAMPLES {
            samples.resize(MIN_SAMPLES, 0.0);
        }

        let mut slot = self.lock_state()?;
        let state = self.ensure_loaded(&mut slot)?;
        state
            .full(params, &samples)
            .map_err(|e| Error::Asr(e.to_string()))?;

        for segment in state.as_iter() {
            let part = segment
                .to_str_lossy()
                .map_err(|e| Error::Asr(e.to_string()))?;
            // Horodatage en centièmes de seconde.
            let start = segment.start_timestamp().max(0) as u64 * 10;
            let end = segment.end_timestamp().max(0) as u64 * 10;
            on_segment(&part, start, end);
        }
        Ok(())
    }
}
