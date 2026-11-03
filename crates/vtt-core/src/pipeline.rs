//! Orchestration d'une dictée : audio → texte brut → texte propre → insertion.
//!
//! Les appels sont bloquants : le pipeline tourne sur un thread de travail
//! dédié, jamais sur le thread du hook clavier ou de l'interface.

use std::time::Instant;

use crate::{audio, text};

/// Fréquence attendue par Whisper.
pub const SAMPLE_RATE: u32 = 16_000;

/// En dessous de cette durée, l'enregistrement est jeté sans appeler l'ASR.
const MIN_CLIP_MS: u64 = 300;

/// Niveau RMS sous lequel un enregistrement est considéré comme du silence (~ -46 dBFS).
const SILENCE_RMS: f32 = 0.005;

/// Audio mono 16 kHz, échantillons f32 dans [-1, 1].
#[derive(Debug, Clone, Default)]
pub struct AudioClip {
    pub samples: Vec<f32>,
}

impl AudioClip {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / SAMPLE_RATE as u64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transcription : {0}")]
    Asr(String),
    #[error("nettoyage : {0}")]
    PostProcess(String),
    #[error("insertion : {0}")]
    Sink(String),
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error>;
}

pub trait PostProcessor: Send + Sync {
    fn process(&self, raw: &str) -> Result<String, Error>;
}

pub trait TextSink: Send + Sync {
    fn insert(&self, text: &str) -> Result<(), Error>;
}

/// Résultat d'une dictée, conservé pour l'historique.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Clip trop court, silence ou hallucination : rien n'a été inséré.
    Skipped,
    Inserted {
        raw: String,
        text: String,
        /// Le nettoyage a échoué et le texte brut a été inséré à la place.
        post_process_error: Option<String>,
    },
}

pub struct Pipeline {
    transcriber: Box<dyn Transcriber>,
    post_processor: Option<Box<dyn PostProcessor>>,
    sink: Box<dyn TextSink>,
}

impl Pipeline {
    pub fn new(
        transcriber: Box<dyn Transcriber>,
        post_processor: Option<Box<dyn PostProcessor>>,
        sink: Box<dyn TextSink>,
    ) -> Self {
        Self {
            transcriber,
            post_processor,
            sink,
        }
    }

    pub fn run(&self, clip: &AudioClip) -> Result<Outcome, Error> {
        if clip.duration_ms() < MIN_CLIP_MS {
            log::info!("ignoré : enregistrement de {} ms", clip.duration_ms());
            return Ok(Outcome::Skipped);
        }
        if audio::is_silent(&clip.samples, SAMPLE_RATE, SILENCE_RMS) {
            log::info!("ignoré : silence ({} ms)", clip.duration_ms());
            return Ok(Outcome::Skipped);
        }

        let started = Instant::now();
        let transcribed = self.transcriber.transcribe(clip)?;
        let asr_ms = started.elapsed().as_millis();
        let raw = text::strip_hallucinations(&transcribed);
        if raw.is_empty() {
            log::info!(
                "ignoré : transcription vide après filtrage ({} caractères écartés, ASR {asr_ms} ms)",
                transcribed.chars().count()
            );
            return Ok(Outcome::Skipped);
        }

        // Un échec du LLM ne doit jamais faire perdre la dictée.
        let started = Instant::now();
        let (text, post_process_error) = match &self.post_processor {
            None => (raw.clone(), None),
            Some(processor) => match processor.process(&raw) {
                Ok(text) if !text.trim().is_empty() => (text.trim().to_owned(), None),
                Ok(_) => (raw.clone(), Some("réponse vide".to_owned())),
                Err(err) => (raw.clone(), Some(err.to_string())),
            },
        };

        let post_ms = started.elapsed().as_millis();

        let started = Instant::now();
        self.sink.insert(&text)?;
        log::info!(
            "dictée de {} ms : ASR {asr_ms} ms, nettoyage {post_ms} ms, insertion {} ms",
            clip.duration_ms(),
            started.elapsed().as_millis()
        );
        Ok(Outcome::Inserted {
            raw,
            text,
            post_process_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FixedAsr(&'static str);
    impl Transcriber for FixedAsr {
        fn transcribe(&self, _: &AudioClip) -> Result<String, Error> {
            Ok(self.0.to_owned())
        }
    }

    struct Upper;
    impl PostProcessor for Upper {
        fn process(&self, raw: &str) -> Result<String, Error> {
            Ok(raw.to_uppercase())
        }
    }

    struct Failing;
    impl PostProcessor for Failing {
        fn process(&self, _: &str) -> Result<String, Error> {
            Err(Error::PostProcess("délai dépassé".into()))
        }
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<String>>>);
    impl TextSink for Recorder {
        fn insert(&self, text: &str) -> Result<(), Error> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    fn clip_ms(ms: u64) -> AudioClip {
        AudioClip {
            samples: vec![0.1; (ms * SAMPLE_RATE as u64 / 1000) as usize],
        }
    }

    fn pipeline(asr: &'static str, post: Option<Box<dyn PostProcessor>>) -> (Pipeline, Recorder) {
        let sink = Recorder::default();
        let p = Pipeline::new(Box::new(FixedAsr(asr)), post, Box::new(sink.clone()));
        (p, sink)
    }

    #[test]
    fn short_clip_is_skipped() {
        let (p, sink) = pipeline("bonjour", None);
        assert_eq!(p.run(&clip_ms(200)).unwrap(), Outcome::Skipped);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn silent_clip_is_skipped() {
        let (p, sink) = pipeline("Sous-titres réalisés par la communauté d'Amara.org", None);
        let silent = AudioClip {
            samples: vec![0.0; SAMPLE_RATE as usize],
        };
        assert_eq!(p.run(&silent).unwrap(), Outcome::Skipped);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn hallucination_only_is_skipped() {
        let (p, sink) = pipeline("Sous-titrage ST' 501", None);
        assert_eq!(p.run(&clip_ms(1000)).unwrap(), Outcome::Skipped);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn blank_transcription_is_skipped() {
        let (p, sink) = pipeline("  \n", None);
        assert_eq!(p.run(&clip_ms(1000)).unwrap(), Outcome::Skipped);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn post_processed_text_is_inserted() {
        let (p, sink) = pipeline(" bonjour ", Some(Box::new(Upper)));
        let outcome = p.run(&clip_ms(1000)).unwrap();
        assert_eq!(
            outcome,
            Outcome::Inserted {
                raw: "bonjour".into(),
                text: "BONJOUR".into(),
                post_process_error: None,
            }
        );
        assert_eq!(*sink.0.lock().unwrap(), ["BONJOUR"]);
    }

    #[test]
    fn post_process_failure_falls_back_to_raw() {
        let (p, sink) = pipeline("bonjour", Some(Box::new(Failing)));
        match p.run(&clip_ms(1000)).unwrap() {
            Outcome::Inserted {
                text,
                post_process_error,
                ..
            } => {
                assert_eq!(text, "bonjour");
                assert!(post_process_error.unwrap().contains("délai dépassé"));
            }
            other => panic!("inattendu : {other:?}"),
        }
        assert_eq!(*sink.0.lock().unwrap(), ["bonjour"]);
    }
}
