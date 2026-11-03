//! Cœur portable de VoiceToText : configuration et pipeline de dictée.
//!
//! Aucune dépendance à un OS ici : les implémentations concrètes (Whisper,
//! LLM, insertion Windows) vivent dans d'autres crates et se branchent via
//! les traits de [`pipeline`].

pub mod audio;
pub mod config;
pub mod history;
pub mod hotkey;
pub mod pipeline;
pub mod text;

pub use config::Config;
pub use pipeline::{
    AudioClip, Error, Outcome, Pipeline, PostProcessor, SAMPLE_RATE, TextSink, Transcriber,
};
