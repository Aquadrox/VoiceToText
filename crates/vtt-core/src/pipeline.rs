//! Orchestration d'une dictée : audio → texte brut → texte propre → insertion.
//!
//! Les appels sont bloquants : le pipeline tourne sur un thread de travail
//! dédié, jamais sur le thread du hook clavier ou de l'interface.

use std::time::Instant;

use crate::config::{Snippet, Style};
use crate::usage::Usage;
use crate::{audio, text};

/// Fréquence attendue par Whisper.
pub const SAMPLE_RATE: u32 = 16_000;

/// En dessous de cette durée, l'enregistrement est jeté sans appeler l'ASR.
const MIN_CLIP_MS: u64 = 300;

/// Niveau RMS sous lequel un enregistrement est considéré comme du silence (~ -46 dBFS).
const SILENCE_RMS: f32 = 0.005;

/// Audio mono 16 kHz, échantillons f32 dans [-1, 1].
#[derive(Debug, Clone, Default)]
pub struct AudioClip {
    pub samples: Vec<f32>,
}

impl AudioClip {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / SAMPLE_RATE as u64
    }
}

/// Fenêtre active au début de la dictée, là où le texte sera inséré.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// Nom de l'exécutable, ex. "slack.exe".
    pub app: Option<String>,
    /// Titre de la fenêtre (jamais journalisé : il peut être personnel).
    pub title: Option<String>,
}

/// Consoles où un texte collé contenant un retour à la ligne est exécuté.
const TERMINALS: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "openconsole.exe",
    "mintty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "putty.exe",
];

impl Target {
    pub fn is_terminal(&self) -> bool {
        self.app
            .as_deref()
            .is_some_and(|app| TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(app)))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transcription : {0}")]
    Asr(String),
    #[error("nettoyage : {0}")]
    PostProcess(String),
    #[error("insertion : {0}")]
    Sink(String),
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error>;

    /// Dictionnaire modifié dans l'interface : nouveaux mots à privilégier.
    fn set_vocabulary(&self, words: &[String]) {
        let _ = words;
    }

    /// Libère le modèle (mémoire graphique) ; `true` s'il était chargé. La
    /// transcription suivante le recharge.
    fn unload(&self) -> bool {
        false
    }

    /// Recharge le modèle s'il a été libéré (au début d'une dictée, pendant
    /// que l'utilisateur parle).
    fn preload(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// Texte nettoyé et, si le fournisseur l'indique, les jetons facturés.
#[derive(Debug, Clone, PartialEq)]
pub struct Cleaned {
    pub text: String,
    pub usage: Option<Usage>,
}

pub trait PostProcessor: Send + Sync {
    /// `style` : consignes propres à l'application de destination.
    fn process(&self, raw: &str, style: Option<&str>) -> Result<Cleaned, Error>;

    /// Command Mode : applique la consigne dictée au texte sélectionné, ou
    /// rédige le texte demandé s'il n'y a pas de sélection.
    fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
        let _ = (instruction, selection);
        Err(Error::PostProcess("Command Mode non pris en charge".into()))
    }
}

pub trait TextSink: Send + Sync {
    fn insert(&self, text: &str) -> Result<(), Error>;
}

/// Résultat d'une dictée, conservé pour l'historique.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Clip trop court, silence ou hallucination : rien n'a été inséré.
    Skipped,
    Inserted {
        raw: String,
        text: String,
        /// Le nettoyage a échoué et le texte brut a été inséré à la place.
        post_process_error: Option<String>,
        /// La dictée était le déclencheur d'un snippet : son texte a été
        /// inséré tel quel, sans passer par le LLM.
        snippet: bool,
        /// Jetons facturés par le LLM pour cette dictée.
        usage: Option<Usage>,
        /// Command Mode : `raw` est la consigne, `text` le résultat.
        command: bool,
    },
}

pub struct Pipeline {
    transcriber: Box<dyn Transcriber>,
    post_processor: Option<Box<dyn PostProcessor>>,
    sink: Box<dyn TextSink>,
    /// Déclencheur normalisé → texte à insérer.
    snippets: Vec<(String, String)>,
    styles: Vec<Style>,
}

impl Pipeline {
    pub fn new(
        transcriber: Box<dyn Transcriber>,
        post_processor: Option<Box<dyn PostProcessor>>,
        sink: Box<dyn TextSink>,
    ) -> Self {
        Self {
            transcriber,
            post_processor,
            sink,
            snippets: Vec::new(),
            styles: Vec::new(),
        }
    }

    pub fn with_styles(mut self, styles: &[Style]) -> Self {
        self.styles = styles.to_vec();
        self
    }

    /// Premier style correspondant à la fenêtre de destination, et son numéro.
    fn style_for(&self, target: &Target) -> Option<(usize, &Style)> {
        self.styles
            .iter()
            .enumerate()
            .find(|(_, s)| s.matches(target.app.as_deref(), target.title.as_deref()))
    }

    pub fn with_snippets(mut self, snippets: &[Snippet]) -> Self {
        self.set_snippets(snippets);
        self
    }

    /// Les déclencheurs vides (ou faits uniquement de ponctuation) sont ignorés.
    fn set_snippets(&mut self, snippets: &[Snippet]) {
        self.snippets = snippets
            .iter()
            .map(|s| (text::normalize_phrase(&s.trigger), s.text.clone()))
            .filter(|(trigger, _)| !trigger.is_empty())
            .collect();
    }

    /// Applique à chaud un dictionnaire, des snippets, des styles et un
    /// nettoyage (reconstruit avec le nouveau dictionnaire) modifiés dans
    /// l'interface, sans recharger le modèle.
    pub fn reconfigure(
        &mut self,
        dictionary: &[String],
        snippets: &[Snippet],
        styles: &[Style],
        post_processor: Option<Box<dyn PostProcessor>>,
    ) {
        self.transcriber.set_vocabulary(dictionary);
        self.post_processor = post_processor;
        self.set_snippets(snippets);
        self.styles = styles.to_vec();
    }

    /// Libère le modèle de transcription ; `true` s'il était chargé.
    pub fn unload_model(&self) -> bool {
        self.transcriber.unload()
    }

    /// Recharge le modèle de transcription s'il a été libéré.
    pub fn preload_model(&self) -> Result<(), Error> {
        self.transcriber.preload()
    }

    fn snippet_for(&self, raw: &str) -> Option<(usize, &str)> {
        let phrase = text::normalize_phrase(raw);
        self.snippets
            .iter()
            .position(|(trigger, _)| *trigger == phrase)
            .map(|i| (i, self.snippets[i].1.as_str()))
    }

    /// Transcription filtrée et durée de l'ASR ; `None` si le clip est trop
    /// court, silencieux ou ne contient qu'une hallucination.
    fn transcribe(&self, clip: &AudioClip) -> Result<Option<(String, u128)>, Error> {
        if clip.duration_ms() < MIN_CLIP_MS {
            log::info!("ignoré : enregistrement de {} ms", clip.duration_ms());
            return Ok(None);
        }
        if audio::is_silent(&clip.samples, SAMPLE_RATE, SILENCE_RMS) {
            log::info!("ignoré : silence ({} ms)", clip.duration_ms());
            return Ok(None);
        }

        let started = Instant::now();
        let transcribed = self.transcriber.transcribe(clip)?;
        let asr_ms = started.elapsed().as_millis();
        let raw = text::strip_hallucinations(&transcribed);
        if raw.is_empty() {
            log::info!(
                "ignoré : transcription vide après filtrage ({} caractères écartés, ASR {asr_ms} ms)",
                transcribed.chars().count()
            );
            return Ok(None);
        }
        Ok(Some((raw, asr_ms)))
    }

    pub fn run(&self, clip: &AudioClip, target: &Target) -> Result<Outcome, Error> {
        self.run_to(clip, target, self.sink.as_ref())
    }

    /// Comme [`Pipeline::run`], mais le texte va dans `sink` au lieu de
    /// l'insertion habituelle (ex. presse-papier seul pour « Réessayer »
    /// depuis le menu de l'icône).
    pub fn run_to(
        &self,
        clip: &AudioClip,
        target: &Target,
        sink: &dyn TextSink,
    ) -> Result<Outcome, Error> {
        let Some((raw, asr_ms)) = self.transcribe(clip)? else {
            return Ok(Outcome::Skipped);
        };

        // Snippet : la dictée entière est un déclencheur → texte fixe, sans LLM.
        // Le journal ne contient que son numéro (le texte peut être personnel).
        if let Some((index, snippet)) = self.snippet_for(&raw) {
            let started = Instant::now();
            sink.insert(snippet)?;
            log::info!(
                "dictée de {} ms : ASR {asr_ms} ms, snippet n°{}, insertion {} ms",
                clip.duration_ms(),
                index + 1,
                started.elapsed().as_millis()
            );
            return Ok(Outcome::Inserted {
                raw,
                text: snippet.to_owned(),
                post_process_error: None,
                snippet: true,
                usage: None,
                command: false,
            });
        }

        // Un échec du LLM ne doit jamais faire perdre la dictée.
        let style = self.style_for(target);
        if let Some((index, style)) = style {
            log::info!(
                "style n°{} ({})",
                index + 1,
                if style.cleanup {
                    "consignes"
                } else {
                    "texte brut"
                }
            );
        }
        let processor = match style {
            Some((_, style)) if !style.cleanup => None,
            _ => self.post_processor.as_ref(),
        };
        let instructions = style
            .map(|(_, s)| s.instructions.trim())
            .filter(|i| !i.is_empty());
        let started = Instant::now();
        let (text, post_process_error, usage) = match processor {
            None => (raw.clone(), None, None),
            Some(processor) => match processor.process(&raw, instructions) {
                Ok(cleaned) if !cleaned.text.trim().is_empty() => {
                    (cleaned.text.trim().to_owned(), None, cleaned.usage)
                }
                Ok(cleaned) => (raw.clone(), Some("réponse vide".to_owned()), cleaned.usage),
                Err(err) => (raw.clone(), Some(err.to_string()), None),
            },
        };

        let post_ms = started.elapsed().as_millis();

        let started = Instant::now();
        sink.insert(&text)?;
        let tokens = usage.map_or(String::new(), |u| {
            format!(" ({} + {} jetons)", u.input_tokens, u.output_tokens)
        });
        log::info!(
            "dictée de {} ms : ASR {asr_ms} ms, nettoyage {post_ms} ms{tokens}, insertion {} ms",
            clip.duration_ms(),
            started.elapsed().as_millis()
        );
        Ok(Outcome::Inserted {
            raw,
            text,
            post_process_error,
            snippet: false,
            usage,
            command: false,
        })
    }

    /// Command Mode : la dictée est une consigne appliquée à `selection`. En
    /// cas d'échec rien n'est inséré : la sélection reste intacte.
    ///
    /// Refusé dans un terminal : une réponse piégée par le texte sélectionné
    /// (injection de consignes) y serait exécutée dès le collage.
    pub fn run_command(
        &self,
        clip: &AudioClip,
        target: &Target,
        selection: Option<&str>,
    ) -> Result<Outcome, Error> {
        if target.is_terminal() {
            return Err(Error::Sink(
                "Command Mode désactivé dans les terminaux (sécurité)".into(),
            ));
        }
        let Some(processor) = &self.post_processor else {
            return Err(Error::PostProcess(
                "Command Mode : le nettoyage LLM est désactivé".into(),
            ));
        };
        let Some((instruction, asr_ms)) = self.transcribe(clip)? else {
            return Ok(Outcome::Skipped);
        };
        let started = Instant::now();
        let result = processor.command(&instruction, selection)?;
        let llm_ms = started.elapsed().as_millis();
        if result.text.trim().is_empty() {
            return Err(Error::PostProcess("Command Mode : réponse vide".into()));
        }
        let started = Instant::now();
        self.sink.insert(result.text.trim())?;
        log::info!(
            "commande de {} ms : ASR {asr_ms} ms, LLM {llm_ms} ms ({}), insertion {} ms",
            clip.duration_ms(),
            match selection {
                Some(s) => format!("sélection de {} caractères", s.chars().count()),
                None => "sans sélection".into(),
            },
            started.elapsed().as_millis()
        );
        Ok(Outcome::Inserted {
            raw: instruction,
            text: result.text.trim().to_owned(),
            post_process_error: None,
            snippet: false,
            usage: result.usage,
            command: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FixedAsr(&'static str);
    impl Transcriber for FixedAsr {
        fn transcribe(&self, _: &AudioClip) -> Result<String, Error> {
            Ok(self.0.to_owned())
        }
    }

    const USAGE: Usage = Usage {
        input_tokens: 120,
        cached_input_tokens: 100,
        output_tokens: 8,
    };

    struct Upper;
    impl PostProcessor for Upper {
        fn process(&self, raw: &str, style: Option<&str>) -> Result<Cleaned, Error> {
            let text = match style {
                Some(style) => format!("{} [{style}]", raw.to_uppercase()),
                None => raw.to_uppercase(),
            };
            Ok(Cleaned {
                text,
                usage: Some(USAGE),
            })
        }

        fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
            Ok(Cleaned {
                text: format!("{instruction} → {}", selection.unwrap_or("∅")),
                usage: Some(USAGE),
            })
        }
    }

    struct Failing;
    impl PostProcessor for Failing {
        fn process(&self, _: &str, _: Option<&str>) -> Result<Cleaned, Error> {
            Err(Error::PostProcess("délai dépassé".into()))
        }
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<String>>>);
    impl TextSink for Recorder {
        fn insert(&self, text: &str) -> Result<(), Error> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    fn no_target() -> Target {
        Target::default()
    }

    fn target(app: &str) -> Target {
        Target {
            app: Some(app.into()),
            title: Some("Fenêtre".into()),
        }
    }

    fn clip_ms(ms: u64) -> AudioClip {
        AudioClip {
            samples: vec![0.1; (ms * SAMPLE_RATE as u64 / 1000) as usize],
        }
    }

    fn pipeline(asr: &'static str, post: Option<Box<dyn PostProcessor>>) -> (Pipeline, Recorder) {
        let sink = Recorder::default();
        let p = Pipeline::new(Box::new(FixedAsr(asr)), post, Box::new(sink.clone()));
        (p, sink)
    }

    #[test]
    fn short_clip_is_skipped() {
        let (p, sink) = pipeline("bonjour", None);
        assert_eq!(
            p.run(&clip_ms(200), &no_target()).unwrap(),
            Outcome::Skipped
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn silent_clip_is_skipped() {
        let (p, sink) = pipeline("Sous-titres réalisés par la communauté d'Amara.org", None);
        let silent = AudioClip {
            samples: vec![0.0; SAMPLE_RATE as usize],
        };
        assert_eq!(
            p.run(&silent, &Target::default()).unwrap(),
            Outcome::Skipped
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn hallucination_only_is_skipped() {
        let (p, sink) = pipeline("Sous-titrage ST' 501", None);
        assert_eq!(
            p.run(&clip_ms(1000), &no_target()).unwrap(),
            Outcome::Skipped
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn blank_transcription_is_skipped() {
        let (p, sink) = pipeline("  \n", None);
        assert_eq!(
            p.run(&clip_ms(1000), &no_target()).unwrap(),
            Outcome::Skipped
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn post_processed_text_is_inserted() {
        let (p, sink) = pipeline(" bonjour ", Some(Box::new(Upper)));
        let outcome = p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(
            outcome,
            Outcome::Inserted {
                raw: "bonjour".into(),
                text: "BONJOUR".into(),
                post_process_error: None,
                snippet: false,
                usage: Some(USAGE),
                command: false,
            }
        );
        assert_eq!(*sink.0.lock().unwrap(), ["BONJOUR"]);
    }

    #[test]
    fn run_to_uses_given_sink() {
        let (p, sink) = pipeline("bonjour", Some(Box::new(Upper)));
        let other = Recorder::default();
        p.run_to(&clip_ms(1000), &no_target(), &other).unwrap();
        assert!(sink.0.lock().unwrap().is_empty());
        assert_eq!(*other.0.lock().unwrap(), ["BONJOUR"]);
    }

    #[test]
    fn post_process_failure_falls_back_to_raw() {
        let (p, sink) = pipeline("bonjour", Some(Box::new(Failing)));
        match p.run(&clip_ms(1000), &no_target()).unwrap() {
            Outcome::Inserted {
                text,
                post_process_error,
                ..
            } => {
                assert_eq!(text, "bonjour");
                assert!(post_process_error.unwrap().contains("délai dépassé"));
            }
            other => panic!("inattendu : {other:?}"),
        }
        assert_eq!(*sink.0.lock().unwrap(), ["bonjour"]);
    }

    fn snippets() -> Vec<Snippet> {
        vec![
            Snippet {
                trigger: "?!".into(),
                text: "jamais inséré".into(),
            },
            Snippet {
                trigger: "Ma signature".into(),
                text: "Florian\nVoiceToText".into(),
            },
        ]
    }

    #[test]
    fn snippet_trigger_inserts_text_without_llm() {
        let (p, sink) = pipeline("  ma signature. ", Some(Box::new(Failing)));
        let p = p.with_snippets(&snippets());
        assert_eq!(
            p.run(&clip_ms(1000), &no_target()).unwrap(),
            Outcome::Inserted {
                raw: "ma signature.".into(),
                text: "Florian\nVoiceToText".into(),
                post_process_error: None,
                snippet: true,
                usage: None,
                command: false,
            }
        );
        assert_eq!(*sink.0.lock().unwrap(), ["Florian\nVoiceToText"]);
    }

    #[test]
    fn trigger_inside_longer_dictation_is_not_a_snippet() {
        let (p, sink) = pipeline("voici ma signature", Some(Box::new(Upper)));
        let p = p.with_snippets(&snippets());
        assert!(matches!(
            p.run(&clip_ms(1000), &no_target()).unwrap(),
            Outcome::Inserted { snippet: false, .. }
        ));
        assert_eq!(*sink.0.lock().unwrap(), ["VOICI MA SIGNATURE"]);
    }

    #[test]
    fn punctuation_only_trigger_is_ignored() {
        let (p, _) = pipeline("bonjour", None);
        assert_eq!(p.with_snippets(&snippets()).snippets.len(), 1);
    }

    fn styles() -> Vec<Style> {
        vec![
            Style {
                apps: vec!["code.exe".into()],
                cleanup: false,
                ..Style::default()
            },
            Style {
                apps: vec!["slack.exe".into()],
                instructions: "décontracté".into(),
                ..Style::default()
            },
        ]
    }

    #[test]
    fn style_instructions_reach_post_processor() {
        let (p, sink) = pipeline("salut", Some(Box::new(Upper)));
        let p = p.with_styles(&styles());
        p.run(&clip_ms(1000), &target("Slack.exe")).unwrap();
        p.run(&clip_ms(1000), &target("notepad.exe")).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["SALUT [décontracté]", "SALUT"]);
    }

    #[test]
    fn style_without_cleanup_inserts_raw() {
        let (p, sink) = pipeline("let x = 1", Some(Box::new(Upper)));
        let p = p.with_styles(&styles());
        assert!(matches!(
            p.run(&clip_ms(1000), &target("code.exe")).unwrap(),
            Outcome::Inserted {
                usage: None,
                post_process_error: None,
                ..
            }
        ));
        assert_eq!(*sink.0.lock().unwrap(), ["let x = 1"]);
    }

    #[test]
    fn command_applies_instruction_to_selection() {
        let (p, sink) = pipeline(" traduis en anglais ", Some(Box::new(Upper)));
        match p
            .run_command(&clip_ms(1000), &no_target(), Some("bonjour"))
            .unwrap()
        {
            Outcome::Inserted {
                raw,
                text,
                command,
                usage,
                ..
            } => {
                assert_eq!(raw, "traduis en anglais");
                assert_eq!(text, "traduis en anglais → bonjour");
                assert!(command);
                assert_eq!(usage, Some(USAGE));
            }
            other => panic!("inattendu : {other:?}"),
        }
        p.run_command(&clip_ms(1000), &no_target(), None).unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            ["traduis en anglais → bonjour", "traduis en anglais → ∅"]
        );
    }

    #[test]
    fn command_failure_inserts_nothing() {
        let (p, sink) = pipeline("traduis", Some(Box::new(Failing)));
        assert!(
            p.run_command(&clip_ms(1000), &no_target(), Some("bonjour"))
                .is_err()
        );
        let (without_llm, _) = pipeline("traduis", None);
        assert!(
            without_llm
                .run_command(&clip_ms(1000), &no_target(), Some("x"))
                .is_err()
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn short_command_is_skipped() {
        let (p, sink) = pipeline("traduis", Some(Box::new(Upper)));
        assert_eq!(
            p.run_command(&clip_ms(200), &no_target(), Some("x"))
                .unwrap(),
            Outcome::Skipped
        );
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn command_is_refused_in_terminals() {
        let (p, sink) = pipeline("liste les fichiers", Some(Box::new(Upper)));
        let err = p
            .run_command(&clip_ms(1000), &target("WindowsTerminal.exe"), Some("x"))
            .unwrap_err();
        assert!(err.to_string().contains("terminaux"));
        assert!(sink.0.lock().unwrap().is_empty());
        assert!(!target("notepad.exe").is_terminal());
    }

    #[test]
    fn reconfigure_applies_new_snippets_styles_and_processor() {
        let (mut p, sink) = pipeline("ma signature", None);
        p.run(&clip_ms(1000), &target("slack.exe")).unwrap();
        p.reconfigure(
            &["Aquadrox".into()],
            &snippets(),
            &styles(),
            Some(Box::new(Upper)),
        );
        p.run(&clip_ms(1000), &target("slack.exe")).unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            ["ma signature", "Florian\nVoiceToText"]
        );
        let (mut p, sink) = pipeline("salut", None);
        p.reconfigure(&[], &[], &styles(), Some(Box::new(Upper)));
        p.run(&clip_ms(1000), &target("slack.exe")).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["SALUT [décontracté]"]);
    }
}
