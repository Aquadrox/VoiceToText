//! Transcription locale avec whisper.cpp.

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use vtt_core::{AudioClip, Error, SAMPLE_RATE, Transcriber};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// whisper.cpp refuse les entrées de moins d'une seconde : on complète par du silence.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize * 11 / 10;

pub struct WhisperTranscriber {
    language: String,
    /// Mots du dictionnaire, donnés à Whisper comme contexte pour l'orthographe.
    initial_prompt: Option<String>,
    n_threads: i32,
    // Un seul état réutilisé : en créer un par dictée réallouerait les tampons GPU.
    state: Mutex<WhisperState>,
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
        let started = Instant::now();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu);
        let path = model
            .to_str()
            .ok_or_else(|| Error::Asr(format!("chemin non UTF-8 : {}", model.display())))?;
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| Error::Asr(format!("chargement de {} : {e}", model.display())))?;
        let state = ctx
            .create_state()
            .map_err(|e| Error::Asr(format!("création de l'état : {e}")))?;
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8) as i32)
            .unwrap_or(4);
        let words: Vec<&str> = dictionary
            .iter()
            .map(|w| w.trim())
            .filter(|w| !w.is_empty())
            .collect();
        let transcriber = Self {
            language: language.to_owned(),
            initial_prompt: (!words.is_empty()).then(|| words.join(", ")),
            n_threads,
            state: Mutex::new(state),
        };
        log::info!(
            "modèle {} chargé en {} ms (GPU demandé : {use_gpu})",
            model.display(),
            started.elapsed().as_millis()
        );

        let started = Instant::now();
        transcriber.transcribe(&AudioClip::default())?;
        log::info!("préchauffage en {} ms", started.elapsed().as_millis());
        Ok(transcriber)
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_translate(false);
        params.set_n_threads(self.n_threads);
        // Chaque dictée est indépendante : pas de contexte hérité de la précédente.
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(prompt) = &self.initial_prompt {
            params.set_initial_prompt(prompt);
        }

        let mut samples = clip.samples.clone();
        if samples.len() < MIN_SAMPLES {
            samples.resize(MIN_SAMPLES, 0.0);
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::Asr("état Whisper empoisonné".into()))?;
        state
            .full(params, &samples)
            .map_err(|e| Error::Asr(e.to_string()))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            let part = segment
                .to_str_lossy()
                .map_err(|e| Error::Asr(e.to_string()))?;
            text.push_str(&part);
        }
        Ok(text.trim().to_owned())
    }
}
