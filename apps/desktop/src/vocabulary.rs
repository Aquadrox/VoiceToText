//! Pages Dictionnaire, Snippets et Styles : lecture et enregistrement dans
//! config.toml, appliqués à chaud (sans recharger le modèle).

use serde::{Deserialize, Serialize};
use tauri::State;
use vtt_core::config::{self, Snippet, Style};

use crate::controller::ControllerState;
use crate::paths::Paths;
use crate::settings::SharedConfig;

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

#[tauri::command]
pub fn save_vocabulary(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    controller: State<'_, ControllerState>,
    update: VocabularyUpdate,
) -> Result<(), String> {
    // Repartir du fichier sur disque ; s'il est invalide, ne pas l'écraser.
    let (mut updated, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    if let Some(words) = update.dictionary {
        updated.dictionary = config::clean_dictionary(&words)?;
    }
    if let Some(snippets) = update.snippets {
        updated.snippets = config::clean_snippets(&snippets)?;
    }
    if let Some(styles) = update.styles {
        updated.styles = config::clean_styles(&styles)?;
    }
    paths.write_config(&updated)?;
    controller.apply(&updated);
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated;
    Ok(())
}
