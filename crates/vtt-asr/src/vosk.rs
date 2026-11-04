//! Transcription avec Vosk (Kaldi) : modèles de quelques dizaines de Mo,
//! temps réel sur n'importe quel processeur. Moins précis que Whisper ou
//! Parakeet, et sans ponctuation ni majuscules (le nettoyage LLM s'en
//! charge ; sans LLM, seuls une majuscule initiale et un point final sont
//! ajoutés).
//!
//! La bibliothèque native (`libvosk.dll`) n'est pas dans l'installeur : elle
//! est téléchargée avec le modèle, vérifiée, puis chargée une fois pour toutes.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_short, c_void};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use libloading::Library;
use vtt_core::{AudioClip, Error, SAMPLE_RATE, Transcriber};

/// Fonctions de `vosk_api.h` utilisées.
struct Api {
    set_log_level: unsafe extern "C" fn(c_int),
    model_new: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    model_free: unsafe extern "C" fn(*mut c_void),
    recognizer_new: unsafe extern "C" fn(*mut c_void, c_float) -> *mut c_void,
    accept_waveform_s: unsafe extern "C" fn(*mut c_void, *const c_short, c_int) -> c_int,
    final_result: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    recognizer_free: unsafe extern "C" fn(*mut c_void),
    /// Garde la bibliothèque chargée tant que les pointeurs ci-dessus servent.
    _library: Library,
}

/// La bibliothèque ne se charge qu'une fois par processus.
static API: OnceLock<Result<Api, String>> = OnceLock::new();

fn open(path: &Path) -> Result<Library, libloading::Error> {
    // Les DLL dont dépend libvosk.dll (libstdc++…) sont à côté d'elle.
    #[cfg(windows)]
    unsafe {
        libloading::os::windows::Library::load_with_flags(
            path,
            libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH,
        )
        .map(Library::from)
    }
    #[cfg(not(windows))]
    unsafe {
        Library::new(path)
    }
}

fn api(library: &Path) -> Result<&'static Api, Error> {
    API.get_or_init(|| {
        let lib =
            open(library).map_err(|e| format!("chargement de {} : {e}", library.display()))?;
        // SAFETY: signatures recopiées de vosk_api.h (Vosk 0.3.45) ; les
        // pointeurs restent valides tant que `_library` vit (toujours : statique).
        unsafe {
            macro_rules! symbol {
                ($name:literal) => {
                    *lib.get($name)
                        .map_err(|e| format!("libvosk : {} : {e}", stringify!($name)))?
                };
            }
            let api = Api {
                set_log_level: symbol!(b"vosk_set_log_level\0"),
                model_new: symbol!(b"vosk_model_new\0"),
                model_free: symbol!(b"vosk_model_free\0"),
                recognizer_new: symbol!(b"vosk_recognizer_new\0"),
                accept_waveform_s: symbol!(b"vosk_recognizer_accept_waveform_s\0"),
                final_result: symbol!(b"vosk_recognizer_final_result\0"),
                recognizer_free: symbol!(b"vosk_recognizer_free\0"),
                _library: lib,
            };
            // Kaldi écrit beaucoup sur la sortie d'erreur : seulement les erreurs.
            (api.set_log_level)(-1);
            Ok(api)
        }
    })
    .as_ref()
    .map_err(|e| Error::Asr(e.clone()))
}

/// Modèle Vosk chargé ; libéré à la destruction.
struct Model {
    api: &'static Api,
    ptr: *mut c_void,
}

// SAFETY: un modèle Vosk est partageable entre threads (documentation de
// vosk_api.h) ; il n'est de toute façon utilisé que sous le verrou.
unsafe impl Send for Model {}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: pointeur obtenu de vosk_model_new, libéré une seule fois.
        unsafe { (self.api.model_free)(self.ptr) }
    }
}

pub struct VoskTranscriber {
    api: &'static Api,
    /// Chemin du dossier du modèle, tel que Kaldi l'ouvrira.
    dir: CString,
    model: Mutex<Option<Model>>,
}

impl VoskTranscriber {
    /// `dir` : dossier du modèle ; `library` : libvosk. Sous Windows, `dir`
    /// doit être en ASCII (Kaldi ouvre les fichiers avec des chemins ANSI).
    pub fn load(dir: &Path, library: &Path) -> Result<Self, Error> {
        let dir = dir
            .to_str()
            .and_then(|d| CString::new(d).ok())
            .ok_or_else(|| Error::Asr(format!("chemin du modèle illisible : {}", dir.display())))?;
        let transcriber = Self {
            api: api(library)?,
            dir,
            model: Mutex::new(None),
        };
        transcriber.preload()?;
        Ok(transcriber)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Model>> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ensure_loaded<'a>(&self, slot: &'a mut Option<Model>) -> Result<&'a Model, Error> {
        if slot.is_none() {
            let started = Instant::now();
            // SAFETY: chaîne C valide pendant l'appel.
            let ptr = unsafe { (self.api.model_new)(self.dir.as_ptr()) };
            if ptr.is_null() {
                return Err(Error::Asr(format!(
                    "modèle Vosk illisible : {}",
                    self.dir.to_string_lossy()
                )));
            }
            log::info!("modèle Vosk chargé en {} ms", started.elapsed().as_millis());
            *slot = Some(Model { api: self.api, ptr });
        }
        Ok(slot.as_ref().expect("modèle chargé ci-dessus"))
    }

    fn recognize(&self, model: &Model, clip: &AudioClip) -> Result<String, Error> {
        let samples: Vec<i16> = clip
            .samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        let length = c_int::try_from(samples.len())
            .map_err(|_| Error::Asr("dictée trop longue pour Vosk".into()))?;
        // SAFETY: modèle valide (sous le verrou), reconnaisseur créé et
        // libéré ici ; le résultat est copié avant la libération.
        unsafe {
            let recognizer = (self.api.recognizer_new)(model.ptr, SAMPLE_RATE as c_float);
            if recognizer.is_null() {
                return Err(Error::Asr("création du reconnaisseur Vosk".into()));
            }
            (self.api.accept_waveform_s)(recognizer, samples.as_ptr(), length);
            let json = CStr::from_ptr((self.api.final_result)(recognizer))
                .to_string_lossy()
                .into_owned();
            (self.api.recognizer_free)(recognizer);
            text_of(&json)
        }
    }
}

/// Texte du résultat JSON de Vosk (`{"text": "…"}`), avec une majuscule
/// initiale et un point final.
fn text_of(json: &str) -> Result<String, Error> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| Error::Asr(format!("résultat Vosk : {e}")))?;
    let text = value["text"].as_str().unwrap_or_default().trim();
    let mut chars = text.chars();
    Ok(match chars.next() {
        None => String::new(),
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
    })
}

impl Transcriber for VoskTranscriber {
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
        self.recognize(model, clip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_gets_capital_and_period() {
        assert_eq!(
            text_of(r#"{"text" : "bonjour à tous"}"#).unwrap(),
            "Bonjour à tous."
        );
        assert_eq!(text_of(r#"{"text" : ""}"#).unwrap(), "");
        assert_eq!(text_of(r#"{"text" : "été"}"#).unwrap(), "Été.");
        assert!(text_of("pas du json").is_err());
    }
}
