//! Orchestration d'une dictée : audio → texte brut → texte propre → insertion.
//!
//! Les appels sont bloquants : le pipeline tourne sur un thread de travail
//! dédié, jamais sur le thread du hook clavier ou de l'interface.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

use crate::config::{Snippet, Style};
use crate::usage::Usage;
use crate::{audio, text};

/// Fréquence attendue par Whisper.
pub const SAMPLE_RATE: u32 = 16_000;

/// En dessous de cette durée, l'enregistrement est jeté sans appeler l'ASR.
const MIN_CLIP_MS: u64 = 300;

/// Niveau RMS sous lequel un enregistrement est considéré comme du silence (~ -46 dBFS).
pub(crate) const SILENCE_RMS: f32 = 0.005;

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

/// Texte brut issu d'une ou plusieurs tranches ASR, prêt pour la finalisation
/// unique (commandes vocales, snippets, style, nettoyage LLM, insertion).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawDictation {
    pub raw: String,
    pub duration_ms: u64,
    pub asr_ms: u64,
}

/// Fenêtre active au début de la dictée, là où le texte sera inséré.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// Nom de l'exécutable, ex. "slack.exe".
    pub app: Option<String>,
    /// Titre de la fenêtre (jamais journalisé : il peut être personnel).
    pub title: Option<String>,
    /// Termes extraits localement autour du curseur (opt-in, jamais journalisés
    /// ni enregistrés dans l'historique).
    pub context_terms: Vec<String>,
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

/// Mot transcrit et son moment dans le clip.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Mots de `text` répartis régulièrement entre `start_ms` et `end_ms` (moteur
/// sans horodatage par mot).
pub fn spread_words(text: &str, start_ms: u64, end_ms: u64) -> Vec<Word> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let span = end_ms.saturating_sub(start_ms) as f64 / words.len() as f64;
    words
        .iter()
        .enumerate()
        .map(|(i, word)| Word {
            text: (*word).to_owned(),
            start_ms: start_ms + (span * i as f64) as u64,
            end_ms: start_ms + (span * (i + 1) as f64) as u64,
        })
        .collect()
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, clip: &AudioClip) -> Result<String, Error>;

    /// Transcription avec termes d'appoint propres à cette dictée (extraits
    /// autour du curseur, opt-in) : n'altère pas le dictionnaire permanent.
    fn transcribe_with_hints(&self, clip: &AudioClip, hints: &[String]) -> Result<String, Error> {
        let _ = hints;
        self.transcribe(clip)
    }

    /// Mots horodatés (capture commentée : savoir quand « ici » a été dit).
    /// Par défaut : le texte réparti sur la durée du clip.
    fn transcribe_words(&self, clip: &AudioClip) -> Result<Vec<Word>, Error> {
        let text = self.transcribe(clip)?;
        Ok(spread_words(&text, 0, clip.duration_ms()))
    }

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
    /// Nom du profil de secours qui a répondu (`None` : le principal).
    pub fallback: Option<String>,
}

/// Consignes ajoutées au nettoyage d'une dictée.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Directives<'a> {
    /// Consignes propres à l'application de destination (style).
    pub style: Option<&'a str>,
    /// Mode traduction : langue dans laquelle traduire le texte final.
    pub translate_to: Option<&'a str>,
    /// Termes extraits localement autour du curseur (orthographe de référence
    /// uniquement, jamais une consigne).
    pub context_terms: &'a [String],
    /// Insertion au fil de la dictée : fin du texte déjà inséré juste avant
    /// cette tranche (continuité des majuscules et de la ponctuation ; ce
    /// texte n'est ni répété ni modifié).
    pub preceding: Option<&'a str>,
}

pub trait PostProcessor: Send + Sync {
    fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error>;

    /// Command Mode : applique la consigne dictée au texte sélectionné, ou
    /// rédige le texte demandé s'il n'y a pas de sélection.
    fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
        let _ = (instruction, selection);
        Err(Error::PostProcess("Command Mode non pris en charge".into()))
    }
}

/// Nettoyage principal doublé d'un profil de secours, essayé quand le
/// principal échoue ou répond vide, avant le repli sur le texte brut.
pub struct WithFallback {
    primary: Box<dyn PostProcessor>,
    fallback: Box<dyn PostProcessor>,
    name: String,
}

impl WithFallback {
    pub fn new(
        primary: Box<dyn PostProcessor>,
        fallback: Box<dyn PostProcessor>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            primary,
            fallback,
            name: name.into(),
        }
    }

    fn attempt(
        &self,
        call: impl Fn(&dyn PostProcessor) -> Result<Cleaned, Error>,
    ) -> Result<Cleaned, Error> {
        let primary_error = match call(self.primary.as_ref()) {
            Ok(cleaned) if !cleaned.text.trim().is_empty() => return Ok(cleaned),
            Ok(_) => "réponse vide".to_owned(),
            Err(e) => e.to_string(),
        };
        log::warn!(
            "LLM principal en échec ({primary_error}), essai du profil de secours « {} »",
            self.name
        );
        match call(self.fallback.as_ref()) {
            Ok(mut cleaned) => {
                cleaned.fallback = Some(self.name.clone());
                Ok(cleaned)
            }
            Err(e) => Err(Error::PostProcess(format!(
                "{primary_error} ; secours « {} » : {e}",
                self.name
            ))),
        }
    }
}

impl PostProcessor for WithFallback {
    fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
        self.attempt(|p| p.process(raw, directives))
    }

    fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
        self.attempt(|p| p.command(instruction, selection))
    }
}

pub trait TextSink: Send + Sync {
    fn insert(&self, text: &str) -> Result<(), Error>;

    /// Annule la dernière insertion (« annule la dernière dictée »).
    fn undo(&self) -> Result<(), Error> {
        Err(Error::Sink("annulation non prise en charge".into()))
    }
}

/// Résultat d'une dictée, conservé pour l'historique.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Clip trop court, silence ou hallucination : rien n'a été inséré.
    Skipped,
    /// Commande vocale « annule la dernière dictée » : Ctrl+Z envoyé.
    Undone,
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
        /// Profil de secours qui a répondu à la place du LLM principal.
        fallback: Option<String>,
        /// Mode traduction : langue dans laquelle `text` a été traduit.
        translated: Option<String>,
    },
}

/// Longueur (en caractères) de la fin du texte déjà inséré donnée au LLM
/// comme contexte de la tranche suivante.
const LIVE_CONTEXT_CHARS: usize = 300;

/// Une tranche insérée au fil de la dictée.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LivePiece {
    /// Texte brut de la tranche (après commandes de mise en page).
    raw: String,
    /// Texte réellement inséré, séparateur compris.
    text: String,
    snippet: bool,
}

/// Insertion au fil de la dictée : ce qui a déjà été inséré tranche par
/// tranche (voir [`Pipeline::insert_live_chunk`]).
#[derive(Debug, Clone, Default)]
pub struct LiveInsertion {
    pieces: Vec<LivePiece>,
    inserted: String,
    usage: Option<Usage>,
    error: Option<String>,
    fallback: Option<String>,
    undone: bool,
}

impl LiveInsertion {
    pub fn new() -> Self {
        Self::default()
    }

    /// Texte inséré depuis le début de la dictée.
    pub fn inserted(&self) -> &str {
        &self.inserted
    }

    /// Nombre de tranches insérées.
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    fn push(&mut self, piece: LivePiece) {
        self.inserted.push_str(&piece.text);
        self.pieces.push(piece);
    }

    /// « annule la dernière dictée » au milieu d'une dictée : la dernière
    /// tranche est retirée du bilan (l'application l'a annulée).
    fn undo_last(&mut self) {
        self.undone = true;
        if self.pieces.pop().is_some() {
            self.inserted = self.pieces.iter().map(|p| p.text.as_str()).collect();
        }
    }

    /// Fin du texte inséré, coupée sur une frontière de caractère.
    fn context_tail(&self) -> &str {
        let count = self.inserted.chars().count();
        if count <= LIVE_CONTEXT_CHARS {
            return &self.inserted;
        }
        let start = self
            .inserted
            .char_indices()
            .nth(count - LIVE_CONTEXT_CHARS)
            .map_or(0, |(i, _)| i);
        &self.inserted[start..]
    }
}

/// Espace à placer entre le texte déjà inséré et la tranche suivante : aucune
/// au début, après un blanc ou un retour à la ligne, après une apostrophe ou
/// une parenthèse ouvrante, ni devant une ponctuation qui s'attache au mot
/// précédent.
pub fn live_separator(previous: &str, next: &str) -> &'static str {
    if previous.is_empty()
        || previous.ends_with(char::is_whitespace)
        || next.starts_with(char::is_whitespace)
        || previous.ends_with(['\'', '’', '(', '['])
        || next.starts_with([',', '.', '…', ')', ']'])
    {
        ""
    } else {
        " "
    }
}

fn add_usage(total: Option<Usage>, more: Option<Usage>) -> Option<Usage> {
    match (total, more) {
        (None, more) => more,
        (total, None) => total,
        (Some(a), Some(b)) => Some(Usage {
            input_tokens: a.input_tokens + b.input_tokens,
            cached_input_tokens: a.cached_input_tokens + b.cached_input_tokens,
            output_tokens: a.output_tokens + b.output_tokens,
        }),
    }
}

/// Résultat du nettoyage d'un texte brut (avec repli sur le brut).
struct Cleanup {
    text: String,
    error: Option<String>,
    usage: Option<Usage>,
    fallback: Option<String>,
}

pub struct Pipeline {
    transcriber: Box<dyn Transcriber>,
    post_processor: Option<Box<dyn PostProcessor>>,
    sink: Box<dyn TextSink>,
    /// Déclencheur normalisé → texte à insérer.
    snippets: Vec<(String, String)>,
    styles: Vec<Style>,
    /// Commandes vocales (mise en page, annulation) reconnues.
    voice_commands: bool,
    /// Prétraitement audio avant la détection de silence et l'ASR
    /// (passe-haut 80 Hz + normalisation douce du niveau de parole).
    audio_enhance: bool,
    /// Durée de la dernière transcription (zone de test des réglages) ; 0 si
    /// le clip a été écarté avant l'ASR.
    last_asr_ms: AtomicU64,
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
            voice_commands: false,
            audio_enhance: false,
            last_asr_ms: AtomicU64::new(0),
        }
    }

    /// Durée de la transcription de la dernière dictée, en millisecondes
    /// (0 : clip trop court ou silencieux, ASR non appelé).
    pub fn last_asr_ms(&self) -> u64 {
        self.last_asr_ms.load(Ordering::Relaxed)
    }

    pub fn with_voice_commands(mut self, enabled: bool) -> Self {
        self.voice_commands = enabled;
        self
    }

    pub fn set_voice_commands(&mut self, enabled: bool) {
        self.voice_commands = enabled;
    }

    pub fn with_audio_enhance(mut self, enabled: bool) -> Self {
        self.audio_enhance = enabled;
        self
    }

    pub fn set_audio_enhance(&mut self, enabled: bool) {
        self.audio_enhance = enabled;
    }

    /// Remplace le modèle de transcription (changement de modèle dans les
    /// réglages).
    pub fn set_transcriber(&mut self, transcriber: Box<dyn Transcriber>) {
        self.transcriber = transcriber;
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
        self.transcribe_with_hints(clip, &[])
    }

    fn prepare_clip<'a>(&self, clip: &'a AudioClip) -> std::borrow::Cow<'a, AudioClip> {
        if self.audio_enhance {
            std::borrow::Cow::Owned(AudioClip {
                samples: audio::preprocess_for_asr(&clip.samples, SAMPLE_RATE),
            })
        } else {
            std::borrow::Cow::Borrowed(clip)
        }
    }

    fn transcribe_with_hints(
        &self,
        clip: &AudioClip,
        hints: &[String],
    ) -> Result<Option<(String, u128)>, Error> {
        self.last_asr_ms.store(0, Ordering::Relaxed);
        if clip.duration_ms() < MIN_CLIP_MS {
            log::info!("ignoré : enregistrement de {} ms", clip.duration_ms());
            return Ok(None);
        }
        let prepared = self.prepare_clip(clip);
        if audio::is_silent(&prepared.samples, SAMPLE_RATE, SILENCE_RMS) {
            log::info!("ignoré : silence ({} ms)", prepared.duration_ms());
            return Ok(None);
        }

        let started = Instant::now();
        let transcribed = if hints.is_empty() {
            self.transcriber.transcribe(&prepared)?
        } else {
            self.transcriber.transcribe_with_hints(&prepared, hints)?
        };
        let asr_ms = started.elapsed().as_millis();
        self.last_asr_ms
            .store(u64::try_from(asr_ms).unwrap_or(u64::MAX), Ordering::Relaxed);
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

    /// Mots horodatés de tout le clip (capture commentée) ; vide pour un
    /// silence.
    pub fn transcribe_words(&self, clip: &AudioClip) -> Result<Vec<Word>, Error> {
        let prepared = self.prepare_clip(clip);
        if audio::is_silent(&prepared.samples, SAMPLE_RATE, SILENCE_RMS) {
            return Ok(Vec::new());
        }
        let words = self.transcriber.transcribe_words(&prepared)?;
        let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        if text::strip_hallucinations(&text.join(" ")).is_empty() {
            return Ok(Vec::new());
        }
        Ok(words)
    }

    /// Transcription seule, filtrée comme une dictée (réunions) ; `None` :
    /// silence ou hallucination.
    pub fn transcribe_only(&self, clip: &AudioClip) -> Result<Option<String>, Error> {
        Ok(self.transcribe(clip)?.map(|(text, _)| text))
    }

    /// Transcription d'une tranche de dictée longue, filtrée comme une dictée
    /// (durée minimale, silence, hallucinations) et avec les termes d'appoint
    /// de la cible ; renvoie `(texte_brut, durée_asr_ms)`.
    pub fn transcribe_chunk(
        &self,
        clip: &AudioClip,
        hints: &[String],
    ) -> Result<Option<(String, u64)>, Error> {
        Ok(self
            .transcribe_with_hints(clip, hints)?
            .map(|(text, asr_ms)| (text, u64::try_from(asr_ms).unwrap_or(u64::MAX))))
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
        self.dictate(clip, target, Some(sink), None)
    }

    /// Dictée complète. `sink` : `None` pour l'insertion habituelle.
    /// `translate_to` (mode traduction) : le texte nettoyé est traduit dans
    /// cette langue par le LLM, même quand le style de l'application demande
    /// le texte brut.
    pub fn dictate(
        &self,
        clip: &AudioClip,
        target: &Target,
        sink: Option<&dyn TextSink>,
        translate_to: Option<&str>,
    ) -> Result<Outcome, Error> {
        let Some((raw, asr_ms)) = self.transcribe_with_hints(clip, &target.context_terms)? else {
            return Ok(Outcome::Skipped);
        };
        let dictation = RawDictation {
            raw,
            duration_ms: clip.duration_ms(),
            asr_ms: u64::try_from(asr_ms).unwrap_or(u64::MAX),
        };
        self.finalize_raw(&dictation, target, sink, translate_to)
    }

    /// Finalisation unique d'une dictée déjà transcrite (une tranche unique ou
    /// plusieurs tranches assemblées par [`crate::live::LiveSession`]) :
    /// commandes vocales, snippets, style, un seul passage LLM cohérent (avec
    /// secours et traduction éventuelle) et une seule insertion dans `sink`.
    pub fn finalize_raw(
        &self,
        dictation: &RawDictation,
        target: &Target,
        sink: Option<&dyn TextSink>,
        translate_to: Option<&str>,
    ) -> Result<Outcome, Error> {
        let sink = sink.unwrap_or(self.sink.as_ref());
        let raw = text::strip_hallucinations(&dictation.raw);
        if raw.is_empty() {
            self.last_asr_ms.store(0, Ordering::Relaxed);
            return Ok(Outcome::Skipped);
        }
        let asr_ms = dictation.asr_ms;
        self.last_asr_ms.store(asr_ms, Ordering::Relaxed);

        if self.voice_commands && text::is_undo_command(&raw) {
            sink.undo()?;
            log::info!("commande vocale : annulation de la dernière dictée");
            return Ok(Outcome::Undone);
        }

        // Snippet : la dictée entière est un déclencheur → texte fixe, sans LLM.
        // Le journal ne contient que son numéro (le texte peut être personnel).
        if let Some((index, snippet)) = self.snippet_for(&raw) {
            let started = Instant::now();
            sink.insert(snippet)?;
            log::info!(
                "dictée de {} ms : ASR {asr_ms} ms, snippet n°{}, insertion {} ms",
                dictation.duration_ms,
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
                fallback: None,
                translated: None,
            });
        }

        // Retours à la ligne dictés et mots épelés, avant le LLM (il les
        // conserve) ; la version brute les contient aussi.
        let raw = if self.voice_commands {
            text::apply_layout_commands(&text::join_spelled_letters(&raw))
        } else {
            raw
        };

        let started = Instant::now();
        let Cleanup {
            text,
            error: post_process_error,
            usage,
            fallback,
        } = self.clean(&raw, target, translate_to, None);
        let post_ms = started.elapsed().as_millis();

        let started = Instant::now();
        sink.insert(&text)?;
        let tokens = usage.map_or(String::new(), |u| {
            format!(" ({} + {} jetons)", u.input_tokens, u.output_tokens)
        });
        let tokens = match &fallback {
            Some(name) => format!("{tokens} via le secours « {name} »"),
            None => tokens,
        };
        log::info!(
            "dictée de {} ms : ASR {asr_ms} ms, nettoyage {post_ms} ms{tokens}, insertion {} ms",
            dictation.duration_ms,
            started.elapsed().as_millis()
        );
        Ok(Outcome::Inserted {
            raw,
            text,
            translated: translate_to
                .filter(|_| post_process_error.is_none())
                .map(str::to_owned),
            post_process_error,
            snippet: false,
            usage,
            command: false,
            fallback,
        })
    }

    /// Insertion au fil de la dictée : une tranche transcrite (phrase coupée
    /// à une pause) est traitée comme une petite dictée qui continue le texte
    /// déjà inséré (commandes vocales, snippet, style, nettoyage LLM avec la
    /// fin du texte précédent comme contexte, traduction) puis insérée
    /// aussitôt dans `sink`, précédée d'une espace si besoin. Rien n'est
    /// jamais effacé ni réécrit. Renvoie le texte inséré (séparateur compris),
    /// `None` si la tranche était vide (silence, hallucination).
    pub fn insert_live_chunk(
        &self,
        state: &mut LiveInsertion,
        chunk_raw: &str,
        target: &Target,
        sink: Option<&dyn TextSink>,
        translate_to: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let sink = sink.unwrap_or(self.sink.as_ref());
        let raw = text::strip_hallucinations(chunk_raw);
        if raw.is_empty() {
            return Ok(None);
        }

        if self.voice_commands && text::is_undo_command(&raw) {
            sink.undo()?;
            state.undo_last();
            log::info!("commande vocale : annulation de la dernière insertion");
            return Ok(None);
        }

        if let Some((index, snippet)) = self.snippet_for(&raw) {
            let piece = format!("{}{snippet}", live_separator(state.inserted(), snippet));
            sink.insert(&piece)?;
            log::info!(
                "insertion au fil de la dictée : tranche {}, snippet n°{}",
                state.pieces.len() + 1,
                index + 1
            );
            state.push(LivePiece {
                raw,
                text: piece.clone(),
                snippet: true,
            });
            return Ok(Some(piece));
        }

        let raw = if self.voice_commands {
            text::apply_layout_commands(&text::join_spelled_letters(&raw))
        } else {
            raw
        };
        if raw.trim().is_empty() && !raw.contains('\n') {
            return Ok(None);
        }

        let started = Instant::now();
        let preceding = state.context_tail();
        let preceding = (!preceding.trim().is_empty()).then_some(preceding);
        let cleanup = self.clean(&raw, target, translate_to, preceding);
        let post_ms = started.elapsed().as_millis();
        state.usage = add_usage(state.usage, cleanup.usage);
        if cleanup.error.is_some() && state.error.is_none() {
            state.error = cleanup.error;
        }
        if cleanup.fallback.is_some() {
            state.fallback = cleanup.fallback;
        }

        let piece = format!(
            "{}{}",
            live_separator(state.inserted(), &cleanup.text),
            cleanup.text
        );
        sink.insert(&piece)?;
        log::info!(
            "insertion au fil de la dictée : tranche {}, nettoyage {post_ms} ms",
            state.pieces.len() + 1
        );
        state.push(LivePiece {
            raw,
            text: piece.clone(),
            snippet: false,
        });
        Ok(Some(piece))
    }

    /// Bilan d'une dictée insérée au fil de l'eau, pour l'historique et les
    /// statistiques : brut et texte assemblés, jetons cumulés, première
    /// erreur de nettoyage. `Skipped` si rien n'a été inséré.
    pub fn finish_live_insertion(
        &self,
        state: LiveInsertion,
        duration_ms: u64,
        asr_ms: u64,
        translate_to: Option<&str>,
    ) -> Outcome {
        self.last_asr_ms.store(asr_ms, Ordering::Relaxed);
        if state.pieces.is_empty() {
            return if state.undone {
                Outcome::Undone
            } else {
                Outcome::Skipped
            };
        }
        log::info!(
            "dictée de {duration_ms} ms insérée au fil de l'eau : {} tranche(s), ASR {asr_ms} ms",
            state.pieces.len()
        );
        let raw = crate::live::join_live_segments(
            &state
                .pieces
                .iter()
                .map(|p| p.raw.clone())
                .collect::<Vec<_>>(),
        );
        let snippet = state.pieces.iter().all(|p| p.snippet);
        Outcome::Inserted {
            raw,
            text: state.inserted().trim().to_owned(),
            translated: translate_to
                .filter(|_| state.error.is_none())
                .map(str::to_owned),
            post_process_error: state.error,
            snippet,
            usage: state.usage,
            command: false,
            fallback: state.fallback,
        }
    }

    /// Nettoyage d'un texte brut (déjà passé par les commandes de mise en
    /// page) selon le style de la cible, avec repli sur le brut : un échec du
    /// LLM ne doit jamais faire perdre la dictée. `preceding` : fin du texte
    /// déjà inséré (insertion au fil de la dictée).
    fn clean(
        &self,
        raw: &str,
        target: &Target,
        translate_to: Option<&str>,
        preceding: Option<&str>,
    ) -> Cleanup {
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
            Some((_, style)) if !style.cleanup && translate_to.is_none() => None,
            _ => self.post_processor.as_ref(),
        };
        let directives = Directives {
            style: style
                .map(|(_, s)| s.instructions.trim())
                .filter(|i| !i.is_empty()),
            translate_to,
            context_terms: &target.context_terms,
            preceding,
        };
        let raw_cleanup = |error: Option<String>, usage, fallback| Cleanup {
            text: raw.to_owned(),
            error,
            usage,
            fallback,
        };
        let mut cleanup = match processor {
            None if translate_to.is_some() => raw_cleanup(
                Some("mode traduction : le nettoyage LLM est désactivé".to_owned()),
                None,
                None,
            ),
            None => raw_cleanup(None, None, None),
            Some(processor) => match processor.process(raw, directives) {
                Ok(cleaned) if !cleaned.text.trim().is_empty() => Cleanup {
                    text: cleaned.text.trim().to_owned(),
                    error: None,
                    usage: cleaned.usage,
                    fallback: cleaned.fallback,
                },
                Ok(cleaned) => raw_cleanup(
                    Some("réponse vide".to_owned()),
                    cleaned.usage,
                    cleaned.fallback,
                ),
                Err(err) => raw_cleanup(Some(err.to_string()), None, None),
            },
        };
        // « nouvelle ligne » en début ou en fin de dictée : le LLM rogne les
        // retours extrêmes.
        let leading = raw.len() - raw.trim_start_matches('\n').len();
        if leading > 0 && !cleanup.text.starts_with('\n') {
            cleanup.text.insert_str(0, &raw[..leading]);
        }
        if raw.ends_with('\n') && !cleanup.text.ends_with('\n') {
            cleanup.text.push('\n');
        }
        cleanup
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
            fallback: result.fallback,
            translated: None,
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
        fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
            let mut text = match directives.style {
                Some(style) => format!("{} [{style}]", raw.to_uppercase()),
                None => raw.to_uppercase(),
            };
            if let Some(language) = directives.translate_to {
                text = format!("{text} ({language})");
            }
            Ok(Cleaned {
                text,
                usage: Some(USAGE),
                fallback: None,
            })
        }

        fn command(&self, instruction: &str, selection: Option<&str>) -> Result<Cleaned, Error> {
            Ok(Cleaned {
                text: format!("{instruction} → {}", selection.unwrap_or("∅")),
                usage: Some(USAGE),
                fallback: None,
            })
        }
    }

    struct Failing;
    impl PostProcessor for Failing {
        fn process(&self, _: &str, _: Directives<'_>) -> Result<Cleaned, Error> {
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

        fn undo(&self) -> Result<(), Error> {
            self.0.lock().unwrap().push("<annulé>".to_owned());
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
            context_terms: Vec::new(),
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

    struct SlowAsr;
    impl Transcriber for SlowAsr {
        fn transcribe(&self, _: &AudioClip) -> Result<String, Error> {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Ok("bonjour".to_owned())
        }
    }

    #[test]
    fn asr_duration_is_kept_for_the_test_zone() {
        let sink = Recorder::default();
        let p = Pipeline::new(Box::new(SlowAsr), None, Box::new(sink));
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert!(p.last_asr_ms() >= 20, "{} ms", p.last_asr_ms());
        // Clip écarté avant l'ASR : aucune durée de la dictée précédente.
        p.run(&clip_ms(200), &no_target()).unwrap();
        assert_eq!(p.last_asr_ms(), 0);
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
                fallback: None,
                translated: None,
            }
        );
        assert_eq!(*sink.0.lock().unwrap(), ["BONJOUR"]);
    }

    #[test]
    fn fallback_answers_when_primary_fails() {
        let processor = WithFallback::new(Box::new(Failing), Box::new(Upper), "Ollama");
        let (p, sink) = pipeline("bonjour", Some(Box::new(processor)));
        match p.run(&clip_ms(1000), &no_target()).unwrap() {
            Outcome::Inserted {
                text,
                post_process_error,
                fallback,
                ..
            } => {
                assert_eq!(text, "BONJOUR");
                assert_eq!(post_process_error, None);
                assert_eq!(fallback.as_deref(), Some("Ollama"));
            }
            other => panic!("inattendu : {other:?}"),
        }
        assert_eq!(*sink.0.lock().unwrap(), ["BONJOUR"]);
    }

    #[test]
    fn fallback_is_skipped_when_primary_succeeds() {
        let processor = WithFallback::new(Box::new(Upper), Box::new(Failing), "Ollama");
        let cleaned = processor.process("salut", Directives::default()).unwrap();
        assert_eq!(cleaned.text, "SALUT");
        assert_eq!(cleaned.fallback, None);
    }

    #[test]
    fn both_failing_falls_back_to_raw_with_both_errors() {
        let processor = WithFallback::new(Box::new(Failing), Box::new(Failing), "Ollama");
        let (p, sink) = pipeline("bonjour", Some(Box::new(processor)));
        match p.run(&clip_ms(1000), &no_target()).unwrap() {
            Outcome::Inserted {
                text,
                post_process_error: Some(error),
                fallback: None,
                ..
            } => {
                assert_eq!(text, "bonjour");
                assert!(error.contains("secours « Ollama »"), "{error}");
            }
            other => panic!("inattendu : {other:?}"),
        }
        assert_eq!(*sink.0.lock().unwrap(), ["bonjour"]);
    }

    #[test]
    fn fallback_also_serves_command_mode() {
        let processor = WithFallback::new(Box::new(Failing), Box::new(Upper), "Ollama");
        let cleaned = processor.command("traduis", Some("bonjour")).unwrap();
        assert_eq!(cleaned.text, "traduis → bonjour");
        assert_eq!(cleaned.fallback.as_deref(), Some("Ollama"));
    }

    #[test]
    fn voice_commands_undo_and_layout() {
        let (p, sink) = pipeline("Annule la dernière dictée.", Some(Box::new(Upper)));
        let p = p.with_voice_commands(true);
        assert_eq!(
            p.run(&clip_ms(1000), &no_target()).unwrap(),
            Outcome::Undone
        );
        assert_eq!(*sink.0.lock().unwrap(), ["<annulé>"]);

        let (p, sink) = pipeline("Salut, nouvelle ligne, ça va", None);
        let p = p.with_voice_commands(true);
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["Salut,\nça va"]);
    }

    #[test]
    fn spelled_word_is_joined_before_cleanup() {
        let (p, sink) = pipeline(
            "Mon pseudo est A, Q, U, A, D, R, O, X.",
            Some(Box::new(Upper)),
        );
        let p = p.with_voice_commands(true);
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["MON PSEUDO EST AQUADROX."]);

        let (p, sink) = pipeline("Mon pseudo est A, Q, U, A.", None);
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            ["Mon pseudo est A, Q, U, A."],
            "avec les commandes vocales seulement"
        );
    }

    #[test]
    fn voice_commands_are_off_by_default() {
        let (p, sink) = pipeline("Annule la dernière dictée.", None);
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["Annule la dernière dictée."]);
    }

    #[test]
    fn trailing_line_break_survives_cleanup() {
        let (p, sink) = pipeline("Merci. Nouvelle ligne.", Some(Box::new(Upper)));
        let p = p.with_voice_commands(true);
        p.run(&clip_ms(1000), &no_target()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), ["MERCI.\n"]);
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
                fallback: None,
                translated: None,
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
    fn translation_reaches_post_processor_even_for_raw_styles() {
        let (p, sink) = pipeline("salut", Some(Box::new(Upper)));
        let p = p.with_styles(&styles());
        let outcome = p
            .dictate(&clip_ms(1000), &target("code.exe"), None, Some("anglais"))
            .unwrap();
        assert!(matches!(
            outcome,
            Outcome::Inserted { translated: Some(ref l), .. } if l == "anglais"
        ));
        assert_eq!(*sink.0.lock().unwrap(), ["SALUT (anglais)"]);
    }

    #[test]
    fn translation_without_llm_inserts_raw_with_error() {
        let (p, sink) = pipeline("salut", None);
        let outcome = p
            .dictate(&clip_ms(1000), &no_target(), None, Some("anglais"))
            .unwrap();
        assert!(matches!(
            outcome,
            Outcome::Inserted {
                translated: None,
                post_process_error: Some(_),
                ..
            }
        ));
        assert_eq!(*sink.0.lock().unwrap(), ["salut"]);
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

    #[test]
    fn context_terms_reach_both_asr_and_post_processor() {
        struct HintAsr(Arc<Mutex<Vec<String>>>);
        impl Transcriber for HintAsr {
            fn transcribe(&self, _: &AudioClip) -> Result<String, Error> {
                Ok("bonjour".into())
            }
            fn transcribe_with_hints(
                &self,
                _: &AudioClip,
                hints: &[String],
            ) -> Result<String, Error> {
                *self.0.lock().unwrap() = hints.to_vec();
                Ok("bonjour".into())
            }
        }
        struct HintLlm(Arc<Mutex<Vec<String>>>);
        impl PostProcessor for HintLlm {
            fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
                *self.0.lock().unwrap() = directives.context_terms.to_vec();
                Ok(Cleaned {
                    text: raw.to_owned(),
                    usage: None,
                    fallback: None,
                })
            }
        }

        let asr_seen = Arc::new(Mutex::new(Vec::new()));
        let llm_seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Recorder::default();
        let p = Pipeline::new(
            Box::new(HintAsr(Arc::clone(&asr_seen))),
            Some(Box::new(HintLlm(Arc::clone(&llm_seen)))),
            Box::new(sink),
        );
        let t = Target {
            app: Some("code.exe".into()),
            title: Some("sync.rs".into()),
            context_terms: vec!["SyncServer".into(), "ChaCha20".into()],
        };
        p.run(&clip_ms(1000), &t).unwrap();
        assert_eq!(*asr_seen.lock().unwrap(), ["SyncServer", "ChaCha20"]);
        assert_eq!(*llm_seen.lock().unwrap(), ["SyncServer", "ChaCha20"]);
    }

    #[test]
    fn live_session_chunks_transcribe_and_finalize_in_one_llm_pass() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct SeqAsr {
            replies: Vec<&'static str>,
            index: AtomicUsize,
            hints_seen: Arc<Mutex<Vec<Vec<String>>>>,
        }
        impl Transcriber for SeqAsr {
            fn transcribe(&self, _: &AudioClip) -> Result<String, Error> {
                let i = self.index.fetch_add(1, Ordering::Relaxed);
                Ok(self.replies.get(i).copied().unwrap_or("").to_owned())
            }
            fn transcribe_with_hints(
                &self,
                clip: &AudioClip,
                hints: &[String],
            ) -> Result<String, Error> {
                self.hints_seen.lock().unwrap().push(hints.to_vec());
                self.transcribe(clip)
            }
        }

        struct CountingUpper(Arc<AtomicUsize>);
        impl PostProcessor for CountingUpper {
            fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Upper.process(raw, directives)
            }
        }

        let hints_seen = Arc::new(Mutex::new(Vec::new()));
        let llm_calls = Arc::new(AtomicUsize::new(0));
        let sink = Recorder::default();
        let p = Pipeline::new(
            Box::new(SeqAsr {
                replies: vec![
                    "première phrase.",
                    "Sous-titrage ST' 501",
                    "nouvelle ligne, deuxième phrase.",
                ],
                index: AtomicUsize::new(0),
                hints_seen: Arc::clone(&hints_seen),
            }),
            Some(Box::new(CountingUpper(Arc::clone(&llm_calls)))),
            Box::new(sink.clone()),
        )
        .with_voice_commands(true)
        .with_styles(&styles());

        let t = Target {
            app: Some("slack.exe".into()),
            title: None,
            context_terms: vec!["VoiceToText".into()],
        };

        let mut session = crate::live::LiveSession::with_bounds(1_000, 3_000);
        let speech = |s: f32| {
            (0..(s * SAMPLE_RATE as f32) as usize)
                .map(|i| if i % 2 == 0 { 0.2 } else { -0.2 })
                .collect::<Vec<_>>()
        };
        let silence = |s: f32| vec![0.0f32; (s * SAMPLE_RATE as f32) as usize];

        let mut stream = Vec::new();
        stream.extend(speech(1.5));
        stream.extend(silence(0.4));
        stream.extend(speech(1.5));
        stream.extend(silence(0.4));
        stream.extend(speech(1.2));

        for chunk in session.feed(&stream) {
            if let Some((text, ms)) = p.transcribe_chunk(&chunk, &t.context_terms).unwrap() {
                session.push_text(&text, ms);
            }
        }
        assert_eq!(session.preview(80), "première phrase.");
        assert_eq!(llm_calls.load(Ordering::Relaxed), 0);
        assert!(sink.0.lock().unwrap().is_empty());

        if let Some(tail) = session.finish()
            && let Some((text, ms)) = p.transcribe_chunk(&tail, &t.context_terms).unwrap()
        {
            session.push_text(&text, ms);
        }

        let mut raw = session.to_raw();
        raw.asr_ms = 420;
        let outcome = p.finalize_raw(&raw, &t, None, Some("anglais")).unwrap();
        assert_eq!(llm_calls.load(Ordering::Relaxed), 1);
        assert_eq!(p.last_asr_ms(), 420);
        assert_eq!(hints_seen.lock().unwrap().len(), 3);
        assert!(
            hints_seen
                .lock()
                .unwrap()
                .iter()
                .all(|h| h == &["VoiceToText"])
        );
        assert_eq!(
            outcome,
            Outcome::Inserted {
                raw: "première phrase.\nDeuxième phrase.".into(),
                text: "PREMIÈRE PHRASE.\nDEUXIÈME PHRASE. [décontracté] (anglais)".into(),
                post_process_error: None,
                snippet: false,
                usage: Some(USAGE),
                command: false,
                fallback: None,
                translated: Some("anglais".into()),
            }
        );
        assert_eq!(
            *sink.0.lock().unwrap(),
            ["PREMIÈRE PHRASE.\nDEUXIÈME PHRASE. [décontracté] (anglais)"]
        );
    }

    #[test]
    fn finalize_raw_skips_empty_or_hallucination_only() {
        let (p, sink) = pipeline("inutilisé", Some(Box::new(Upper)));
        let empty = RawDictation {
            raw: "   Sous-titrage ST' 501  ".into(),
            duration_ms: 4_000,
            asr_ms: 150,
        };
        assert_eq!(
            p.finalize_raw(&empty, &no_target(), None, None).unwrap(),
            Outcome::Skipped
        );
        assert_eq!(p.last_asr_ms(), 0);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn transcribe_chunk_skips_short_and_silent() {
        let (p, _) = pipeline("bonjour", None);
        assert_eq!(p.transcribe_chunk(&clip_ms(100), &[]).unwrap(), None);
        let silent = AudioClip {
            samples: vec![0.0; SAMPLE_RATE as usize],
        };
        assert_eq!(p.transcribe_chunk(&silent, &[]).unwrap(), None);
        let (text, _ms) = p
            .transcribe_chunk(&clip_ms(1000), &[])
            .unwrap()
            .expect("tranche valide");
        assert_eq!(text, "bonjour");
    }

    /// Nettoyage qui note le contexte reçu et met le texte en majuscules.
    #[derive(Clone, Default)]
    struct Contextual(Arc<Mutex<Vec<Option<String>>>>);
    impl PostProcessor for Contextual {
        fn process(&self, raw: &str, directives: Directives<'_>) -> Result<Cleaned, Error> {
            self.0
                .lock()
                .unwrap()
                .push(directives.preceding.map(str::to_owned));
            Ok(Cleaned {
                text: raw.to_uppercase(),
                usage: Some(USAGE),
                fallback: None,
            })
        }
    }

    #[test]
    fn live_separator_rules() {
        assert_eq!(live_separator("", "Bonjour"), "");
        assert_eq!(live_separator("Bonjour.", "Ça va ?"), " ");
        assert_eq!(live_separator("Bonjour.\n", "Ça va ?"), "");
        assert_eq!(live_separator("Bonjour ", "Ça va ?"), "");
        assert_eq!(live_separator("Bonjour", ", ça va ?"), "");
        assert_eq!(live_separator("l'", "avion"), "");
        assert_eq!(live_separator("Bonjour.", "\nÇa va ?"), "");
    }

    #[test]
    fn live_chunks_are_cleaned_with_preceding_text_and_inserted_at_once() {
        let post = Contextual::default();
        let (p, sink) = pipeline("inutilisé", Some(Box::new(post.clone())));
        let mut state = LiveInsertion::new();
        let first = p
            .insert_live_chunk(&mut state, "bonjour à tous.", &no_target(), None, None)
            .unwrap();
        assert_eq!(first.as_deref(), Some("BONJOUR À TOUS."));
        let second = p
            .insert_live_chunk(&mut state, "on commence", &no_target(), None, None)
            .unwrap();
        assert_eq!(second.as_deref(), Some(" ON COMMENCE"));
        // Silence ou hallucination : rien.
        assert_eq!(
            p.insert_live_chunk(
                &mut state,
                " Sous-titrage ST' 501 ",
                &no_target(),
                None,
                None
            )
            .unwrap(),
            None
        );
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec!["BONJOUR À TOUS.".to_owned(), " ON COMMENCE".to_owned()]
        );
        // La première tranche n'a pas de contexte, la seconde reçoit la première.
        assert_eq!(
            *post.0.lock().unwrap(),
            vec![None, Some("BONJOUR À TOUS.".to_owned())]
        );
        assert_eq!(state.inserted(), "BONJOUR À TOUS. ON COMMENCE");

        let outcome = p.finish_live_insertion(state, 12_000, 900, None);
        let Outcome::Inserted {
            raw,
            text,
            usage,
            post_process_error,
            snippet,
            ..
        } = outcome
        else {
            panic!("dictée insérée attendue");
        };
        assert_eq!(raw, "bonjour à tous. on commence");
        assert_eq!(text, "BONJOUR À TOUS. ON COMMENCE");
        assert_eq!(usage.unwrap().input_tokens, 2 * USAGE.input_tokens);
        assert_eq!(post_process_error, None);
        assert!(!snippet);
        assert_eq!(p.last_asr_ms(), 900);
    }

    #[test]
    fn live_context_is_bounded_to_the_end_of_inserted_text() {
        let post = Contextual::default();
        let (p, _) = pipeline("inutilisé", Some(Box::new(post.clone())));
        let mut state = LiveInsertion::new();
        let long = "é".repeat(LIVE_CONTEXT_CHARS + 50);
        p.insert_live_chunk(&mut state, &long, &no_target(), None, None)
            .unwrap();
        p.insert_live_chunk(&mut state, "fin", &no_target(), None, None)
            .unwrap();
        let seen = post.0.lock().unwrap()[1].clone().unwrap();
        assert_eq!(seen.chars().count(), LIVE_CONTEXT_CHARS);
    }

    #[test]
    fn live_llm_failure_inserts_raw_chunk_and_keeps_first_error() {
        let (p, sink) = pipeline("inutilisé", Some(Box::new(Failing)));
        let mut state = LiveInsertion::new();
        p.insert_live_chunk(&mut state, "Bonjour.", &no_target(), None, None)
            .unwrap();
        p.insert_live_chunk(&mut state, "Au revoir.", &no_target(), None, None)
            .unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec!["Bonjour.".to_owned(), " Au revoir.".to_owned()]
        );
        let Outcome::Inserted {
            post_process_error, ..
        } = p.finish_live_insertion(state, 8_000, 500, None)
        else {
            panic!("dictée insérée attendue");
        };
        assert!(post_process_error.unwrap().contains("délai dépassé"));
    }

    #[test]
    fn live_voice_commands_snippets_and_undo() {
        let (p, sink) = pipeline("inutilisé", Some(Box::new(Upper)));
        let p = p.with_voice_commands(true).with_snippets(&snippets());
        let mut state = LiveInsertion::new();
        p.insert_live_chunk(
            &mut state,
            "Bonjour Paul, nouvelle ligne.",
            &no_target(),
            None,
            None,
        )
        .unwrap();
        // Retour à la ligne conservé : pas d'espace avant la suite.
        let next = p
            .insert_live_chunk(&mut state, "ma signature", &no_target(), None, None)
            .unwrap();
        assert_eq!(next.as_deref(), Some("Florian\nVoiceToText"));
        p.insert_live_chunk(&mut state, "Phrase en trop.", &no_target(), None, None)
            .unwrap();
        assert_eq!(
            p.insert_live_chunk(
                &mut state,
                "Annule la dernière dictée.",
                &no_target(),
                None,
                None
            )
            .unwrap(),
            None
        );
        assert_eq!(
            sink.0.lock().unwrap().last().map(String::as_str),
            Some("<annulé>")
        );
        assert_eq!(state.inserted(), "BONJOUR PAUL,\nFlorian\nVoiceToText");
        assert_eq!(state.len(), 2);
        let Outcome::Inserted { snippet, .. } = p.finish_live_insertion(state, 9_000, 700, None)
        else {
            panic!("dictée insérée attendue");
        };
        assert!(!snippet, "une seule tranche était un snippet");
    }

    #[test]
    fn live_insertion_without_any_chunk_is_skipped_or_undone() {
        let (p, _) = pipeline("inutilisé", None);
        let p = p.with_voice_commands(true);
        assert_eq!(
            p.finish_live_insertion(LiveInsertion::new(), 1_000, 0, None),
            Outcome::Skipped
        );
        let mut state = LiveInsertion::new();
        p.insert_live_chunk(
            &mut state,
            "annule la dernière dictée",
            &no_target(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            p.finish_live_insertion(state, 1_000, 0, None),
            Outcome::Undone
        );
    }

    #[test]
    fn live_translation_is_reported_per_dictation() {
        let (p, sink) = pipeline("inutilisé", Some(Box::new(Upper)));
        let mut state = LiveInsertion::new();
        p.insert_live_chunk(&mut state, "bonjour", &no_target(), None, Some("anglais"))
            .unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec!["BONJOUR (anglais)".to_owned()]
        );
        let Outcome::Inserted { translated, .. } =
            p.finish_live_insertion(state, 2_000, 100, Some("anglais"))
        else {
            panic!("dictée insérée attendue");
        };
        assert_eq!(translated.as_deref(), Some("anglais"));
    }

    #[test]
    fn audio_enhance_filters_dc_hum_before_asr_and_boosts_quiet_speech() {
        struct InspectAsr(Arc<Mutex<Vec<f32>>>);
        impl Transcriber for InspectAsr {
            fn transcribe(&self, clip: &AudioClip) -> Result<String, Error> {
                self.0
                    .lock()
                    .unwrap()
                    .push(audio::rms(&clip.samples[4000..]));
                Ok("bonjour".into())
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Recorder::default();
        let mut p = Pipeline::new(
            Box::new(InspectAsr(Arc::clone(&seen))),
            None,
            Box::new(sink.clone()),
        )
        .with_audio_enhance(true);

        // Décalage continu pur : filtré avant is_silent, l'ASR n'est même pas appelé.
        let dc_only = AudioClip {
            samples: vec![0.05; SAMPLE_RATE as usize],
        };
        assert_eq!(p.run(&dc_only, &no_target()).unwrap(), Outcome::Skipped);
        assert!(seen.lock().unwrap().is_empty());

        // Voix faible 440 Hz (~0,014 RMS) : remontée vers ~0,09 RMS avant l'ASR.
        let quiet_speech = AudioClip {
            samples: (0..SAMPLE_RATE as usize)
                .map(|i| {
                    0.02 * (i as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin()
                })
                .collect(),
        };
        assert!(matches!(
            p.run(&quiet_speech, &no_target()).unwrap(),
            Outcome::Inserted { .. }
        ));
        let boosted = seen.lock().unwrap()[0];
        assert!(boosted > 0.07 && boosted < 0.11, "RMS reçu : {boosted}");

        // Désactivé à chaud : le niveau brut est transmis tel quel.
        p.set_audio_enhance(false);
        assert!(matches!(
            p.run(&quiet_speech, &no_target()).unwrap(),
            Outcome::Inserted { .. }
        ));
        let raw_level = seen.lock().unwrap()[1];
        assert!(raw_level < 0.02, "RMS brut attendu : {raw_level}");
    }
}
