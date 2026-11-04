//! Suggestions pour le dictionnaire, tirées de l'historique : noms propres,
//! sigles et mots techniques qui reviennent, et mots que le LLM a dû
//! corriger (Whisper les a mal orthographiés).
//!
//! Tout se passe sur le PC, à partir de l'historique local.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::history::Entry;

/// Mots trop courts pour être utiles (« IA », « OK »…).
const MIN_CHARS: usize = 3;
/// Un mot non corrigé doit revenir dans au moins ce nombre de dictées.
const MIN_DICTATIONS: usize = 2;
/// Majuscules courantes qui ne sont pas des noms à apprendre.
const COMMON: &[&str] = &[
    "bonjour",
    "bonsoir",
    "cordialement",
    "madame",
    "merci",
    "monsieur",
    "salut",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub word: String,
    /// Nombre de dictées où le mot apparaît.
    pub count: usize,
    /// Le LLM l'a écrit alors que Whisper avait transcrit autre chose.
    pub corrected: bool,
}

/// Mots candidats de `text` : nom propre hors début de phrase (`Paris`),
/// sigle (`JSON`) ou casse mixte (`GitHub`, `iPhone`).
fn candidates(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut sentence_start = true;
    for token in text.split_whitespace() {
        let word = token.trim_matches(|c: char| !c.is_alphanumeric());
        let ends_sentence = token.ends_with(['.', '!', '?', ':']);
        if word.chars().count() >= MIN_CHARS
            && word.chars().any(char::is_alphabetic)
            && !COMMON.contains(&word.to_lowercase().as_str())
        {
            let mut chars = word.chars();
            let first_upper = chars.next().is_some_and(char::is_uppercase);
            let inner_upper = chars.any(char::is_uppercase);
            if inner_upper || (first_upper && !sentence_start) {
                found.push(word);
            }
        }
        sentence_start = ends_sentence;
    }
    found
}

/// Mots en minuscules de `text`, pour savoir si Whisper avait déjà le mot.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '.')
        .map(|w| w.trim_matches('.').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Suggestions, les plus utiles d'abord (mots corrigés, puis les plus
/// fréquents). Les mots déjà au dictionnaire ou écartés sont exclus, casse
/// ignorée. Les commandes, snippets, nettoyages échoués et traductions
/// (texte dans une autre langue) ne comptent pas.
pub fn suggestions(
    entries: &[Entry],
    dictionary: &[String],
    ignored: &[String],
    limit: usize,
) -> Vec<Suggestion> {
    let known: HashSet<String> = dictionary
        .iter()
        .chain(ignored)
        .map(|w| w.trim().to_lowercase())
        .collect();
    // Clé en minuscules → (orthographe la plus vue, nombre par orthographe,
    // dictées, corrigé).
    struct Seen {
        spellings: HashMap<String, usize>,
        count: usize,
        corrected: bool,
    }
    let mut seen: HashMap<String, Seen> = HashMap::new();
    for entry in entries {
        if entry.command || entry.snippet || entry.cleanup_failed || entry.translated.is_some() {
            continue;
        }
        let raw = words(&entry.raw);
        let mut in_entry = HashSet::new();
        for word in candidates(&entry.text) {
            let key = word.to_lowercase();
            if known.contains(&key) || !in_entry.insert(key.clone()) {
                continue;
            }
            let item = seen.entry(key.clone()).or_insert_with(|| Seen {
                spellings: HashMap::new(),
                count: 0,
                corrected: false,
            });
            *item.spellings.entry(word.to_owned()).or_default() += 1;
            item.count += 1;
            item.corrected |= !raw.contains(&key);
        }
    }
    let mut found: Vec<Suggestion> = seen
        .into_values()
        .filter(|s| s.corrected || s.count >= MIN_DICTATIONS)
        .map(|s| {
            let word = s
                .spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(w, _)| w)
                .unwrap_or_default();
            Suggestion {
                word,
                count: s.count,
                corrected: s.corrected,
            }
        })
        .collect();
    found.sort_by(|a, b| {
        b.corrected
            .cmp(&a.corrected)
            .then(b.count.cmp(&a.count))
            .then_with(|| a.word.to_lowercase().cmp(&b.word.to_lowercase()))
    });
    found.truncate(limit);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(raw: &str, text: &str) -> Entry {
        Entry {
            at_ms: 0,
            duration_ms: 1000,
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
    fn candidates_skip_sentence_starts_and_short_words() {
        assert_eq!(
            candidates("Demain je vois Paul à Lyon. Ensuite OK pour le JSON sur GitHub."),
            ["Paul", "Lyon", "JSON", "GitHub"]
        );
        assert_eq!(candidates("Bonjour Monsieur, merci."), Vec::<&str>::new());
    }

    #[test]
    fn corrected_words_come_first() {
        let entries = [
            entry("envoie le jason à paul", "Envoie le JSON à Paul."),
            entry("paul est là", "Ensuite Paul est là."),
            entry("lyon", "Je pars à Lyon."),
        ];
        let found = suggestions(&entries, &[], &[], 10);
        assert_eq!(
            found,
            [
                Suggestion {
                    word: "JSON".into(),
                    count: 1,
                    corrected: true
                },
                Suggestion {
                    word: "Paul".into(),
                    count: 2,
                    corrected: false
                },
            ],
            "Lyon : une seule dictée, déjà bien transcrit"
        );
    }

    #[test]
    fn known_ignored_and_special_entries_are_excluded() {
        let mut command = entry("jason", "Le JSON.");
        command.command = true;
        let mut translated = entry("jason", "Send the JSON to Paul.");
        translated.translated = Some("anglais".into());
        let entries = [
            entry("le jason", "Le JSON."),
            entry("vu avec kubernetes et paul", "Vu avec Kubernetes et Paul."),
            command,
            translated,
        ];
        let found = suggestions(&entries, &["json".into()], &["Kubernetes".into()], 10);
        assert!(found.is_empty(), "{found:?}");
    }
}
