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
use vtt_core::changelog::{self, Platform, Release};

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

/// Exécute `f` en rattrapant ses paniques : une panique qui sortirait d'une
/// fonction JNI arrêterait tout le processus (clavier, application et
/// service de la bulle, qu'Android marque alors « Ne fonctionne pas »). Elle
/// devient une erreur `{"error": "…"}` (même forme que [`reply`]), affichée
/// à l'utilisateur. Équivalent d'un `try/catch` autour de tout l'appel.
fn protect(f: impl FnOnce() -> String) -> String {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "cause inconnue".to_owned());
        reply::<()>(Err(format!("erreur interne du moteur : {message}")))
    })
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
    // Message et emplacement des paniques dans le journal Android (logcat) :
    // sans ce crochet, ils partiraient sur la sortie d'erreur, invisible.
    std::panic::set_hook(Box::new(|info| log::error!("panique : {info}")));
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
        protect(|| {
            reply(settings::view(&toml, &dir).and_then(|v| {
                serde_json::from_str::<serde_json::Value>(&v).map_err(|e| e.to_string())
            }))
        });
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
    let json = protect(|| {
        reply(
            serde_json::from_str::<settings::Update>(&update)
                .map_err(|e| e.to_string())
                .and_then(|u| settings::update(&toml, &u)),
        )
    });
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_importSettings<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    file: JString<'a>,
) -> jstring {
    let file = text(&mut env, &file);
    let json = protect(|| reply(settings::import(&file)));
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
    let json = protect(|| reply(models::download(&dir, &id).map(|_| ())));
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
    protect(|| {
        engine::set_api_key(Some(key));
        String::new()
    });
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
    let json = protect(|| reply(engine::start(&dir, &history, &toml)));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_release(
    _env: JNIEnv,
    _class: JClass,
) {
    protect(|| {
        engine::release();
        String::new()
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_history<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    history_file: JString<'a>,
    limit: jint,
) -> jstring {
    let file = PathBuf::from(text(&mut env, &history_file));
    let json = protect(|| reply(engine::history(&file, limit.max(0) as usize)));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_clearHistory<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    history_file: JString<'a>,
) -> jstring {
    let file = PathBuf::from(text(&mut env, &history_file));
    let json = protect(|| reply(engine::clear_history(&file)));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_vocabulary<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let json = protect(|| reply(settings::vocabulary(&toml)));
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
    let json = protect(|| {
        reply(
            serde_json::from_str::<settings::Vocabulary>(&vocabulary)
                .map_err(|e| e.to_string())
                .and_then(|v| settings::save_vocabulary(&toml, &v)),
        )
    });
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_prompts<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let json = protect(|| reply(settings::prompts(&toml)));
    java(&mut env, &json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_savePrompts<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    toml: JString<'a>,
    prompts: JString<'a>,
) -> jstring {
    let toml = text(&mut env, &toml);
    let prompts = text(&mut env, &prompts);
    let json = protect(|| {
        reply(
            serde_json::from_str::<settings::Prompts>(&prompts)
                .map_err(|e| e.to_string())
                .and_then(|p| settings::save_prompts(&toml, &p)),
        )
    });
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
    privacy: jint,
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
    // 1 : pas d'historique ; 2 : mot de passe (ni LLM ni historique).
    let privacy = match privacy {
        1 => engine::Privacy::NoHistory,
        2 => engine::Privacy::Password,
        _ => engine::Privacy::Normal,
    };
    // Erreur rattrapée : `{"error": …}` sans `kind`, affichée comme une
    // dictée en erreur.
    let json = protect(|| {
        let result = engine::dictate(buffer, app, mode, privacy);
        serde_json::to_string(&result).unwrap_or_default()
    });
    java(&mut env, &json)
}

/// Notes de version pour le téléphone (changements `[Android]` et communs),
/// de la plus récente à la plus ancienne. `text` vide : notes embarquées dans
/// l'application ; sinon champ `changelog` du manifeste de mise à jour.
/// Versions après `after` (exclue) et jusqu'à `up_to` (incluse) ; borne
/// vide : sans limite.
pub fn release_notes(text: &str, after: &str, up_to: &str) -> Vec<Release> {
    let text = if text.is_empty() {
        changelog::BUNDLED
    } else {
        text
    };
    let bound = |version: &str| (!version.is_empty()).then(|| version.to_owned());
    changelog::releases(
        text,
        Platform::Android,
        bound(after).as_deref(),
        bound(up_to).as_deref(),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_aquadrox_voicetotext_Native_releaseNotes<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    changelog: JString<'a>,
    after: JString<'a>,
    up_to: JString<'a>,
) -> jstring {
    let changelog = text(&mut env, &changelog);
    let after = text(&mut env, &after);
    let up_to = text(&mut env, &up_to);
    let json = protect(|| reply(Ok(release_notes(&changelog, &after, &up_to))));
    java(&mut env, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_for_the_phone() {
        let manifest = "## 0.23.0 — 2026-11-10\n- [PC] PC seul\n- [Android] Téléphone\n\n\
                        ## 0.22.0 — 2026-11-08\n- Commun\n";
        let notes = release_notes(manifest, "0.22.0", "0.23.0");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].items, ["Téléphone"]);
        assert_eq!(release_notes(manifest, "", "").len(), 2);
        // Notes embarquées : la version compilée en tête, sans date absente
        // envoyée en `null` (lue comme « null » par Android).
        let bundled = release_notes("", "", env!("CARGO_PKG_VERSION"));
        assert!(!bundled.is_empty());
        let json = reply(Ok(bundled));
        assert!(!json.contains("null"), "{json}");
        let undated: serde_json::Value =
            serde_json::from_str(&reply(Ok(release_notes("## 1.0.0\n- a\n", "", "")))).unwrap();
        assert_eq!(
            undated,
            serde_json::json!({ "ok": [{ "version": "1.0.0", "items": ["a"] }] })
        );
    }

    #[test]
    fn protect_returns_the_result_when_nothing_panics() {
        assert_eq!(protect(|| reply(Ok(1))), r#"{"ok":1}"#);
    }

    #[test]
    fn protect_turns_a_panic_into_an_error() {
        let json: serde_json::Value =
            serde_json::from_str(&protect(|| panic!("index hors limites"))).unwrap();
        assert_eq!(
            json["error"],
            "erreur interne du moteur : index hors limites"
        );
        let owned = protect(|| panic!("{} échantillons", 3));
        assert!(owned.contains("3 échantillons"), "{owned}");
    }
}
