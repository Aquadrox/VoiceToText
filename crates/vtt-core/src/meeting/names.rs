//! Noms des intervenants trouvés dans ce qu'ils disent : quelqu'un qui se
//! présente (« Bonjour, je m'appelle Marie », « Moi c'est Paul », « Marie
//! Dupont à l'appareil ») est nommé à la place de « Intervenant N ».
//!
//! Tout se fait sur le texte transcrit, localement, sans LLM. Des règles
//! prudentes : mieux vaut pas de nom qu'un faux nom. Le nom trouvé reste une
//! proposition, montrée avec la phrase d'où il vient ; un nom donné par
//! l'utilisateur passe toujours devant.

use serde::{Deserialize, Serialize};

use super::{Segment, turns};
use crate::search::fold;

/// Nom trouvé pour un intervenant, avec sa preuve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameGuess {
    pub name: String,
    /// Début de la réplique où l'intervenant se présente.
    pub at_ms: u64,
    /// La phrase (ou son début) où il se présente.
    pub quote: String,
}

/// Confiance dans une présentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Strength {
    /// « Je suis Marie » : « je suis » sert à bien d'autres choses, le nom
    /// doit porter une majuscule.
    Weak = 1,
    /// « Moi c'est Marie », « Marie à l'appareil », « Bonjour, c'est Marie ».
    Medium = 2,
    /// « Je m'appelle Marie », « mon nom est Dupont » : même sans majuscule
    /// (Vosk écrit tout en minuscules).
    Strong = 3,
}

/// Mot transcrit, avec ce qui le sépare du précédent.
#[derive(Debug)]
struct Word<'a> {
    text: &'a str,
    /// Minuscules, apostrophe droite.
    norm: String,
    start: usize,
    end: usize,
    /// Ponctuation entre le mot précédent et celui-ci (espaces exclus).
    sep: String,
}

impl Word<'_> {
    /// Le mot précédent est dans une autre phrase.
    fn new_sentence(&self) -> bool {
        self.sep.contains(['.', '!', '?', '…', ';'])
    }

    /// Rien d'autre qu'un espace (ou une virgule, si `comma`) avant ce mot.
    fn joined(&self, comma: bool) -> bool {
        self.sep.is_empty() || (comma && self.sep == ",")
    }
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// Mots du texte : lettres et chiffres, avec l'apostrophe et le trait
/// d'union à l'intérieur d'un mot (« m'appelle », « Jean-Pierre »).
fn words(text: &str) -> Vec<Word<'_>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut words = Vec::new();
    let mut sep = String::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !c.is_alphanumeric() {
            if !c.is_whitespace() {
                sep.push(c);
            }
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j].1;
            let inner = (is_apostrophe(c) || c == '-')
                && chars.get(j + 1).is_some_and(|(_, n)| n.is_alphanumeric());
            if c.is_alphanumeric() || inner {
                j += 1;
            } else {
                break;
            }
        }
        let end = chars.get(j).map_or(text.len(), |(at, _)| *at);
        let word = &text[start..end];
        words.push(Word {
            text: word,
            norm: word.replace('’', "'").to_lowercase(),
            start,
            end,
            sep: std::mem::take(&mut sep),
        });
        i = j;
    }
    words
}

/// Mots qui suivent souvent « je suis », « c'est »… sans être des noms, ou
/// qui commencent une phrase avec une majuscule (comparés sans accents).
const NOT_NAMES: &[&str] = &[
    // Français
    "a",
    "afin",
    "ah",
    "ainsi",
    "alors",
    "allo",
    "apres",
    "assez",
    "au",
    "aujourd'hui",
    "aussi",
    "autre",
    "aux",
    "avant",
    "avec",
    "bah",
    "beaucoup",
    "ben",
    "bien",
    "bon",
    "bonjour",
    "bonne",
    "bonsoir",
    "bref",
    "c'est",
    "ca",
    "car",
    "ce",
    "cela",
    "celui",
    "celle",
    "ces",
    "cet",
    "cette",
    "chez",
    "ci",
    "comme",
    "comment",
    "content",
    "contente",
    "coucou",
    "d'accord",
    "dans",
    "de",
    "deja",
    "derriere",
    "des",
    "desole",
    "desolee",
    "devant",
    "donc",
    "du",
    "elle",
    "en",
    "encore",
    "enfin",
    "entre",
    "est",
    "et",
    "eux",
    "euh",
    "hein",
    "heureux",
    "heureuse",
    "hum",
    "ici",
    "il",
    "ils",
    "j'ai",
    "jamais",
    "je",
    "juste",
    "l'a",
    "la",
    "le",
    "les",
    "leur",
    "lui",
    "mais",
    "mal",
    "meme",
    "merci",
    "mes",
    "moi",
    "mon",
    "ne",
    "ni",
    "nos",
    "notre",
    "nous",
    "oh",
    "ok",
    "okay",
    "on",
    "ou",
    "oui",
    "par",
    "pareil",
    "parti",
    "partie",
    "pas",
    "pendant",
    "personne",
    "peu",
    "plus",
    "plutot",
    "pour",
    "pourquoi",
    "pret",
    "prete",
    "quand",
    "que",
    "quel",
    "quelle",
    "qui",
    "quoi",
    "ravi",
    "ravie",
    "rien",
    "sa",
    "salut",
    "sans",
    "se",
    "ses",
    "si",
    "son",
    "sous",
    "sur",
    "ta",
    "tes",
    "toi",
    "ton",
    "toujours",
    "tous",
    "tout",
    "toute",
    "toutes",
    "tres",
    "tu",
    "un",
    "une",
    "vers",
    "voila",
    "vos",
    "votre",
    "vous",
    "vraiment",
    "y",
    // Anglais
    "about",
    "afraid",
    "all",
    "also",
    "am",
    "an",
    "and",
    "are",
    "at",
    "back",
    "but",
    "calling",
    "fine",
    "from",
    "glad",
    "going",
    "good",
    "great",
    "happy",
    "hello",
    "here",
    "hey",
    "hi",
    "in",
    "is",
    "it",
    "just",
    "not",
    "now",
    "of",
    "on",
    "really",
    "so",
    "sorry",
    "sure",
    "thank",
    "thanks",
    "that",
    "the",
    "there",
    "this",
    "to",
    "trying",
    "very",
    "well",
    "with",
    "yeah",
    "yes",
];

/// Civilités gardées devant le nom (« Madame Dupont »), jamais seules.
const TITLES: &[&str] = &[
    "madame",
    "monsieur",
    "mademoiselle",
    "mme",
    "mlle",
    "docteur",
    "dr",
    "maitre",
    "professeur",
    "mr",
    "mrs",
    "ms",
    "miss",
];

/// Mots d'accueil avant « c'est Marie » / « this is Paul ».
const GREETINGS: &[&str] = &[
    "allo", "bonjour", "bonsoir", "coucou", "hello", "hey", "hi", "salut",
];

fn not_a_name(word: &Word) -> bool {
    NOT_NAMES.contains(&fold(&word.norm).as_str())
}

fn is_title(word: &Word) -> bool {
    TITLES.contains(&fold(&word.norm).as_str())
}

/// Mot écrit comme un nom propre : majuscule puis minuscules (« Marie »,
/// « Jean-Pierre », « O'Neil ») ; ni sigle (« RH »), ni chiffre.
fn capitalized(word: &Word) -> bool {
    let mut chars = word.text.chars();
    chars.next().is_some_and(char::is_uppercase)
        && word.text.chars().any(char::is_lowercase)
        && word
            .text
            .chars()
            .all(|c| c.is_alphabetic() || c == '-' || is_apostrophe(c))
}

/// « jean-pierre » → « Jean-Pierre ».
fn capitalize(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut upper = true;
    for c in word.chars() {
        if upper {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        upper = c == '-';
    }
    out
}

/// Prénom (et nom) qui commence au mot `at` : jusqu'à trois mots à
/// majuscule, collés, avec une civilité éventuelle devant. Sans majuscule
/// (`lowercase`), un seul mot, remis en forme. `None` : pas un nom, ou un
/// nom suivi de « qui », « que » (« c'est Paul qui s'en occupe » : on parle
/// de quelqu'un d'autre).
fn name_at(words: &[Word], at: usize, lowercase: bool) -> Option<(String, usize)> {
    const MAX_WORDS: usize = 3;
    let mut i = at;
    if words.get(i).is_some_and(|w| is_title(w) && capitalized(w)) {
        i += 1;
    }
    let first = i;
    while let Some(word) = words.get(i) {
        if i - first >= MAX_WORDS
            || (i > at && !word.joined(false))
            || !capitalized(word)
            || not_a_name(word)
            || is_title(word)
        {
            break;
        }
        i += 1;
    }
    let (name, last) = if i > first {
        let parts: Vec<&str> = words[at..i].iter().map(|w| w.text).collect();
        (parts.join(" "), i - 1)
    } else {
        // Sans majuscule : un seul mot, sans civilité devant.
        let word = words.get(at)?;
        let plain = word.text.chars().count() >= 2
            && word.text.chars().all(|c| c.is_alphabetic() || c == '-')
            && !not_a_name(word)
            && !is_title(word);
        if !(lowercase && plain && first == at) {
            return None;
        }
        (capitalize(word.text), at)
    };
    if words.get(last + 1).is_some_and(|w| {
        w.joined(false)
            && ["qui", "que", "qu'il", "qu'elle", "qu'on", "who", "that"].contains(&w.norm.as_str())
    }) {
        return None;
    }
    Some((name, words[last].end))
}

/// Les mots à partir de `at` sont exactement `pattern` (virgules permises
/// entre eux, « moi, c'est »).
fn matches(words: &[Word], at: usize, pattern: &[&str]) -> bool {
    pattern.iter().enumerate().all(|(k, p)| {
        words
            .get(at + k)
            .is_some_and(|w| w.norm == *p && (k == 0 || w.joined(true)))
    })
}

/// Formules suivies du nom.
const BEFORE_NAME: &[(&[&str], Strength)] = &[
    (&["je", "m'appelle"], Strength::Strong),
    (&["je", "me", "nomme"], Strength::Strong),
    (&["mon", "nom", "est"], Strength::Strong),
    (&["mon", "nom", "c'est"], Strength::Strong),
    (&["mon", "prénom", "est"], Strength::Strong),
    (&["mon", "prénom", "c'est"], Strength::Strong),
    (&["my", "name", "is"], Strength::Strong),
    (&["moi", "c'est"], Strength::Medium),
    (&["je", "me", "présente"], Strength::Medium),
    (&["je", "suis"], Strength::Weak),
    (&["i'm"], Strength::Weak),
    (&["i", "am"], Strength::Weak),
];

/// Après un mot d'accueil : « Bonjour, c'est Marie », « Hi, this is Paul ».
const AFTER_GREETING: &[&[&str]] = &[&["c'est"], &["ici"], &["this", "is"], &["it's"]];

/// Présentation trouvée dans un texte : nom, force, début de la formule et
/// fin du nom (octets).
fn introductions(text: &str) -> Vec<(String, Strength, usize, usize)> {
    let words = words(text);
    let mut found = Vec::new();
    for at in 0..words.len() {
        for (pattern, strength) in BEFORE_NAME {
            let next = at + pattern.len();
            // « Je suis » en milieu de phrase, après « si », « quand »… :
            // pas une présentation.
            if !matches(&words, at, pattern) || words.get(next).is_none_or(|w| w.new_sentence()) {
                continue;
            }
            let lowercase = *strength == Strength::Strong;
            if let Some((name, end)) = name_at(&words, next, lowercase) {
                found.push((name, *strength, words[at].start, end));
            }
        }
        let greeted = at > 0 && GREETINGS.contains(&fold(&words[at - 1].norm).as_str());
        for pattern in AFTER_GREETING {
            let next = at + pattern.len();
            if greeted
                && matches(&words, at, pattern)
                && words.get(next).is_some_and(|w| w.joined(false))
                && let Some((name, end)) = name_at(&words, next, false)
            {
                found.push((name, Strength::Medium, words[at].start, end));
            }
        }
        // « Marie Dupont à l'appareil » : le nom avant.
        if matches(&words, at, &["à", "l'appareil"]) && at > 0 && words[at].joined(false) {
            let mut first = at;
            while first > 0
                && at - first < 3
                && capitalized(&words[first - 1])
                && !not_a_name(&words[first - 1])
                && (first == at || words[first].joined(false))
            {
                first -= 1;
            }
            if words[first..at].iter().any(|w| !is_title(w)) {
                let name = words[first..at]
                    .iter()
                    .map(|w| w.text)
                    .collect::<Vec<_>>()
                    .join(" ");
                found.push((
                    name,
                    Strength::Medium,
                    words[first].start,
                    words[at + 1].end,
                ));
            }
        }
    }
    found
}

/// Phrase autour de `start..end` (octets), coupée entre deux mots si elle
/// est longue.
fn quote(text: &str, start: usize, end: usize) -> String {
    const AROUND: usize = 40;
    let is_end = |c: char| matches!(c, '.' | '!' | '?' | '…');
    let sentence_start = text[..start]
        .char_indices()
        .rfind(|(_, c)| is_end(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let sentence_end = text[end..]
        .char_indices()
        .find(|(_, c)| is_end(*c))
        .map_or(text.len(), |(i, c)| end + i + c.len_utf8());
    let head = text[sentence_start..start].trim_start();
    let tail = text[end..sentence_end].trim_end();
    let mut out = String::new();
    if head.chars().count() > AROUND {
        let skip = head.chars().count() - AROUND;
        let cut = head.char_indices().nth(skip).map_or(0, |(i, _)| i);
        // Au début du mot suivant.
        let cut = head[cut..].find(' ').map_or(cut, |i| cut + i + 1);
        out.push('…');
        out.push_str(&head[cut..]);
    } else {
        out.push_str(head);
    }
    out.push_str(&text[start..end]);
    if tail.chars().count() > AROUND {
        let cut = tail
            .char_indices()
            .nth(AROUND)
            .map_or(tail.len(), |(i, _)| i);
        // À la fin du mot précédent.
        let cut = tail[..cut].rfind(' ').unwrap_or(cut);
        out.push_str(tail[..cut].trim_end_matches([',', ' ']));
        out.push('…');
    } else {
        out.push_str(tail);
    }
    out
}

/// Clé de comparaison d'un nom : son premier mot hors civilité, sans
/// casse ni accents (« Marie » et « Marie Dupont » sont la même personne).
fn key(name: &str) -> String {
    let words = words(name);
    words
        .iter()
        .find(|w| !is_title(w))
        .or(words.first())
        .map(|w| fold(&w.norm))
        .unwrap_or_default()
}

/// Présentations d'un intervenant pour un même nom.
struct Candidate {
    speaker: usize,
    key: String,
    guess: NameGuess,
    score: u32,
    first_ms: u64,
}

/// Noms trouvés pour chaque intervenant (indice = numéro de l'intervenant),
/// d'après les répliques où il se présente.
///
/// `given` : noms donnés par l'utilisateur ; ces intervenants gardent le
/// leur, et leurs noms ne sont proposés à personne d'autre. Un même nom
/// n'est donné qu'à un intervenant (celui qui se présente le plus
/// nettement), et un intervenant n'a qu'un nom.
pub fn guess_names(segments: &[Segment], given: &[String]) -> Vec<Option<NameGuess>> {
    let named = |i: usize| given.get(i).is_some_and(|n| !n.trim().is_empty());
    let taken: Vec<String> = given
        .iter()
        .filter(|n| !n.trim().is_empty())
        .map(|n| key(n))
        .collect();
    let mut candidates: Vec<Candidate> = Vec::new();
    for reply in turns(segments) {
        let Some(speaker) = reply.speaker else {
            continue;
        };
        if named(speaker) {
            continue;
        }
        for (name, strength, start, end) in introductions(&reply.text) {
            let key = key(&name);
            if key.is_empty() || taken.contains(&key) {
                continue;
            }
            match candidates
                .iter_mut()
                .find(|c| c.speaker == speaker && c.key == key)
            {
                Some(candidate) => {
                    candidate.score += strength as u32;
                    // Le nom le plus complet (« Marie Dupont » plutôt que
                    // « Marie »).
                    if name.chars().count() > candidate.guess.name.chars().count() {
                        candidate.guess = NameGuess {
                            name,
                            at_ms: reply.start_ms,
                            quote: quote(&reply.text, start, end),
                        };
                    }
                }
                None => candidates.push(Candidate {
                    speaker,
                    key,
                    guess: NameGuess {
                        name,
                        at_ms: reply.start_ms,
                        quote: quote(&reply.text, start, end),
                    },
                    score: strength as u32,
                    first_ms: reply.start_ms,
                }),
            }
        }
    }
    // Les présentations les plus nettes d'abord, puis les plus anciennes.
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.first_ms.cmp(&b.first_ms)));
    let mut guesses: Vec<Option<NameGuess>> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for candidate in candidates {
        let speaker = candidate.speaker;
        if guesses.get(speaker).is_some_and(Option::is_some) || used.contains(&candidate.key) {
            continue;
        }
        if guesses.len() <= speaker {
            guesses.resize(speaker + 1, None);
        }
        used.push(candidate.key);
        guesses[speaker] = Some(candidate.guess);
    }
    guesses
}

/// Même personne : même premier mot (hors civilité), sans casse ni accents.
pub fn same_person(a: &str, b: &str) -> bool {
    let (a, b) = (key(a), key(b));
    !a.is_empty() && a == b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::Track;

    /// Nom trouvé dans une phrase seule.
    fn name(text: &str) -> Option<String> {
        introductions(text)
            .into_iter()
            .max_by_key(|(_, strength, ..)| *strength)
            .map(|(name, ..)| name)
    }

    #[test]
    fn introductions_give_the_name() {
        for (text, expected) in [
            ("Bonjour à tous, je m'appelle Marie.", "Marie"),
            (
                "Je m’appelle Jean-Pierre Martin, de la compta.",
                "Jean-Pierre Martin",
            ),
            ("je m'appelle sophie et je gère les achats", "Sophie"),
            ("Alors moi, c'est Paul, je suis chef de projet.", "Paul"),
            ("Oui, Marie Dupont à l'appareil.", "Marie Dupont"),
            ("Allô ? Bonjour, c'est Karim.", "Karim"),
            ("Je suis Claire Lefèvre, responsable RH.", "Claire Lefèvre"),
            ("Je me présente : Thomas Bernard.", "Thomas Bernard"),
            ("Mon prénom c'est Léa.", "Léa"),
            ("Je suis Madame Dupont.", "Madame Dupont"),
            ("Madame Dupont à l'appareil.", "Madame Dupont"),
            ("Hi everyone, my name is John Smith.", "John Smith"),
            ("Hello, this is Sarah from marketing.", "Sarah"),
            ("I'm David, I lead the data team.", "David"),
        ] {
            assert_eq!(name(text).as_deref(), Some(expected), "{text}");
        }
    }

    #[test]
    fn ordinary_sentences_give_no_name() {
        for text in [
            "Je suis désolé, je suis en retard.",
            "Je suis d'accord avec Marie.",
            "Bonjour Paul, comment ça va ?",
            "Moi c'est pareil, je ne sais pas.",
            "Je suis RH depuis deux ans.",
            "Je suis. Paul va parler.",
            "Je m'appelle comment déjà ?",
            "C'est Paul qui s'en occupe.",
            "Oui, c'est Paul qui s'en occupe.",
            "Bonjour, c'est Marie qui va présenter.",
            "Moi c'est Excel que j'utilise.",
            "Je suis Madame.",
            "Je suis à Paris demain.",
            "I'm Sorry, I'm not sure.",
            "Je suis allé voir Marie hier.",
            "Bonjour à l'appareil",
        ] {
            assert_eq!(name(text), None, "{text}");
        }
    }

    #[test]
    fn quotes_show_the_sentence() {
        let text = "Bon, on commence. Bonjour, je m'appelle Marie et je pilote le projet depuis \
                    le mois dernier avec toute l'équipe. Ensuite…";
        let (_, _, start, end) = introductions(text).remove(0);
        assert_eq!(
            quote(text, start, end),
            "Bonjour, je m'appelle Marie et je pilote le projet depuis le mois…"
        );
    }

    fn line(speaker: usize, start_ms: u64, text: &str) -> Segment {
        Segment {
            track: Track::Eux,
            start_ms,
            end_ms: start_ms + 5_000,
            text: text.into(),
            copied: false,
            speaker: Some(speaker),
        }
    }

    #[test]
    fn each_speaker_gets_the_name_they_give() {
        let segments = vec![
            Segment {
                speaker: None,
                track: Track::Moi,
                ..line(0, 0, "Bonjour, je suis Florian, on fait un tour de table ?")
            },
            line(0, 20_000, "Oui, je m'appelle Marie, je suis au marketing."),
            line(1, 40_000, "Et moi c'est Paul."),
            line(2, 60_000, "Bonjour Paul, bonjour Marie."),
            line(
                0,
                80_000,
                "Je suis Marie Dupont pour ceux qui ne me connaissent pas.",
            ),
        ];
        let guesses = guess_names(&segments, &[]);
        let names: Vec<_> = guesses
            .iter()
            .map(|g| g.as_ref().map(|g| g.name.as_str()))
            .collect();
        assert_eq!(names, [Some("Marie Dupont"), Some("Paul")]);
        let marie = guesses[0].as_ref().unwrap();
        assert_eq!(marie.at_ms, 80_000);
        assert!(
            marie.quote.contains("Je suis Marie Dupont"),
            "{}",
            marie.quote
        );
    }

    #[test]
    fn a_name_goes_to_one_speaker_and_given_names_win() {
        let segments = vec![
            // Voix mal séparée : Marie se présente aussi depuis l'intervenant 2.
            line(2, 0, "Je suis Marie."),
            line(0, 10_000, "Bonjour, je m'appelle Marie."),
            line(1, 20_000, "Moi c'est Paul."),
            line(3, 30_000, "Je m'appelle Karim."),
        ];
        let guesses = guess_names(&segments, &[]);
        assert_eq!(guesses[0].as_ref().unwrap().name, "Marie");
        assert!(guesses[2].is_none(), "Marie déjà prise");
        // Noms donnés : l'intervenant 1 s'appelle Paul pour l'utilisateur,
        // l'intervenant 3 « Karim B. » ; personne d'autre ne reçoit Paul ou
        // Karim.
        let given = ["".into(), "Paul".into(), "".into(), "Karim B.".into()];
        let guesses = guess_names(&segments, &given);
        assert_eq!(guesses[0].as_ref().unwrap().name, "Marie");
        assert!(guesses.get(1).is_none_or(Option::is_none));
        assert!(guesses.get(3).is_none_or(Option::is_none));
    }

    #[test]
    fn names_compare_without_case_accents_or_title() {
        assert!(same_person("Hélène", "helene Martin"));
        assert!(same_person("Madame Dupont", "dupont"));
        assert!(!same_person("Marie", "Paul"));
        assert!(!same_person("", ""));
    }
}
