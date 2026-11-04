//! Nettoyage déterministe du texte transcrit.

/// Retire les phrases que Whisper invente sur du bruit ou du silence.
///
/// Whisper a été entraîné sur des sous-titres : en français, il produit
/// typiquement des crédits de sous-titrage quand il n'entend rien de clair.
pub fn strip_hallucinations(text: &str) -> String {
    split_sentences(text)
        .into_iter()
        .filter(|sentence| !is_hallucination(sentence))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn is_hallucination(sentence: &str) -> bool {
    let s = sentence.trim().to_lowercase();
    s.contains("amara.org")
        || s.starts_with("sous-titr")
        || s.starts_with("merci d'avoir regardé")
        || s.starts_with("merci d’avoir regardé")
}

/// Forme comparable d'une phrase dictée : minuscules, sans accents ni
/// ponctuation, espaces simples. « Ma signature ! » → « ma signature ».
///
/// Sert à reconnaître un déclencheur de snippet quelle que soit la façon dont
/// Whisper l'a ponctué ou capitalisé.
pub fn normalize_phrase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'â' | 'ä' | 'á' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' | 'í' => out.push('i'),
            'ô' | 'ö' | 'ó' => out.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Phrases qui, dictées seules, annulent la dernière dictée (Ctrl+Z).
const UNDO_COMMANDS: &[&str] = &[
    "annule la derniere dictee",
    "annuler la derniere dictee",
    "efface la derniere dictee",
    "supprime la derniere dictee",
];

/// La dictée entière demande d'annuler la précédente.
pub fn is_undo_command(raw: &str) -> bool {
    UNDO_COMMANDS.contains(&normalize_phrase(raw).as_str())
}

/// Commandes de mise en page (forme normalisée) → texte inséré.
const LAYOUT_COMMANDS: &[(&str, &str)] = &[
    ("nouveau paragraphe", "\n\n"),
    ("nouvelle ligne", "\n"),
    ("retour a la ligne", "\n"),
    ("a la ligne", "\n"),
    ("point a la ligne", ".\n"),
];

/// Seule commande reconnue en fin de morceau (« Je pars demain point à la
/// ligne ») : les autres y seraient ambiguës (« une nouvelle ligne de bus »).
const SUFFIX_COMMAND: &str = "point a la ligne";

/// Remplace les commandes de mise en page dictées (« nouvelle ligne »,
/// « nouveau paragraphe », « point à la ligne »…) par des retours à la ligne.
///
/// Une commande n'est reconnue que si elle forme un morceau entier, délimité
/// par la ponctuation de Whisper ou le début / la fin de la dictée :
/// « Bonjour Paul, nouvelle ligne, je voulais… » oui, « une nouvelle ligne de
/// bus » non.
pub fn apply_layout_commands(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut capitalize = false;
    for (content, separator) in split_clauses(raw) {
        let phrase = normalize_phrase(content);
        let command = LAYOUT_COMMANDS
            .iter()
            .find(|(name, _)| *name == phrase)
            .map(|(_, text)| (content.len(), *text))
            .or_else(|| suffix_command(content, &phrase));
        let Some((command_len, text)) = command else {
            push_capitalized(&mut out, content, capitalize);
            out.push_str(separator);
            capitalize = false;
            continue;
        };
        // Texte dicté avant « point à la ligne » dans le même morceau.
        let before = content[..content.len() - command_len].trim_end();
        push_capitalized(&mut out, before, capitalize);
        while out.ends_with(' ') {
            out.pop();
        }
        // Pas de point en double si Whisper en a déjà mis un.
        let text = match text.strip_prefix('.') {
            Some(rest) if out.is_empty() || out.ends_with(['.', '!', '?', '…', '\n']) => rest,
            _ => text,
        };
        out.push_str(text);
        // La ponctuation de la commande elle-même est retirée.
        let trimmed = out.trim_end_matches(['\n', ' ']);
        capitalize = trimmed.is_empty() || trimmed.ends_with(['.', '!', '?', '…']);
    }
    out.trim_end_matches(' ').to_owned()
}

/// Commande en fin de morceau : longueur (en octets) de sa partie dictée.
fn suffix_command(content: &str, phrase: &str) -> Option<(usize, &'static str)> {
    let rest = phrase.strip_suffix(SUFFIX_COMMAND)?;
    if !rest.ends_with(' ') {
        return None;
    }
    // Retrouve où commence la commande dans le texte original : ses 4 derniers
    // mots (« point à la ligne »).
    let mut words = 0;
    let mut start = content.len();
    for (i, c) in content.trim_end().char_indices().rev() {
        if c.is_whitespace() {
            words += 1;
            if words == 4 {
                break;
            }
        }
        start = i;
    }
    let text = LAYOUT_COMMANDS
        .iter()
        .find(|(name, _)| *name == SUFFIX_COMMAND)
        .map(|(_, text)| *text)?;
    Some((content.len() - start, text))
}

fn push_capitalized(out: &mut String, text: &str, capitalize: bool) {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if capitalize => {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
        _ => out.push_str(text),
    }
}

/// Découpe en morceaux (texte, ponctuation et espaces qui suivent). Les
/// espaces avant la ponctuation (« demain ? ») vont avec elle.
fn split_clauses(text: &str) -> Vec<(&str, &str)> {
    let is_mark = |c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '…');
    let mut clauses = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let content_end = rest.find(is_mark).unwrap_or(rest.len());
        let separator_end = rest[content_end..]
            .find(|c: char| !is_mark(c) && !c.is_whitespace())
            .map_or(rest.len(), |n| content_end + n);
        let content = rest[..content_end].trim_end();
        clauses.push((content, &rest[content.len()..separator_end]));
        rest = &rest[separator_end..];
    }
    clauses
}

/// Découpe après `.`, `!`, `?` ou `…` (sauf si collé à la suite, ex. « Amara.org »),
/// en gardant ponctuation et espaces.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '…') {
            // Ponctuation répétée (« ?! », « ... ») rattachée à la même phrase.
            let mut end = i + c.len_utf8();
            while let Some(&(j, next)) = chars.peek() {
                if matches!(next, '.' | '!' | '?' | '…') {
                    end = j + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            // « Amara.org », « 3.5 » : pas une fin de phrase.
            if chars
                .peek()
                .is_some_and(|&(_, next)| next.is_alphanumeric())
            {
                continue;
            }
            sentences.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        sentences.push(&text[start..]);
    }
    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_phrase_ignores_case_accents_and_punctuation() {
        assert_eq!(normalize_phrase("  Ma Signature ! "), "ma signature");
        assert_eq!(
            normalize_phrase("Mon adresse e-mail."),
            "mon adresse e mail"
        );
        assert_eq!(
            normalize_phrase("Réponse « type », s'il te plaît…"),
            "reponse type s il te plait"
        );
        assert_eq!(normalize_phrase("Cœur ÇA"), "coeur ca");
        assert_eq!(normalize_phrase("?!"), "");
    }

    #[test]
    fn layout_commands_become_line_breaks() {
        assert_eq!(
            apply_layout_commands("Bonjour Paul, nouvelle ligne, je voulais te dire merci."),
            "Bonjour Paul,\nje voulais te dire merci."
        );
        assert_eq!(
            apply_layout_commands("Premier point. Nouveau paragraphe. deuxième point."),
            "Premier point.\n\nDeuxième point."
        );
        assert_eq!(
            apply_layout_commands("Je pars demain point à la ligne. merci"),
            "Je pars demain.\nMerci"
        );
        assert_eq!(
            apply_layout_commands("C'est noté. Point à la ligne. À plus."),
            "C'est noté.\nÀ plus."
        );
        assert_eq!(apply_layout_commands("À la ligne."), "\n");
    }

    #[test]
    fn layout_words_inside_a_sentence_are_kept() {
        for text in [
            "On ouvre une nouvelle ligne de bus.",
            "J'ai ajouté un nouveau paragraphe au contrat.",
            "Il reste à la ligne d'arrivée, demain ?",
            "Bonjour à tous. On se voit demain ? Oui !",
        ] {
            assert_eq!(apply_layout_commands(text), text);
        }
    }

    #[test]
    fn undo_command_is_the_whole_dictation() {
        assert!(is_undo_command("Annule la dernière dictée."));
        assert!(is_undo_command("efface la dernière dictée"));
        assert!(!is_undo_command("Il annule la dernière dictée de Paul."));
    }

    #[test]
    fn keeps_normal_text() {
        let text = "Bonjour à tous. On se voit demain ? Oui !";
        assert_eq!(strip_hallucinations(text), text);
    }

    #[test]
    fn removes_subtitle_credits() {
        assert_eq!(
            strip_hallucinations("Sous-titres réalisés par la communauté d'Amara.org"),
            ""
        );
        assert_eq!(strip_hallucinations("Sous-titrage ST' 501"), "");
        assert_eq!(
            strip_hallucinations("Il fait beau. Merci d'avoir regardé cette vidéo !"),
            "Il fait beau."
        );
    }

    #[test]
    fn keeps_merci_alone() {
        assert_eq!(strip_hallucinations("Merci."), "Merci.");
    }

    #[test]
    fn split_keeps_repeated_punctuation() {
        assert_eq!(split_sentences("Quoi ?! Bon..."), ["Quoi ?!", " Bon..."]);
        assert_eq!(
            split_sentences("Version 3.5 ok. Fin"),
            ["Version 3.5 ok.", " Fin"]
        );
    }
}
