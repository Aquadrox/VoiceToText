//! Recherche dans les transcriptions (réunions, conférences, fichiers) et
//! choix des passages à donner au LLM pour répondre à une question.
//!
//! Comparaison sans casse ni accents : « reunion » trouve « Réunion ».

use serde::Serialize;

use crate::meeting::{Segment, timestamp};

/// Minuscule sans accent, caractère pour caractère (les positions restent
/// alignées sur le texte d'origine).
fn fold_char(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'œ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).collect()
}

/// Mots recherchés (au moins deux caractères).
fn terms(query: &str) -> Vec<String> {
    fold(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_owned)
        .collect()
}

/// Texte découpé pour l'affichage : (morceau, surligné).
pub type Parts = Vec<(String, bool)>;

/// Le segment contient tous les mots recherchés : texte découpé avec les
/// occurrences surlignées. `None` sinon.
pub fn highlight(text: &str, query: &str) -> Option<Parts> {
    let terms = terms(query);
    if terms.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars.iter().map(|&c| fold_char(c)).collect();
    let mut marked = vec![false; chars.len()];
    for term in &terms {
        let needle: Vec<char> = term.chars().collect();
        let mut found = false;
        for start in 0..folded.len().saturating_sub(needle.len() - 1) {
            if folded[start..start + needle.len()] == needle[..] {
                found = true;
                marked[start..start + needle.len()].fill(true);
            }
        }
        if !found {
            return None;
        }
    }
    let mut parts: Parts = Vec::new();
    for (c, mark) in chars.into_iter().zip(marked) {
        match parts.last_mut() {
            Some((text, m)) if *m == mark => text.push(c),
            _ => parts.push((c.to_string(), mark)),
        }
    }
    Some(parts)
}

/// Résultat de recherche dans une transcription.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub start_ms: u64,
    pub parts: Parts,
}

/// Segments qui contiennent tous les mots de `query`, dans l'ordre.
pub fn find(segments: &[Segment], query: &str) -> Vec<Hit> {
    segments
        .iter()
        .filter_map(|s| {
            highlight(&s.text, query).map(|parts| Hit {
                start_ms: s.start_ms,
                parts,
            })
        })
        .collect()
}

/// Mots vides ignorés pour choisir les passages d'une question.
const STOPWORDS: &[&str] = &[
    "alors",
    "aussi",
    "avec",
    "avait",
    "avoir",
    "cela",
    "cette",
    "comment",
    "dans",
    "donc",
    "elle",
    "elles",
    "est-ce",
    "etait",
    "etre",
    "faire",
    "fait",
    "leur",
    "leurs",
    "mais",
    "meme",
    "nous",
    "pour",
    "pourquoi",
    "quand",
    "quel",
    "quelle",
    "quelles",
    "quels",
    "qu'est-ce",
    "sont",
    "sous",
    "tout",
    "tous",
    "tres",
    "vous",
    "what",
    "when",
    "where",
    "which",
    "with",
];

/// Mots significatifs d'une question (au moins quatre lettres, hors mots
/// vides).
pub fn keywords(question: &str) -> Vec<String> {
    fold(question)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4 && !STOPWORDS.contains(w))
        .map(str::to_owned)
        .collect()
}

/// Passages d'une transcription utiles pour une question : chaque segment
/// qui contient un mot significatif, avec ses voisins, sous la forme
/// `[titre · 12:34] texte`. Vide si rien ne correspond.
pub fn excerpts(title: &str, segments: &[Segment], keywords: &[String]) -> Vec<String> {
    let mut keep = vec![false; segments.len()];
    for (i, segment) in segments.iter().enumerate() {
        let text = fold(&segment.text);
        if keywords.iter().any(|k| text.contains(k.as_str())) {
            keep[i.saturating_sub(1)..=(i + 1).min(segments.len() - 1)].fill(true);
        }
    }
    segments
        .iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(s, _)| format!("[{title} · {}] {}", timestamp(s.start_ms), s.text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::Track;

    fn segment(start_ms: u64, text: &str) -> Segment {
        Segment {
            track: Track::Eux,
            start_ms,
            end_ms: start_ms + 5_000,
            text: text.into(),
            copied: false,
        }
    }

    #[test]
    fn search_ignores_case_and_accents_and_needs_every_word() {
        assert_eq!(
            highlight("La Réunion budgétaire", "reunion BUDGET").unwrap(),
            [
                ("La ".to_owned(), false),
                ("Réunion".to_owned(), true),
                (" ".to_owned(), false),
                ("budgét".to_owned(), true),
                ("aire".to_owned(), false),
            ]
        );
        assert_eq!(highlight("La réunion", "réunion annulée"), None);
        assert_eq!(highlight("Texte", " "), None);
        let hits = find(
            &[
                segment(0, "Budget validé."),
                segment(60_000, "Le budget, encore."),
            ],
            "budget",
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].start_ms, 60_000);
    }

    #[test]
    fn question_excerpts_keep_neighbours_of_matching_passages() {
        let segments = vec![
            segment(0, "Bonjour à tous."),
            segment(5_000, "Parlons du lancement."),
            segment(10_000, "Le budget de la campagne est validé."),
            segment(15_000, "Cinq mille euros."),
            segment(20_000, "Passons au recrutement."),
            segment(25_000, "Deux postes ouverts."),
        ];
        let words = keywords("Quel budget pour la campagne ?");
        assert_eq!(words, ["budget", "campagne"]);
        assert_eq!(
            excerpts("Point projet", &segments, &words),
            [
                "[Point projet · 00:05] Parlons du lancement.",
                "[Point projet · 00:10] Le budget de la campagne est validé.",
                "[Point projet · 00:15] Cinq mille euros.",
            ]
        );
        assert!(excerpts("X", &segments, &keywords("météo")).is_empty());
    }
}
