//! Recherche dans les transcriptions (réunions, conférences, fichiers) et
//! choix des passages à donner au LLM pour répondre à une question.
//!
//! Comparaison sans casse ni accents : « reunion » trouve « Réunion ».

use serde::Serialize;

use crate::meeting::{MeetingReport, Segment, timestamp};

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
pub(crate) fn terms(query: &str) -> Vec<String> {
    fold(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_owned)
        .collect()
}

/// Texte découpé pour l'affichage : (morceau, surligné).
pub type Parts = Vec<(String, bool)>;

fn mark_parts(text: &str, terms: &[String], require_all: bool) -> Option<Parts> {
    if terms.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars.iter().map(|&c| fold_char(c)).collect();
    let mut marked = vec![false; chars.len()];
    for term in terms {
        let needle: Vec<char> = term.chars().collect();
        let mut found = false;
        for start in 0..folded.len().saturating_sub(needle.len() - 1) {
            if folded[start..start + needle.len()] == needle[..] {
                found = true;
                marked[start..start + needle.len()].fill(true);
            }
        }
        if require_all && !found {
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

/// Le segment contient tous les mots recherchés : texte découpé avec les
/// occurrences surlignées. `None` sinon.
pub fn highlight(text: &str, query: &str) -> Option<Parts> {
    mark_parts(text, &terms(query), true)
}

/// Découpe `text` en surlignant les mots de `query` présents, même si tous ne
/// figurent pas dans `text` (utile pour les résultats sémantiques).
pub fn highlight_partial(text: &str, query: &str) -> Parts {
    let t = terms(query);
    mark_parts(text, &t, false).unwrap_or_else(|| vec![(text.to_owned(), false)])
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

/// Résultat de recherche dans un rapport complet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportHit {
    pub section: &'static str,
    pub start_ms: Option<u64>,
    pub parts: Parts,
}

/// Cherche `query` (sans casse ni accents) dans toutes les sections d'un
/// rapport complet : synthèse, points clés, chapitres, décisions, citations,
/// analyse, questions-réponses et plan d'action.
pub fn find_in_report(report: &MeetingReport, query: &str) -> Vec<ReportHit> {
    let mut hits = Vec::new();
    let mut push = |section: &'static str, start_ms: Option<u64>, text: &str| {
        if let Some(parts) = highlight(text, query) {
            hits.push(ReportHit {
                section,
                start_ms,
                parts,
            });
        }
    };

    for paragraph in report
        .synthesis
        .split('\n')
        .filter(|p| !p.trim().is_empty())
    {
        push("Synthèse", None, paragraph.trim());
    }
    for point in &report.key_points {
        push("Point clé", point.at_ms, &point.text);
    }
    for chapter in &report.chapters {
        let start = Some(chapter.start_ms);
        push("Chapitre", start, &chapter.title);
        if !chapter.summary.is_empty() {
            push("Chapitre", start, &chapter.summary);
        }
        for point in &chapter.points {
            push("Chapitre", point.at_ms.or(start), &point.text);
        }
        for decision in &chapter.decisions {
            push("Décision", decision.at_ms.or(start), &decision.text);
        }
        for quote in &chapter.quotes {
            push("Citation", quote.at_ms.or(start), &quote.text);
        }
    }
    for (section, points) in [
        ("Accord", &report.agreements),
        ("Désaccord", &report.disagreements),
        ("Risque", &report.risks),
        ("Chiffre", &report.figures),
        ("Question ouverte", &report.open_questions),
    ] {
        for point in points {
            push(section, point.at_ms, &point.text);
        }
    }
    for qa in &report.qa {
        let combined = if qa.answer.is_empty() {
            qa.question.clone()
        } else {
            format!("{} — {}", qa.question, qa.answer)
        };
        push("Question / Réponse", qa.at_ms, &combined);
    }
    for action in &report.actions {
        let mut line = action.task.clone();
        if !action.owner.is_empty() {
            line.push_str(" (");
            line.push_str(&action.owner);
            if !action.due.is_empty() {
                line.push_str(", ");
                line.push_str(&action.due);
            }
            line.push(')');
        } else if !action.due.is_empty() {
            line.push_str(" (");
            line.push_str(&action.due);
            line.push(')');
        }
        push("Action", action.at_ms, &line);
    }
    hits
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
/// `names` : noms des intervenants ; une réplique d'un intervenant distingué
/// est précédée de son nom.
pub fn excerpts(
    title: &str,
    segments: &[Segment],
    keywords: &[String],
    names: &[String],
) -> Vec<String> {
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
        .map(|(s, _)| match s.speaker {
            Some(_) => format!(
                "[{title} · {}] {} : {}",
                timestamp(s.start_ms),
                crate::meeting::speaker_label(s.track, s.speaker, names),
                s.text
            ),
            None => format!("[{title} · {}] {}", timestamp(s.start_ms), s.text),
        })
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
            speaker: None,
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
            excerpts("Point projet", &segments, &words, &[]),
            [
                "[Point projet · 00:05] Parlons du lancement.",
                "[Point projet · 00:10] Le budget de la campagne est validé.",
                "[Point projet · 00:15] Cinq mille euros.",
            ]
        );
        assert!(excerpts("X", &segments, &keywords("météo"), &[]).is_empty());
        // Intervenant distingué : son nom devant la réplique.
        let named = vec![Segment {
            speaker: Some(1),
            ..segment(10_000, "Le budget est validé.")
        }];
        let names = vec![String::new(), "Anne".to_owned()];
        assert_eq!(
            excerpts("P", &named, &keywords("budget"), &names),
            ["[P · 00:10] Anne : Le budget est validé."]
        );
    }

    #[test]
    fn find_in_report_searches_synthesis_chapters_and_actions() {
        let mut report = MeetingReport::from_json(
            &serde_json::json!({
                "synthese": "Validation du budget marketing pour la rentrée.",
                "chapitres": [{
                    "titre": "Lancement produit",
                    "debut": "01:00",
                    "resume": "Calendrier de mise en production.",
                    "decisions": [{"texte": "Budget plafonné à 10 k€", "t": "01:30"}]
                }],
                "risques": [{"texte": "Délai serré sur les traductions", "t": "02:10"}],
                "actions": [{"tache": "Envoyer le budget final", "responsable": "Julie", "echeance": "vendredi", "t": "03:00"}]
            }),
            5 * 60_000,
        );
        report.finish(5 * 60_000);

        let hits = find_in_report(&report, "budget");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].section, "Synthèse");
        assert_eq!(hits[0].start_ms, None);
        assert_eq!(hits[1].section, "Décision");
        assert_eq!(hits[1].start_ms, Some(90_000));
        assert_eq!(hits[2].section, "Action");
        assert_eq!(hits[2].start_ms, Some(180_000));

        // Recherche sur le responsable d'une action, sans casse ni accents.
        let julie = find_in_report(&report, "julie vendredi");
        assert_eq!(julie.len(), 1);
        assert_eq!(julie[0].section, "Action");
    }
}
