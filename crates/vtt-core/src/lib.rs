//! Cœur portable de VoiceToText : configuration, pipeline de dictée,
//! réunions.
//!
//! Aucune dépendance à un OS ici : les implémentations concrètes (Whisper,
//! LLM, insertion Windows) vivent dans d'autres crates et se branchent via
//! les traits de [`pipeline`].

pub mod annotation;
pub mod audio;
pub mod calls;
pub mod config;
pub mod history;
pub mod hotkey;
pub mod meeting;
pub mod pipeline;
pub mod search;
pub mod suggest;
pub mod text;
pub mod usage;

pub use config::Config;
pub use pipeline::{
    AudioClip, Cleaned, Directives, Error, Outcome, Pipeline, PostProcessor, SAMPLE_RATE, Target,
    TextSink, Transcriber, WithFallback, Word, spread_words,
};
