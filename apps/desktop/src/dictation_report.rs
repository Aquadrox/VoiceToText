//! Résultat d'une dictée pour la zone de test des réglages : ce qui a été
//! entendu, ce qui a été inséré, et le temps passé à chaque étape.
//!
//! Envoyé seulement à la fenêtre principale (événement `dictation`), jamais
//! écrit sur le disque : l'historique reste le seul endroit où le texte est
//! conservé, et il peut être désactivé.

use serde::Serialize;
use vtt_core::config::Pricing;
use vtt_core::{Outcome, Target};

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Texte inséré (ou copié pour « Réessayer » depuis le menu).
    Inserted,
    /// Trop court, silence ou hallucination : rien n'a été inséré.
    Skipped,
    /// « Annule la dernière dictée ».
    Undone,
    /// Erreur (transcription, insertion, Command Mode…).
    Error,
}

#[derive(Debug, Serialize)]
pub struct DictationReport {
    pub kind: Kind,
    /// Transcription du moteur local (consigne en Command Mode).
    pub raw: String,
    /// Texte inséré.
    pub text: String,
    pub command: bool,
    pub snippet: bool,
    pub retry: bool,
    /// Langue d'arrivée en mode traduction.
    pub translated: Option<String>,
    /// Profil de secours qui a répondu à la place du principal.
    pub fallback: Option<String>,
    /// Le nettoyage a échoué : texte brut inséré.
    pub cleanup_error: Option<String>,
    /// Erreur de la dictée (rien d'inséré).
    pub error: Option<String>,
    /// Durée de l'audio.
    pub audio_ms: u64,
    /// Transcription par le moteur local (0 : non appelée).
    pub asr_ms: u64,
    /// Traitement complet : transcription, nettoyage et insertion.
    pub total_ms: u64,
    /// Fichier du modèle de transcription utilisé.
    pub model: String,
    /// Exécutable de la fenêtre cible.
    pub app: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
}

/// Mesures prises autour du traitement d'une dictée.
pub struct Measures<'a> {
    pub audio_ms: u64,
    pub asr_ms: u64,
    pub total_ms: u64,
    pub model: &'a str,
    pub command: bool,
    pub retry: bool,
}

pub fn build(
    result: &Result<Outcome, vtt_core::Error>,
    target: &Target,
    pricing: &Pricing,
    m: &Measures<'_>,
) -> DictationReport {
    let mut report = DictationReport {
        kind: Kind::Skipped,
        raw: String::new(),
        text: String::new(),
        command: m.command,
        snippet: false,
        retry: m.retry,
        translated: None,
        fallback: None,
        cleanup_error: None,
        error: None,
        audio_ms: m.audio_ms,
        asr_ms: m.asr_ms,
        total_ms: m.total_ms,
        model: m.model.to_owned(),
        app: target.app.clone(),
        input_tokens: None,
        output_tokens: None,
        cost: None,
        currency: None,
    };
    match result {
        Ok(Outcome::Skipped) => {}
        Ok(Outcome::Undone) => report.kind = Kind::Undone,
        Err(e) => {
            report.kind = Kind::Error;
            report.error = Some(e.to_string());
        }
        Ok(Outcome::Inserted {
            raw,
            text,
            post_process_error,
            snippet,
            usage,
            command,
            fallback,
            translated,
        }) => {
            report.kind = Kind::Inserted;
            report.raw = raw.clone();
            report.text = text.clone();
            report.command = *command;
            report.snippet = *snippet;
            report.translated = translated.clone();
            report.fallback = fallback.clone();
            report.cleanup_error = post_process_error.clone();
            if let Some(usage) = usage {
                report.input_tokens = Some(usage.input_tokens);
                report.output_tokens = Some(usage.output_tokens);
                report.cost = Some(pricing.cost(usage));
                report.currency = Some(pricing.currency.clone());
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use vtt_core::usage::Usage;

    fn measures() -> Measures<'static> {
        Measures {
            audio_ms: 2400,
            asr_ms: 400,
            total_ms: 1200,
            model: "ggml-small.bin",
            command: false,
            retry: false,
        }
    }

    fn target() -> Target {
        Target {
            app: Some("notepad.exe".into()),
            title: Some("Titre personnel".into()),
        }
    }

    #[test]
    fn inserted_dictation_carries_texts_tokens_and_cost() {
        let usage = Usage {
            input_tokens: 1_000_000,
            cached_input_tokens: 0,
            output_tokens: 0,
        };
        let outcome = Outcome::Inserted {
            raw: "euh bonjour".into(),
            text: "Bonjour.".into(),
            post_process_error: None,
            snippet: false,
            usage: Some(usage),
            command: false,
            fallback: Some("Ollama".into()),
            translated: None,
        };
        let pricing = Pricing::default();
        let report = build(&Ok(outcome), &target(), &pricing, &measures());
        assert_eq!(report.kind, Kind::Inserted);
        assert_eq!(report.raw, "euh bonjour");
        assert_eq!(report.text, "Bonjour.");
        assert_eq!(report.input_tokens, Some(1_000_000));
        assert_eq!(report.cost, Some(pricing.input_per_million));
        assert_eq!(report.fallback.as_deref(), Some("Ollama"));
        assert_eq!(report.model, "ggml-small.bin");
        assert_eq!((report.asr_ms, report.total_ms), (400, 1200));
    }

    #[test]
    fn window_title_is_never_sent() {
        let report = build(
            &Ok(Outcome::Skipped),
            &target(),
            &Pricing::default(),
            &measures(),
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("Titre personnel"), "{json}");
        assert!(json.contains("notepad.exe"));
        assert_eq!(report.kind, Kind::Skipped);
    }

    #[test]
    fn errors_and_undo_are_reported() {
        let error = Err(vtt_core::Error::Sink("collage refusé".into()));
        let report = build(&error, &target(), &Pricing::default(), &measures());
        assert_eq!(report.kind, Kind::Error);
        assert!(report.error.unwrap().contains("collage refusé"));
        let undone = build(
            &Ok(Outcome::Undone),
            &target(),
            &Pricing::default(),
            &measures(),
        );
        assert_eq!(undone.kind, Kind::Undone);
        assert!(undone.text.is_empty());
    }
}
