//! Pages Dictionnaire, Snippets et Styles : lecture et enregistrement dans
//! config.toml, appliqués à chaud (sans recharger le modèle). Suggestions
//! pour le dictionnaire tirées de l'historique local.

use serde::{Deserialize, Serialize};
use tauri::State;
use vtt_core::config::{self, Snippet, Style};
use vtt_core::suggest::{self, Suggestion};

use crate::controller::ControllerState;
use crate::history::HistoryStore;
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};

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

/// Nombre de suggestions affichées.
const SUGGESTIONS: usize = 20;

#[derive(Serialize)]
pub struct SuggestionsView {
    /// Historique désactivé : rien à analyser.
    history_enabled: bool,
    suggestions: Vec<Suggestion>,
}

/// Mots à ajouter au dictionnaire, tirés de l'historique (sur ce PC).
#[tauri::command(async)]
pub fn dictionary_suggestions(
    config: State<'_, SharedConfig>,
    history: State<'_, HistoryStore>,
) -> Result<SuggestionsView, String> {
    let (dictionary, ignored) = {
        let config = settings::read(&config);
        (config.dictionary.clone(), config.dictionary_ignored.clone())
    };
    let entries = {
        let guard = history.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            None => {
                return Ok(SuggestionsView {
                    history_enabled: false,
                    suggestions: Vec::new(),
                });
            }
            Some(history) => history.recent(usize::MAX).map_err(|e| e.to_string())?,
        }
    };
    Ok(SuggestionsView {
        history_enabled: true,
        suggestions: suggest::suggestions(&entries, &dictionary, &ignored, SUGGESTIONS),
    })
}

/// Écarte une suggestion : elle n'est plus proposée.
#[tauri::command]
pub fn ignore_suggestion(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    word: String,
) -> Result<(), String> {
    let word = word.trim();
    if word.is_empty() || word.chars().count() > 80 {
        return Err("mot invalide".into());
    }
    let (mut updated, error) = paths.load_config();
    if let Some(error) = error {
        return Err(format!(
            "{error} : corrigez-le d'abord (menu « Modifier la configuration »)"
        ));
    }
    if !updated
        .dictionary_ignored
        .iter()
        .any(|w| w.eq_ignore_ascii_case(word))
    {
        updated.dictionary_ignored.push(word.to_owned());
    }
    paths.write_config(&updated)?;
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated;
    Ok(())
}
