//! Pages Dictionnaire, Snippets et Styles : lecture et enregistrement dans
//! config.toml, appliqués à chaud (sans recharger le modèle). Suggestions
//! pour le dictionnaire tirées de l'historique local.

use serde::{Deserialize, Serialize};
use tauri::State;
use vtt_core::Config;
use vtt_core::config::{self, Replacement, Snippet, Style};
use vtt_core::suggest::{self, Suggestion};

use crate::controller::ControllerState;
use crate::history::HistoryStore;
use crate::paths::Paths;
use crate::settings::{self, SharedConfig};

#[derive(Serialize)]
pub struct VocabularyView {
    dictionary: Vec<String>,
    replacements: Vec<Replacement>,
    snippets: Vec<Snippet>,
    styles: Vec<Style>,
}

fn load_vocabulary_view(paths: &Paths) -> Result<VocabularyView, String> {
    let (config, error) = paths.load_config();
    if let Some(error) = error {
        return Err(error);
    }
    Ok(VocabularyView {
        dictionary: config.dictionary,
        replacements: config.replacements,
        snippets: config.snippets,
        styles: config.styles,
    })
}

#[tauri::command]
pub fn vocabulary_view(paths: State<'_, Paths>) -> Result<VocabularyView, String> {
    load_vocabulary_view(&paths)
}

/// Sections à remplacer ; une section absente reste telle quelle.
#[derive(Deserialize)]
pub struct VocabularyUpdate {
    dictionary: Option<Vec<String>>,
    #[serde(default)]
    replacements: Option<Vec<Replacement>>,
    snippets: Option<Vec<Snippet>>,
    styles: Option<Vec<Style>>,
}

fn apply_vocabulary_update(paths: &Paths, update: VocabularyUpdate) -> Result<Config, String> {
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
    if let Some(replacements) = update.replacements {
        updated.replacements = config::clean_replacements(&replacements)?;
    }
    if let Some(snippets) = update.snippets {
        updated.snippets = config::clean_snippets(&snippets)?;
    }
    if let Some(styles) = update.styles {
        updated.styles = config::clean_styles(&styles)?;
    }
    paths.write_config(&updated)?;
    Ok(updated)
}

#[tauri::command]
pub fn save_vocabulary(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    controller: State<'_, ControllerState>,
    update: VocabularyUpdate,
) -> Result<(), String> {
    let updated = apply_vocabulary_update(&paths, update)?;
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

fn compute_suggestions(
    dictionary: &[String],
    ignored: &[String],
    history: &HistoryStore,
) -> Result<SuggestionsView, String> {
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
        suggestions: suggest::suggestions(&entries, dictionary, ignored, SUGGESTIONS),
    })
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
    compute_suggestions(&dictionary, &ignored, &history)
}

fn apply_ignore_suggestion(paths: &Paths, word: &str) -> Result<Config, String> {
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
    Ok(updated)
}

/// Écarte une suggestion : elle n'est plus proposée.
#[tauri::command]
pub fn ignore_suggestion(
    paths: State<'_, Paths>,
    config: State<'_, SharedConfig>,
    word: String,
) -> Result<(), String> {
    let updated = apply_ignore_suggestion(&paths, &word)?;
    *config.write().unwrap_or_else(|e| e.into_inner()) = updated;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use vtt_core::history::{Entry, History};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("vtt-vocab-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(raw: &str, text: &str) -> Entry {
        Entry {
            at_ms: 1,
            duration_ms: 1_000,
            raw: raw.into(),
            text: text.into(),
            snippet: false,
            cleanup_failed: false,
            command: false,
            app: None,
            usage: None,
            cost: None,
            currency: None,
            fallback: None,
            translated: None,
        }
    }

    #[test]
    fn vocabulary_view_reads_sections_and_rejects_invalid_toml() {
        let dir = temp_dir("view");
        let paths = Paths::for_tests(&dir);
        let mut config = Config::default();
        config.dictionary.push("Aquadrox".into());
        config.snippets.push(Snippet {
            trigger: "ma signature".into(),
            text: "Cordialement,\nJean".into(),
        });
        paths.write_config(&config).unwrap();

        let view = load_vocabulary_view(&paths).unwrap();
        assert_eq!(view.dictionary, vec!["Aquadrox"]);
        assert_eq!(view.snippets.len(), 1);
        assert!(view.styles.is_empty());

        fs::write(&paths.config_file, "invalide = = =").unwrap();
        assert!(load_vocabulary_view(&paths).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn vocabulary_update_replaces_only_given_sections() {
        let dir = temp_dir("update");
        let paths = Paths::for_tests(&dir);
        let replacement = Replacement {
            heard: "poste graisse".into(),
            written: "PostgreSQL".into(),
        };
        let snippet = Snippet {
            trigger: "sig".into(),
            text: "Salut".into(),
        };
        let style = Style {
            apps: vec!["slack.exe".into()],
            titles: Vec::new(),
            instructions: "ton direct".into(),
            cleanup: true,
        };
        apply_vocabulary_update(
            &paths,
            VocabularyUpdate {
                dictionary: Some(vec!["Aquadrox".into(), "  ".into()]),
                replacements: Some(vec![replacement.clone()]),
                snippets: Some(vec![snippet.clone()]),
                styles: Some(vec![style.clone()]),
            },
        )
        .unwrap();

        // Mise à jour du dictionnaire seul : corrections, snippets et styles conservés.
        let updated = apply_vocabulary_update(
            &paths,
            VocabularyUpdate {
                dictionary: Some(vec!["VoiceToText".into()]),
                replacements: None,
                snippets: None,
                styles: None,
            },
        )
        .unwrap();
        assert_eq!(updated.dictionary, vec!["VoiceToText"]);
        assert_eq!(updated.replacements, vec![replacement]);
        assert_eq!(updated.snippets, vec![snippet]);
        assert_eq!(updated.styles, vec![style]);

        // Une erreur de validation ne corrompt pas config.toml.
        assert!(
            apply_vocabulary_update(
                &paths,
                VocabularyUpdate {
                    dictionary: None,
                    replacements: None,
                    snippets: Some(vec![Snippet {
                        trigger: "   ".into(),
                        text: "ok".into(),
                    }]),
                    styles: None,
                },
            )
            .is_err()
        );
        let view = load_vocabulary_view(&paths).unwrap();
        assert_eq!(view.replacements.len(), 1);
        assert_eq!(view.snippets.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggestions_handle_disabled_history_and_exclude_ignored_words() {
        let disabled: HistoryStore = Arc::new(Mutex::new(None));
        let view = compute_suggestions(&[], &[], &disabled).unwrap();
        assert!(!view.history_enabled);
        assert!(view.suggestions.is_empty());

        let dir = temp_dir("suggest");
        let mut history = History::open(dir.join("history.jsonl"), 100);
        history
            .append(&entry(
                "on utilise aquatroc sur le serveur",
                "On utilise Aquadrox sur le serveur.",
            ))
            .unwrap();
        history
            .append(&entry(
                "bonjour aquatroc demain",
                "Bonjour Aquadrox demain.",
            ))
            .unwrap();
        let store: HistoryStore = Arc::new(Mutex::new(Some(history)));

        let view = compute_suggestions(&[], &[], &store).unwrap();
        assert!(view.history_enabled);
        assert!(view.suggestions.iter().any(|s| s.word == "Aquadrox"));

        let filtered = compute_suggestions(&[], &["Aquadrox".into()], &store).unwrap();
        assert!(!filtered.suggestions.iter().any(|s| s.word == "Aquadrox"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignore_suggestion_validates_and_deduplicates_case_insensitively() {
        let dir = temp_dir("ignore");
        let paths = Paths::for_tests(&dir);

        assert!(apply_ignore_suggestion(&paths, "   ").is_err());
        assert!(apply_ignore_suggestion(&paths, &"a".repeat(81)).is_err());

        let first = apply_ignore_suggestion(&paths, "  Aquadrox ").unwrap();
        assert_eq!(first.dictionary_ignored, vec!["Aquadrox"]);

        let second = apply_ignore_suggestion(&paths, "aquadrox").unwrap();
        assert_eq!(second.dictionary_ignored, vec!["Aquadrox"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
