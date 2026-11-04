//! VoiceToText sur Android : bibliothèque appelée par l'application Kotlin
//! (classe `io.github.aquadrox.voicetotext.Native`). Transcription Whisper
//! sur le téléphone, puis pipeline commun avec le PC (snippets, commandes
//! vocales, nettoyage LLM).
//!
//! Les échanges se font en texte (JSON ou config.toml) pour garder l'interface
//! JNI minimale. L'audio ne quitte jamais le téléphone ; la clé API n'est
//! gardée qu'en mémoire.

pub mod engine;
pub mod models;
pub mod settings;

use std::path::PathBuf;

use jni::JNIEnv;
use jni::objects::{JClass, JFloatArray, JString};
use jni::sys::{jint, jstring};
use serde::Serialize;

/// Texte Java → Rust (vide si nul ou illisible).
fn text(env: &mut JNIEnv, value: &JString) -> String {
    if value.is_null() {
        return String::new();
    }
    env.get_string(value).map(String::from).unwrap_or_default()
}

/// Rust → texte Java (nul si l'allocation échoue).
fn java(env: &mut JNIEnv, value: &str) -> jstring {
    env.new_string(value)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// `{"ok": …}` ou `{"error": "…"}`.
fn reply<T: Serialize>(result: Result<T, String>) -> String {
    let value = match result {
        Ok(ok) => serde_json::json!({ "ok": ok }),
        Err(error) => serde_json::json!({ "error": error }),
    };
    value.to_string()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_init(
    _env: JNIEnv,
    _class: JClass,
) {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("VoiceToText"),
    );
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_settings<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
    models_dir: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let dir = PathBuf::from(text(&mut env, &models_dir));
    let json =
        reply(settings::view(&toml, &dir).and_then(|v| {
            serde_json::from_str::<serde_json::Value>(&v).map_err(|e| e.to_string())
        }));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_updateSettings<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
    update: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let update = text(&mut env, &update);
    let json = reply(
        serde_json::from_str::<settings::Update>(&update)
            .map_err(|e| e.to_string())
            .and_then(|u| settings::update(&toml, &u)),
    );
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_importSettings<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    file: JString<'a>,
) -> jstring {
    let file = text(&mut env, &file);
    let json = reply(settings::import(&file));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_download<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    models_dir: JString<'a>,
    id: JString<'a>,
) -> jstring {
    let dir = PathBuf::from(text(&mut env, &models_dir));
    let id = text(&mut env, &id);
    let json = reply(models::download(&dir, &id).map(|_| ()));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_progress(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    models::PROGRESS
        .load(std::sync::atomic::Ordering::Relaxed)
        .into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_setApiKey<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    key: JString<'a>,
) {
    let key = text(&mut env, &key);
    engine::set_api_key(Some(key));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_start<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    models_dir: JString<'a>,
    history_file: JString<'a>,
    toml: JString<'a>,
) -> jstring {
    let dir = PathBuf::from(text(&mut env, &models_dir));
    let history = PathBuf::from(text(&mut env, &history_file));
    let toml = text(&mut env, &toml);
    let json = reply(engine::start(&dir, &history, &toml));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_history<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    history_file: JString<'a>,
    limit: jint,
) -> jstring {
    let file = PathBuf::from(text(&mut env, &history_file));
    let json = reply(engine::history(&file, limit.max(0) as usize));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_clearHistory<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    history_file: JString<'a>,
) -> jstring {
    let file = PathBuf::from(text(&mut env, &history_file));
    let json = reply(engine::clear_history(&file));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_vocabulary<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let json = reply(settings::vocabulary(&toml));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_saveVocabulary<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
    vocabulary: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let vocabulary = text(&mut env, &vocabulary);
    let json = reply(
        serde_json::from_str::<settings::Vocabulary>(&vocabulary)
            .map_err(|e| e.to_string())
            .and_then(|v| settings::save_vocabulary(&toml, &v)),
    );
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_dictate<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    samples: JFloatArray<'a>,
    app: JString<'a>,
    mode: jint,
    selection: JString<'a>,
) -> jstring {
    let length = env.get_array_length(&samples).unwrap_or(0).max(0) as usize;
    let mut buffer = vec![0f32; length];
    if env
        .get_float_array_region(&samples, 0, &mut buffer)
        .is_err()
    {
        buffer.clear();
    }
    let app = Some(text(&mut env, &app)).filter(|a| !a.is_empty());
    // 0 : dictée ; 1 : traduction ; 2 : Command Mode (sélection éventuelle).
    let mode = match mode {
        1 => engine::Mode::Translate,
        2 => engine::Mode::Command(Some(text(&mut env, &selection)).filter(|s| !s.is_empty())),
        _ => engine::Mode::Dictate,
    };
    let result = engine::dictate(buffer, app, mode);
    let json = serde_json::to_string(&result).unwrap_or_default();
    java(&mut env, &json)
}
