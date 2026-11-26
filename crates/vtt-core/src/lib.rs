//! Cœur portable de VoiceToText : configuration, pipeline de dictée,
//! réunions.
//!
//! Aucune dépendance à un OS ici : les implémentations concrètes (Whisper,
//! LLM, insertion Windows) vivent dans d'autres crates et se branchent via
//! les traits de [`pipeline`].

pub mod annotation;
pub mod audio;
pub mod calls;
pub mod changelog;
pub mod config;
pub mod context;
pub mod diarize;
pub mod fbank;
pub mod history;
pub mod hotkey;
pub mod live;
pub mod meeting;
pub mod pipeline;
pub mod search;
pub mod semantic;
pub mod suggest;
pub mod sync;
pub mod text;
pub mod usage;
pub mod voicebook;

pub use config::Config;
pub use pipeline::{
    AudioClip, Cleaned, Directives, Error, LiveInsertion, Outcome, Pipeline, PostProcessor,
    RawDictation, SAMPLE_RATE, Target, TextSink, Transcriber, WithFallback, Word, spread_words,
};
