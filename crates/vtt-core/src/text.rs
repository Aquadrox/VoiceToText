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
