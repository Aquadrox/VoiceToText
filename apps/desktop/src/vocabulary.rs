//! Pages Dictionnaire, Snippets et Styles : lecture et enregistrement dans
//! config.toml, appliqués à chaud (sans recharger le modèle).

use serde::{Deserialize, Serialize};
use tauri::State;
use vtt_core::PostProcessor;
use vtt_core::config::{self, Snippet, Style};
use vtt_llm::LlmCleaner;

use crate::controller::{Reconfiguration, Work, WorkerHandle};
use crate::paths::Paths;
use crate::settings;

#[derive(Serialize)]
pub struct VocabularyView {
    dictionary: Vec<String>,
    snippets: Vec<Snippet>,
    styles: Vec<Style>,
}

#[tauri::command]
pub fn vocabulary_view(paths: State<'_, Paths>) -> Result<VocabularyView, String> {
    let (config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(error);
    }
    Ok(VocabularyView {
        dictionary: config.dictionary,
        snippets: config.snippets,
        styles: config.styles,
    })
}

/// Sections à remplacer ; une section absente reste telle quelle.
#[derive(Deserialize)]
pub struct VocabularyUpdate {
    dictionary: Option<Vec<String>>,
    snippets: Option<Vec<Snippet>>,
    styles: Option<Vec<Style>>,
}

#[derive(Serialize)]
pub struct Saved {
    /// `false` : modèle encore en chargement, appliqué au prochain démarrage.
    applied: bool,
}

#[tauri::command]
pub fn save_vocabulary(
    paths: State<'_, Paths>,
    worker: State<'_, WorkerHandle>,
    update: VocabularyUpdate,
) -> Result<Saved, String> {
    // Repartir du fichier sur disque ; s'il est invalide, ne pas l'écraser.
    let (mut config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    if let Some(words) = update.dictionary {
        config.dictionary = config::clean_dictionary(&words)?;
    }
    if let Some(snippets) = update.snippets {
        config.snippets = config::clean_snippets(&snippets)?;
    }
    if let Some(styles) = update.styles {
        config.styles = config::clean_styles(&styles)?;
    }
    paths.write_config(&config)?;

    let post_processor: Option<Box<dyn PostProcessor>> = config.llm.enabled.then(|| {
        Box::new(LlmCleaner::new(
            &config.llm,
            &config.dictionary,
            settings::api_key_provider(&config.llm),
        )) as Box<dyn PostProcessor>
    });
    let sender = worker.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let applied = sender.is_some_and(|sender| {
        sender
            .send(Work::Reconfigure(Reconfiguration {
                dictionary: config.dictionary,
                snippets: config.snippets,
                styles: config.styles,
                post_processor,
            }))
            .is_ok()
    });
    Ok(Saved { applied })
}
