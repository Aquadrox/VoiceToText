//! Rapport complet d'un enregistrement : synthèse, points clés, chapitres
//! (avec leur passage de transcription), analyse, plan d'action.
//!
//! Le LLM répond en JSON (clés françaises, heures `mm:ss`) ; ce module le
//! lit avec indulgence (bloc ```json, texte autour, liste de chaînes au lieu
//! d'objets…) puis le vérifie : une heure qui n'existe pas dans
//! l'enregistrement est retirée, les chapitres sont remis dans l'ordre et
//! couvrent tout l'enregistrement. Rendu en HTML autonome (sommaire,
//! recherche, filtres, sans aucune ressource extérieure) et en Markdown.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::record::MeetingRecord;
use super::{Reply, Track, speaker_label, timestamp, turns};

/// Au plus autant d'éléments par liste : une réponse aberrante du LLM ne
/// produit pas un document interminable.
const MAX_ITEMS: usize = 60;
/// Tolérance sur une heure citée au-delà de la fin (arrondis du LLM).
const TIME_SLACK_MS: u64 = 5_000;

const CSS: &str = include_str!("report.css");
const JS: &str = include_str!("report.js");

/// Élément daté : point clé, décision, risque…
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub text: String,
    /// Moment où c'est dit (ms depuis le début), s'il est connu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub text: String,
    /// Qui parle (vide : inconnu).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub who: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<Quote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qa {
    pub question: String,
    pub answer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Haute,
    #[default]
    Normale,
    Basse,
}

impl Priority {
    fn parse(value: &str) -> Self {
        let value = crate::search::fold(value.trim());
        if ["haut", "high", "eleve", "urgent", "critique", "forte"]
            .iter()
            .any(|w| value.starts_with(w))
        {
            Priority::Haute
        } else if ["bas", "low", "faible"]
            .iter()
            .any(|w| value.starts_with(w))
        {
            Priority::Basse
        } else {
            Priority::Normale
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Priority::Haute => "Haute",
            Priority::Normale => "Normale",
            Priority::Basse => "Basse",
        }
    }
}

/// Action du plan d'action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub task: String,
    /// Responsable (vide : à définir).
    #[serde(default)]
    pub owner: String,
    /// Échéance telle que dite (« vendredi », « fin mars »), vide si aucune.
    #[serde(default)]
    pub due: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    /// Cochée par l'utilisateur.
    #[serde(default)]
    pub done: bool,
}

/// Rapport, gardé dans `meeting.json` (champ `report`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeetingReport {
    pub synthesis: String,
    #[serde(default)]
    pub key_points: Vec<Point>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub agreements: Vec<Point>,
    #[serde(default)]
    pub disagreements: Vec<Point>,
    #[serde(default)]
    pub risks: Vec<Point>,
    #[serde(default)]
    pub figures: Vec<Point>,
    #[serde(default)]
    pub open_questions: Vec<Point>,
    #[serde(default)]
    pub qa: Vec<Qa>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub at_ms: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub currency: String,
}

// ---- Lecture de la réponse du LLM ----

/// Objet JSON de la réponse : bloc ```json, phrase avant ou après, tout est
/// toléré tant qu'un objet `{…}` s'y trouve.
pub fn parse_json(content: &str) -> Result<Value, String> {
    let start = content.find('{').ok_or("réponse sans objet JSON")?;
    let end = content.rfind('}').ok_or("réponse sans objet JSON")?;
    if end < start {
        return Err("réponse sans objet JSON".into());
    }
    let value: Value =
        serde_json::from_str(&content[start..=end]).map_err(|e| format!("JSON illisible : {e}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("réponse sans objet JSON".into())
    }
}

/// Heure citée : `"12:34"`, `"1:02:03"`, `"[12:34]"`, `"12:34–13:10"`
/// (le début), ou un nombre de secondes. `None` si elle est illisible ou
/// au-delà de la fin.
pub fn parse_time(value: &Value, duration_ms: u64) -> Option<u64> {
    let ms = match value {
        Value::Number(n) => (n.as_f64()? * 1000.0) as u64,
        Value::String(s) => {
            let s = s.trim().trim_start_matches('[');
            let digits: String = s
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == ':')
                .collect();
            let parts: Vec<u64> = digits
                .split(':')
                .map(|p| p.parse().ok())
                .collect::<Option<_>>()?;
            let seconds = match parts[..] {
                [m, s] if s < 60 => m * 60 + s,
                [h, m, s] if m < 60 && s < 60 => h * 3600 + m * 60 + s,
                _ => return None,
            };
            seconds * 1000
        }
        _ => return None,
    };
    (ms <= duration_ms + TIME_SLACK_MS).then(|| ms.min(duration_ms))
}

fn field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|k| value.get(*k))
        .filter(|v| !v.is_null())
}

fn string(value: &Value, keys: &[&str]) -> String {
    match field(value, keys) {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn time(value: &Value, keys: &[&str], duration_ms: u64) -> Option<u64> {
    field(value, keys).and_then(|v| parse_time(v, duration_ms))
}

fn list<'a>(value: &'a Value, keys: &[&str]) -> impl Iterator<Item = &'a Value> {
    let items: &[Value] = match field(value, keys) {
        Some(Value::Array(items)) => items,
        Some(single @ (Value::Object(_) | Value::String(_))) => std::slice::from_ref(single),
        _ => &[],
    };
    items.iter().take(MAX_ITEMS)
}

const TEXT: &[&str] = &["texte", "text", "point", "contenu"];
const T: &[&str] = &["t", "heure", "time", "debut"];

fn points(value: &Value, keys: &[&str], duration_ms: u64) -> Vec<Point> {
    list(value, keys)
        .filter_map(|item| {
            let (text, at_ms) = match item {
                Value::String(s) => (s.trim().to_owned(), None),
                _ => (string(item, TEXT), time(item, T, duration_ms)),
            };
            (!text.is_empty()).then_some(Point { text, at_ms })
        })
        .collect()
}

fn quotes(value: &Value, duration_ms: u64) -> Vec<Quote> {
    list(value, &["citations", "quotes"])
        .filter_map(|item| {
            let quote = match item {
                Value::String(s) => Quote {
                    text: s.trim().to_owned(),
                    who: String::new(),
                    at_ms: None,
                },
                _ => Quote {
                    text: string(item, TEXT),
                    who: string(item, &["qui", "who", "auteur"]),
                    at_ms: time(item, T, duration_ms),
                },
            };
            let text = quote.text.trim_matches(['«', '»', '"', ' ']).to_owned();
            (!text.is_empty()).then_some(Quote { text, ..quote })
        })
        .collect()
}

fn chapters(value: &Value, duration_ms: u64) -> Vec<Chapter> {
    list(value, &["chapitres", "chapters"])
        .filter(|item| item.is_object())
        .map(|item| Chapter {
            title: string(item, &["titre", "title"]),
            start_ms: time(item, &["debut", "début", "start", "t"], duration_ms).unwrap_or(0),
            end_ms: time(item, &["fin", "end"], duration_ms).unwrap_or(0),
            summary: string(item, &["resume", "résumé", "summary"]),
            points: points(item, &["points", "points_cles"], duration_ms),
            decisions: points(item, &["decisions", "décisions"], duration_ms),
            quotes: quotes(item, duration_ms),
        })
        .collect()
}

fn actions(value: &Value, duration_ms: u64) -> Vec<Action> {
    list(value, &["actions", "plan_action"])
        .filter_map(|item| {
            let action = match item {
                Value::String(s) => Action {
                    task: s.trim().to_owned(),
                    owner: String::new(),
                    due: String::new(),
                    priority: Priority::Normale,
                    at_ms: None,
                    done: false,
                },
                _ => Action {
                    task: string(item, &["tache", "tâche", "action", "task", "texte"]),
                    owner: string(item, &["responsable", "qui", "owner"]),
                    due: string(item, &["echeance", "échéance", "due"]),
                    priority: Priority::parse(&string(item, &["priorite", "priorité", "priority"])),
                    at_ms: time(item, T, duration_ms),
                    done: false,
                },
            };
            let unknown = |s: &str| {
                let s = crate::search::fold(s.trim());
                s.is_empty()
                    || ["n/a", "-", "?", "aucun", "aucune"].contains(&s.as_str())
                    || [
                        "a definir",
                        "inconnu",
                        "non precis",
                        "non mentionn",
                        "non indiqu",
                    ]
                    .iter()
                    .any(|w| s.starts_with(w))
            };
            if action.task.is_empty() {
                return None;
            }
            Some(Action {
                owner: if unknown(&action.owner) {
                    String::new()
                } else {
                    action.owner.clone()
                },
                due: if unknown(&action.due) {
                    String::new()
                } else {
                    action.due.clone()
                },
                ..action
            })
        })
        .collect()
}

fn qa(value: &Value, duration_ms: u64) -> Vec<Qa> {
    list(value, &["questions_reponses", "qa"])
        .filter(|item| item.is_object())
        .map(|item| Qa {
            question: string(item, &["question", "q"]),
            answer: string(item, &["reponse", "réponse", "answer", "r"]),
            at_ms: time(item, T, duration_ms),
        })
        .filter(|qa| !qa.question.is_empty())
        .collect()
}

impl MeetingReport {
    /// Rapport tiré de la réponse JSON du LLM, vérifié (sans les chapitres
    /// remis en ordre : voir [`MeetingReport::finish`]).
    pub fn from_json(value: &Value, duration_ms: u64) -> Self {
        Self {
            synthesis: string(value, &["synthese", "synthèse", "synthesis", "resume"]),
            key_points: points(
                value,
                &["points_cles", "points_clés", "key_points"],
                duration_ms,
            ),
            chapters: chapters(value, duration_ms),
            agreements: points(value, &["accords", "agreements"], duration_ms),
            disagreements: points(
                value,
                &["desaccords", "désaccords", "disagreements"],
                duration_ms,
            ),
            risks: points(value, &["risques", "risks"], duration_ms),
            figures: points(value, &["chiffres", "figures"], duration_ms),
            open_questions: points(
                value,
                &["questions_ouvertes", "open_questions"],
                duration_ms,
            ),
            qa: qa(value, duration_ms),
            actions: actions(value, duration_ms),
            ..Self::default()
        }
    }

    /// Chapitres dans l'ordre, sans chevauchement, couvrant tout
    /// l'enregistrement (au moins un). Un chapitre sans titre est nommé
    /// « Partie N ».
    pub fn finish(&mut self, duration_ms: u64) {
        self.chapters.sort_by_key(|c| c.start_ms);
        self.chapters.dedup_by(|b, a| {
            // Même début : le second rejoint le premier.
            if a.start_ms == b.start_ms {
                a.points.append(&mut b.points);
                a.decisions.append(&mut b.decisions);
                a.quotes.append(&mut b.quotes);
                true
            } else {
                false
            }
        });
        if self.chapters.is_empty() {
            self.chapters.push(Chapter {
                title: "Enregistrement complet".into(),
                start_ms: 0,
                end_ms: duration_ms,
                summary: String::new(),
                points: Vec::new(),
                decisions: Vec::new(),
                quotes: Vec::new(),
            });
        }
        self.chapters[0].start_ms = 0;
        let starts: Vec<u64> = self.chapters.iter().skip(1).map(|c| c.start_ms).collect();
        for (i, chapter) in self.chapters.iter_mut().enumerate() {
            chapter.end_ms = starts.get(i).copied().unwrap_or(duration_ms);
            if chapter.title.trim().is_empty() {
                chapter.title = format!("Partie {}", i + 1);
            }
        }
    }

    /// Nouvelle rédaction : les actions déjà cochées le restent si elles
    /// sont reprises à l'identique.
    pub fn keep_done(&mut self, previous: &MeetingReport) {
        let done: Vec<String> = previous
            .actions
            .iter()
            .filter(|a| a.done)
            .map(|a| crate::search::fold(a.task.trim()))
            .collect();
        for action in &mut self.actions {
            if done.contains(&crate::search::fold(action.task.trim())) {
                action.done = true;
            }
        }
    }

    pub fn has_analysis(&self) -> bool {
        !(self.agreements.is_empty()
            && self.disagreements.is_empty()
            && self.risks.is_empty()
            && self.figures.is_empty()
            && self.open_questions.is_empty())
    }

    /// Sections de l'analyse : (ancre, titre, classe des éléments, liste).
    fn analysis(&self) -> [(&'static str, &'static str, &'static str, &[Point]); 5] {
        [
            ("accords", "Points d'accord", "agreement", &self.agreements),
            (
                "desaccords",
                "Désaccords et tensions",
                "disagreement",
                &self.disagreements,
            ),
            ("risques", "Risques et vigilance", "risk", &self.risks),
            ("chiffres", "Chiffres et données", "figure", &self.figures),
            (
                "questions",
                "Questions ouvertes",
                "question",
                &self.open_questions,
            ),
        ]
    }
}

// ---- Rendu ----

/// Réplique de la transcription, avec son nom affiché.
struct Line {
    reply: Reply,
    label: Option<String>,
    end_ms: u64,
}

/// Ce qui vient de la fiche de la réunion pour le rendu.
struct Context {
    title: String,
    date: String,
    duration_ms: u64,
    conference: bool,
    lines: Vec<Line>,
    /// (nom, temps de parole, couleur) des intervenants.
    people: Vec<(String, u64, Option<usize>)>,
}

/// Options du document HTML.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
    /// Affiché dans l'application : les cases du plan d'action et les ▶
    /// appellent l'objet `VTT` fourni par l'application.
    pub app: bool,
    /// Boutons ▶ de réécoute (audio disponible ; seulement avec `app`).
    pub replay: bool,
    /// `"light"` ou `"dark"` imposé ; `None` : celui du système du lecteur.
    pub theme: Option<String>,
    /// Nommer les pistes (« Moi », « Eux ») quand l'intervenant n'est pas
    /// connu (PC : deux sources).
    pub tracks: bool,
}

fn context(record: &MeetingRecord, date: &str, tracks: bool) -> Context {
    let names = record.names();
    let replies = turns(&record.segments());
    let lines: Vec<Line> = replies
        .iter()
        .enumerate()
        .map(|(i, r)| Line {
            label: (tracks || r.speaker.is_some())
                .then(|| speaker_label(r.track, r.speaker, &names)),
            end_ms: replies
                .get(i + 1)
                .map_or(record.duration_ms, |n| n.start_ms)
                .max(r.start_ms),
            reply: r.clone(),
        })
        .collect();
    let mut people: Vec<(String, u64, Option<usize>)> = Vec::new();
    if !record.speaker_turns.is_empty() {
        let track = record.speaker_track.unwrap_or(Track::Eux);
        let count = record
            .speaker_turns
            .iter()
            .map(|t| t.speaker + 1)
            .max()
            .unwrap_or(0);
        for speaker in 0..count {
            let talk = record
                .speaker_turns
                .iter()
                .filter(|t| t.speaker == speaker)
                .map(|t| t.end_ms.saturating_sub(t.start_ms))
                .sum();
            people.push((
                speaker_label(track, Some(speaker), &names),
                talk,
                Some(speaker),
            ));
        }
        people.sort_by_key(|p| std::cmp::Reverse(p.1));
    } else if tracks && record.mic && record.system {
        for track in [Track::Moi, Track::Eux] {
            let talk = record
                .segments()
                .iter()
                .filter(|s| s.track == track)
                .map(|s| s.end_ms.saturating_sub(s.start_ms))
                .sum();
            people.push((track.label().to_owned(), talk, None));
        }
    }
    Context {
        title: record.title.clone(),
        date: date.to_owned(),
        duration_ms: record.duration_ms,
        conference: record.conference_summary(),
        lines,
        people,
    }
}

/// Texte échappé pour le HTML (contenu et attributs).
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn time_link(at_ms: Option<u64>) -> String {
    match at_ms {
        Some(ms) => format!(
            " <a class=\"t\" href=\"#\" data-ms=\"{ms}\" title=\"Voir dans la transcription\">{}</a>",
            timestamp(ms)
        ),
        None => String::new(),
    }
}

fn point_items(points: &[Point], kind: &str) -> String {
    points
        .iter()
        .map(|p| {
            format!(
                "<li class=\"item k-{kind}\">{}{}</li>",
                esc(&p.text),
                time_link(p.at_ms)
            )
        })
        .collect()
}

fn group(title: &str, body: &str, class: &str) -> String {
    if body.is_empty() {
        return String::new();
    }
    format!("<div class=\"group\"><h4>{title}</h4><ul class=\"{class}\">{body}</ul></div>")
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count > 1 { many } else { one })
}

impl MeetingRecord {
    /// Rapport en HTML autonome (sommaire, recherche, filtres), ou `None`
    /// s'il n'a pas encore été rédigé. `date` : date lisible de la réunion.
    pub fn report_html(&self, date: &str, options: &HtmlOptions) -> Option<String> {
        let report = self.report.as_ref()?;
        Some(render_html(
            report,
            &context(self, date, options.tracks),
            options,
        ))
    }

    /// Rapport en Markdown, sans la transcription (à coller dans un e-mail,
    /// un wiki…).
    pub fn report_markdown(&self, date: &str, tracks: bool) -> Option<String> {
        let report = self.report.as_ref()?;
        Some(render_markdown(report, &context(self, date, tracks)))
    }
}

fn render_html(report: &MeetingReport, ctx: &Context, options: &HtmlOptions) -> String {
    let theme = options
        .theme
        .as_deref()
        .filter(|t| matches!(*t, "light" | "dark"))
        .map(|t| format!(" data-theme=\"{t}\""))
        .unwrap_or_default();
    let mode = if options.app { "app" } else { "file" };
    let mut toc = String::new();
    let mut doc = String::new();

    // En-tête.
    let kind = if ctx.conference {
        "Rapport · conférence"
    } else {
        "Rapport · réunion"
    };
    let mut meta = vec![
        esc(&ctx.date),
        format!("durée {}", timestamp(ctx.duration_ms)),
        plural(report.chapters.len(), "chapitre", "chapitres"),
    ];
    if !report.actions.is_empty() {
        meta.push(plural(report.actions.len(), "action", "actions"));
    }
    meta.retain(|m| !m.is_empty());
    doc.push_str(&format!(
        "<header class=\"head\"><p class=\"kicker\">{kind}</p><h1>{}</h1><p class=\"meta\">{}</p>",
        esc(&ctx.title),
        meta.join(" · ")
    ));
    let total: u64 = ctx.people.iter().map(|p| p.1).sum::<u64>().max(1);
    if !ctx.people.is_empty() {
        doc.push_str("<ul class=\"people\" aria-label=\"Intervenants\">");
        for (name, talk, speaker) in &ctx.people {
            let color = speaker.map(|s| format!(" s{}", s % 6)).unwrap_or_default();
            doc.push_str(&format!(
                "<li><span class=\"dot{color}\"></span>{} <span class=\"muted\">{} · {} %</span></li>",
                esc(name),
                timestamp(*talk),
                talk * 100 / total
            ));
        }
        doc.push_str("</ul>");
    }
    doc.push_str("</header>");

    // Synthèse et points clés.
    toc.push_str("<li><a href=\"#synthese\">Synthèse</a><span class=\"hits\"></span></li>");
    doc.push_str("<section id=\"synthese\" data-section><h2>Synthèse</h2>");
    for paragraph in report
        .synthesis
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
    {
        doc.push_str(&format!("<p class=\"prose\">{}</p>", esc(paragraph.trim())));
    }
    if !report.key_points.is_empty() {
        doc.push_str(&format!(
            "<h3>Points clés</h3><ul class=\"points key\">{}</ul>",
            point_items(&report.key_points, "point")
        ));
    }
    doc.push_str("</section>");

    // Plan d'action.
    if !report.actions.is_empty() {
        let done = report.actions.iter().filter(|a| a.done).count();
        toc.push_str(
            "<li><a href=\"#plan-action\">Plan d'action</a><span class=\"hits\"></span></li>",
        );
        doc.push_str(&format!(
            "<section id=\"plan-action\" data-section><h2>Plan d'action</h2>\
             <p class=\"muted\" id=\"done-count\" data-total=\"{}\">{done} / {} faite(s)</p><ol class=\"actions\">",
            report.actions.len(),
            report.actions.len()
        ));
        for (i, action) in report.actions.iter().enumerate() {
            let owner = if action.owner.is_empty() {
                "<span class=\"owner none\">Responsable à définir</span>".to_owned()
            } else {
                format!("<span class=\"owner\">{}</span>", esc(&action.owner))
            };
            let due = if action.due.is_empty() {
                String::new()
            } else {
                format!("<span class=\"due\">Échéance : {}</span>", esc(&action.due))
            };
            let priority = action.priority;
            doc.push_str(&format!(
                "<li class=\"item k-action action{}\"><label><input type=\"checkbox\" data-index=\"{i}\"{}> \
                 <span class=\"task\">{}</span></label><div class=\"facts\">{owner}{due}\
                 <span class=\"prio prio-{}\">Priorité {}</span>{}</div></li>",
                if action.done { " done" } else { "" },
                if action.done { " checked" } else { "" },
                esc(&action.task),
                priority.label().to_lowercase(),
                priority.label().to_lowercase(),
                time_link(action.at_ms),
            ));
        }
        doc.push_str("</ol></section>");
    }

    // Chapitres, avec leur passage de transcription.
    toc.push_str("<li><a href=\"#chapitres\">Chapitres</a><ol>");
    doc.push_str("<section id=\"chapitres\"><h2>Chapitres</h2>");
    let mut lines = ctx.lines.iter().peekable();
    let count = report.chapters.len();
    for (i, chapter) in report.chapters.iter().enumerate() {
        let n = i + 1;
        toc.push_str(&format!(
            "<li><a href=\"#ch-{n}\" class=\"chapter-link\"><span class=\"num\">{n}</span><span class=\"title\">{}</span><span class=\"muted\">{}</span></a><span class=\"hits\"></span></li>",
            esc(&chapter.title),
            timestamp(chapter.start_ms)
        ));
        doc.push_str(&format!(
            "<article id=\"ch-{n}\" class=\"chapter\" data-section><h3><span class=\"num\">{n}</span> {} \
             <a class=\"t range\" href=\"#\" data-ms=\"{}\">{} – {}</a></h3>",
            esc(&chapter.title),
            chapter.start_ms,
            timestamp(chapter.start_ms),
            timestamp(chapter.end_ms)
        ));
        if !chapter.summary.is_empty() {
            doc.push_str(&format!("<p class=\"prose\">{}</p>", esc(&chapter.summary)));
        }
        if !chapter.points.is_empty() {
            doc.push_str(&format!(
                "<ul class=\"points\">{}</ul>",
                point_items(&chapter.points, "point")
            ));
        }
        doc.push_str(&group(
            "Décisions",
            &point_items(&chapter.decisions, "decision"),
            "points",
        ));
        if !chapter.quotes.is_empty() {
            doc.push_str("<div class=\"group\"><h4>Citations</h4>");
            for quote in &chapter.quotes {
                let who = [
                    esc(&quote.who),
                    quote.at_ms.map(timestamp).unwrap_or_default(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let cite = match quote.at_ms {
                    Some(ms) => format!(
                        "<a class=\"t\" href=\"#\" data-ms=\"{ms}\">{}</a>",
                        if who.is_empty() { timestamp(ms) } else { who }
                    ),
                    None => who,
                };
                doc.push_str(&format!(
                    "<blockquote class=\"item k-quote\"><p>« {} »</p>{}</blockquote>",
                    esc(&quote.text),
                    if cite.is_empty() {
                        String::new()
                    } else {
                        format!("<cite>{cite}</cite>")
                    }
                ));
            }
            doc.push_str("</div>");
        }
        // Répliques du chapitre (celles d'avant le premier chapitre vont au premier).
        let last = i + 1 == count;
        let mut body = String::new();
        let mut replies = 0;
        while let Some(line) = lines.next_if(|l| last || l.reply.start_ms < chapter.end_ms) {
            replies += 1;
            body.push_str(&reply_html(line, options));
        }
        if replies > 0 {
            doc.push_str(&format!(
                "<details class=\"transcript\"><summary>Transcription du chapitre · {}</summary>{body}</details>",
                plural(replies, "réplique", "répliques")
            ));
        }
        doc.push_str("</article>");
    }
    toc.push_str("</ol></li>");
    doc.push_str("</section>");

    // Analyse.
    if report.has_analysis() {
        toc.push_str("<li><a href=\"#analyse\">Analyse</a><span class=\"hits\"></span></li>");
        doc.push_str("<section id=\"analyse\" data-section><h2>Analyse</h2>");
        for (_, title, kind, points) in report.analysis() {
            doc.push_str(&group(title, &point_items(points, kind), "points"));
        }
        doc.push_str("</section>");
    }

    // Questions et réponses (conférences).
    if !report.qa.is_empty() {
        toc.push_str("<li><a href=\"#questions-reponses\">Questions et réponses</a><span class=\"hits\"></span></li>");
        doc.push_str("<section id=\"questions-reponses\" data-section><h2>Questions et réponses</h2><dl class=\"qa\">");
        for qa in &report.qa {
            doc.push_str(&format!(
                "<div class=\"item k-qa\"><dt>{}{}</dt><dd>{}</dd></div>",
                esc(&qa.question),
                time_link(qa.at_ms),
                esc(&qa.answer)
            ));
        }
        doc.push_str("</dl></section>");
    }

    let footer = format!(
        "<footer>Rapport rédigé par {} à partir de la transcription automatique : \
         touchez une heure pour vérifier le passage. VoiceToText.</footer>",
        esc(if report.model.is_empty() {
            "un LLM"
        } else {
            &report.model
        })
    );

    // Filtres : seulement les types présents.
    let mut filters = vec![("all", "Tout")];
    let has = |kind: &str| doc.contains(&format!("k-{kind}"));
    for (kind, label) in [
        ("point", "Points clés"),
        ("decision", "Décisions"),
        ("action", "Actions"),
        ("quote", "Citations"),
        ("risk", "Risques"),
        ("question", "Questions"),
        ("qa", "Q&R"),
    ] {
        if has(kind) {
            filters.push((kind, label));
        }
    }
    let filters: String = filters
        .iter()
        .map(|(kind, label)| {
            format!(
                "<button type=\"button\" class=\"chip\" data-filter=\"{kind}\" aria-pressed=\"{}\">{label}</button>",
                *kind == "all"
            )
        })
        .collect();
    let mut speakers: Vec<&str> = ctx
        .lines
        .iter()
        .filter_map(|l| l.label.as_deref())
        .collect();
    speakers.sort_unstable();
    speakers.dedup();
    let speaker_filter = if speakers.len() > 1 {
        let options: String = speakers
            .iter()
            .map(|s| format!("<option value=\"{0}\">{0}</option>", esc(s)))
            .collect();
        format!(
            "<label class=\"who-filter\">Transcription : <select id=\"who\"><option value=\"\">tous les intervenants</option>{options}</select></label>"
        )
    } else {
        String::new()
    };

    format!(
        "<!doctype html>\n<html lang=\"fr\"{theme} data-mode=\"{mode}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'\">\
<meta name=\"generator\" content=\"VoiceToText\">\
<title>{title} · rapport</title><style>{CSS}</style></head><body>\
<div class=\"bar\" role=\"search\"><button type=\"button\" id=\"toc-toggle\" aria-expanded=\"false\" aria-controls=\"toc\" title=\"Sommaire\">☰</button>\
<input id=\"q\" type=\"search\" placeholder=\"Rechercher dans le rapport\" aria-label=\"Rechercher dans le rapport\" autocomplete=\"off\">\
<span id=\"count\" aria-live=\"polite\"></span>\
<button type=\"button\" id=\"prev\" aria-label=\"Résultat précédent\" disabled>↑</button>\
<button type=\"button\" id=\"next\" aria-label=\"Résultat suivant\" disabled>↓</button></div>\
<div class=\"layout\"><nav id=\"toc\" aria-label=\"Sommaire\"><p class=\"nav-title\">Sommaire</p><ol>{toc}</ol>\
<p class=\"nav-title\">Afficher</p><div class=\"filters\">{filters}</div>{speaker_filter}</nav>\
<main id=\"doc\"><p id=\"filter-note\" hidden><span></span> <button type=\"button\" class=\"chip\" id=\"filter-reset\">Tout afficher</button></p>{doc}{footer}</main></div><script>{JS}</script></body></html>\n",
        title = esc(&ctx.title),
    )
}

fn reply_html(line: &Line, options: &HtmlOptions) -> String {
    let reply = &line.reply;
    let color = reply
        .speaker
        .map(|s| format!(" class=\"s{}\"", s % 6))
        .unwrap_or_default();
    let who = line
        .label
        .as_deref()
        .map(|l| format!("<b{color}>{}</b> ", esc(l)))
        .unwrap_or_default();
    let play = if options.app && options.replay {
        format!(
            "<button type=\"button\" class=\"play\" data-ms=\"{}\" data-end=\"{}\" aria-label=\"Réécouter à {}\">▶</button>",
            reply.start_ms,
            line.end_ms.min(reply.start_ms + 60_000),
            timestamp(reply.start_ms)
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"reply\" data-ms=\"{}\" data-who=\"{}\"><div class=\"who\">{who}<a class=\"t\" href=\"#\" data-ms=\"{}\">{}</a>{play}</div><p>{}</p></div>",
        reply.start_ms,
        esc(line.label.as_deref().unwrap_or_default()),
        reply.start_ms,
        timestamp(reply.start_ms),
        esc(&reply.text)
    )
}

fn md_time(at_ms: Option<u64>) -> String {
    at_ms
        .map(|ms| format!(" [{}]", timestamp(ms)))
        .unwrap_or_default()
}

fn md_points(md: &mut String, points: &[Point]) {
    for p in points {
        md.push_str(&format!("- {}{}\n", p.text, md_time(p.at_ms)));
    }
    if !points.is_empty() {
        md.push('\n');
    }
}

fn render_markdown(report: &MeetingReport, ctx: &Context) -> String {
    let mut md = format!(
        "# {} — rapport\n\n{} · durée {}\n\n",
        ctx.title,
        ctx.date,
        timestamp(ctx.duration_ms)
    );
    if !ctx.people.is_empty() {
        let people: Vec<String> = ctx
            .people
            .iter()
            .map(|(name, talk, _)| format!("{name} ({})", timestamp(*talk)))
            .collect();
        md.push_str(&format!("Intervenants : {}\n\n", people.join(", ")));
    }
    md.push_str("## Sommaire\n\n- Synthèse\n");
    if !report.actions.is_empty() {
        md.push_str("- Plan d'action\n");
    }
    for (i, chapter) in report.chapters.iter().enumerate() {
        md.push_str(&format!(
            "- {}. {} [{}]\n",
            i + 1,
            chapter.title,
            timestamp(chapter.start_ms)
        ));
    }
    if report.has_analysis() {
        md.push_str("- Analyse\n");
    }
    md.push_str("\n## Synthèse\n\n");
    md.push_str(report.synthesis.trim());
    md.push_str("\n\n");
    if !report.key_points.is_empty() {
        md.push_str("### Points clés\n\n");
        md_points(&mut md, &report.key_points);
    }
    if !report.actions.is_empty() {
        md.push_str("## Plan d'action\n\n");
        for a in &report.actions {
            let mut facts = vec![if a.owner.is_empty() {
                "à définir".to_owned()
            } else {
                a.owner.clone()
            }];
            if !a.due.is_empty() {
                facts.push(format!("échéance : {}", a.due));
            }
            facts.push(format!("priorité {}", a.priority.label().to_lowercase()));
            md.push_str(&format!(
                "- [{}] {} — {}{}\n",
                if a.done { "x" } else { " " },
                a.task,
                facts.join(", "),
                md_time(a.at_ms)
            ));
        }
        md.push('\n');
    }
    for (i, chapter) in report.chapters.iter().enumerate() {
        md.push_str(&format!(
            "## {}. {} [{} – {}]\n\n",
            i + 1,
            chapter.title,
            timestamp(chapter.start_ms),
            timestamp(chapter.end_ms)
        ));
        if !chapter.summary.is_empty() {
            md.push_str(&format!("{}\n\n", chapter.summary.trim()));
        }
        md_points(&mut md, &chapter.points);
        if !chapter.decisions.is_empty() {
            md.push_str("**Décisions**\n\n");
            md_points(&mut md, &chapter.decisions);
        }
        for q in &chapter.quotes {
            let who = if q.who.is_empty() {
                String::new()
            } else {
                format!(" — {}", q.who)
            };
            md.push_str(&format!("> « {} »{who}{}\n\n", q.text, md_time(q.at_ms)));
        }
    }
    if report.has_analysis() {
        md.push_str("## Analyse\n\n");
        for (_, title, _, points) in report.analysis() {
            if !points.is_empty() {
                md.push_str(&format!("### {title}\n\n"));
                md_points(&mut md, points);
            }
        }
    }
    if !report.qa.is_empty() {
        md.push_str("## Questions et réponses\n\n");
        for qa in &report.qa {
            md.push_str(&format!(
                "**{}**{}\n\n{}\n\n",
                qa.question,
                md_time(qa.at_ms),
                qa.answer
            ));
        }
    }
    md.trim_end().to_owned() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::{MeetingKind, Segment};
    use serde_json::json;

    const HOUR: u64 = 3_600_000;

    #[test]
    fn times_are_read_and_checked_against_the_duration() {
        let d = HOUR;
        assert_eq!(parse_time(&json!("12:34"), d), Some(754_000));
        assert_eq!(parse_time(&json!("[05:00]"), d), Some(300_000));
        assert_eq!(parse_time(&json!("0:59:59"), d), Some(3_599_000));
        assert_eq!(parse_time(&json!("10:00–12:00"), d), Some(600_000));
        assert_eq!(parse_time(&json!(90), d), Some(90_000));
        // Au-delà de la fin, minutes impossibles, texte : écartées.
        assert_eq!(parse_time(&json!("1:30:00"), d), None);
        assert_eq!(parse_time(&json!("12:75"), d), None);
        assert_eq!(parse_time(&json!("vers la fin"), d), None);
        assert_eq!(parse_time(&json!(null), d), None);
        // Arrondi juste après la fin : ramené à la fin.
        assert_eq!(parse_time(&json!("60:02"), d), Some(d));
    }

    #[test]
    fn json_is_found_inside_a_code_block() {
        let value = parse_json("Voici le rapport :\n```json\n{\"synthese\": \"ok\"}\n```").unwrap();
        assert_eq!(value["synthese"], "ok");
        assert!(parse_json("pas de JSON").is_err());
        assert!(parse_json("{ cassé").is_err());
    }

    #[test]
    fn llm_answer_is_read_leniently() {
        let value = json!({
            "synthese": "Point sur le budget.",
            "points_cles": ["Budget validé", {"texte": "Lancement en mars", "t": "02:10"}, {"texte": ""}],
            "chapitres": [
                {"titre": "Budget", "debut": "00:00", "resume": "On parle budget.",
                 "decisions": [{"texte": "Budget à 40 k€", "t": "01:00"}],
                 "citations": [{"texte": "« On y va »", "qui": "Marie", "t": "01:05"}]},
                {"titre": "", "debut": "03:00", "points": "Un seul point"}
            ],
            "risques": [{"texte": "Retard fournisseur", "t": "99:00"}],
            "actions": [
                {"tache": "Envoyer le devis", "responsable": "Paul", "echeance": "vendredi", "priorite": "Élevée", "t": "02:30"},
                {"tache": "Relancer", "responsable": "À définir", "echeance": "non précisée"},
                {"tache": ""}
            ]
        });
        let mut report = MeetingReport::from_json(&value, 5 * 60_000);
        report.finish(5 * 60_000);
        assert_eq!(report.synthesis, "Point sur le budget.");
        assert_eq!(report.key_points.len(), 2);
        assert_eq!(report.key_points[1].at_ms, Some(130_000));
        assert_eq!(report.chapters.len(), 2);
        assert_eq!(report.chapters[0].end_ms, 180_000);
        assert_eq!(report.chapters[1].title, "Partie 2");
        assert_eq!(report.chapters[1].end_ms, 300_000);
        assert_eq!(report.chapters[1].points[0].text, "Un seul point");
        assert_eq!(report.chapters[0].quotes[0].text, "On y va");
        // Heure inexistante retirée, l'élément reste.
        assert_eq!(report.risks[0].at_ms, None);
        assert_eq!(report.actions.len(), 2);
        assert_eq!(report.actions[0].priority, Priority::Haute);
        assert_eq!(report.actions[1].owner, "");
        assert_eq!(report.actions[1].due, "");
    }

    #[test]
    fn chapters_cover_the_whole_recording() {
        let mut report = MeetingReport::from_json(
            &json!({"chapitres": [
                {"titre": "B", "debut": "10:00"},
                {"titre": "A", "debut": "02:00", "points": ["a"]},
                {"titre": "A bis", "debut": "02:00", "points": ["b"]}
            ]}),
            HOUR,
        );
        report.finish(HOUR);
        let spans: Vec<_> = report
            .chapters
            .iter()
            .map(|c| (c.title.as_str(), c.start_ms, c.end_ms))
            .collect();
        assert_eq!(spans, [("A", 0, 600_000), ("B", 600_000, HOUR)]);
        assert_eq!(report.chapters[0].points.len(), 2);

        let mut empty = MeetingReport::default();
        empty.finish(HOUR);
        assert_eq!(empty.chapters.len(), 1);
        assert_eq!(empty.chapters[0].end_ms, HOUR);
    }

    #[test]
    fn checked_actions_survive_a_new_report() {
        let mut old =
            MeetingReport::from_json(&json!({"actions": ["Envoyer le devis", "Relancer"]}), HOUR);
        old.actions[0].done = true;
        let mut new =
            MeetingReport::from_json(&json!({"actions": ["envoyer le devis", "Autre"]}), HOUR);
        new.keep_done(&old);
        assert!(new.actions[0].done);
        assert!(!new.actions[1].done);
    }

    fn record() -> MeetingRecord {
        let mut record = MeetingRecord::new(
            "id".into(),
            "Point <projet>".into(),
            0,
            MeetingKind::Reunion,
        );
        record.duration_ms = 10 * 60_000;
        let segment = |start_ms: u64, text: &str| Segment {
            track: Track::Moi,
            start_ms,
            end_ms: start_ms + 4_000,
            text: text.into(),
            copied: false,
            speaker: None,
        };
        record.segments = vec![
            segment(1_000, "Bonjour à tous."),
            segment(130_000, "Le budget <script>alert(1)</script> est validé."),
            segment(400_000, "On passe au planning."),
        ];
        let mut report = MeetingReport::from_json(
            &json!({
                "synthese": "Budget validé, planning à revoir.",
                "chapitres": [
                    {"titre": "Budget", "debut": "00:30"},
                    {"titre": "Planning", "debut": "06:00", "decisions": [{"texte": "Revoir le <planning>", "t": "06:40"}]}
                ],
                "actions": [{"tache": "Envoyer \"le\" devis", "responsable": "Paul", "t": "02:10"}]
            }),
            record.duration_ms,
        );
        report.finish(record.duration_ms);
        report.model = "deepseek".into();
        record.report = Some(report);
        record
    }

    #[test]
    fn html_is_self_contained_escaped_and_chaptered() {
        let record = record();
        let html = record
            .report_html("18/11/2026", &HtmlOptions::default())
            .unwrap();
        // Aucune ressource extérieure.
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("default-src 'none'"));
        // Texte de la transcription et du LLM échappé.
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("Point &lt;projet&gt;"));
        assert!(html.contains("Revoir le &lt;planning&gt;"));
        assert!(html.contains("Envoyer &quot;le&quot; devis"));
        // Chaque réplique dans son chapitre : la première (avant 00:30) va au premier.
        let ch1 = html.find("id=\"ch-1\"").unwrap();
        let ch2 = html.find("id=\"ch-2\"").unwrap();
        let hello = html.find("Bonjour à tous.").unwrap();
        let budget = html.find("Le budget").unwrap();
        let planning = html.find("On passe au planning.").unwrap();
        assert!(ch1 < hello && hello < budget && budget < ch2 && ch2 < planning);
        // Plan d'action, sommaire, filtres présents ; pas de ▶ hors application.
        assert!(html.contains("id=\"plan-action\""));
        assert!(html.contains("href=\"#ch-2\""));
        assert!(html.contains("data-filter=\"decision\""));
        assert!(!html.contains("data-filter=\"risk\""));
        assert!(!html.contains("class=\"play\""));
        assert!(html.contains("data-mode=\"file\""));
    }

    #[test]
    fn html_in_the_app_has_replay_and_theme() {
        let options = HtmlOptions {
            app: true,
            replay: true,
            theme: Some("dark".into()),
            tracks: false,
        };
        let html = record().report_html("18/11/2026", &options).unwrap();
        assert!(html.contains("<html lang=\"fr\" data-theme=\"dark\""));
        assert!(html.contains("data-mode=\"app\""));
        assert!(html.contains("class=\"play\" data-ms=\"130000\""));
        // Thème inconnu ignoré.
        let options = HtmlOptions {
            theme: Some("\"><script>".into()),
            ..HtmlOptions::default()
        };
        assert!(
            record()
                .report_html("", &options)
                .unwrap()
                .contains("<html lang=\"fr\" data-mode=")
        );
    }

    #[test]
    fn markdown_has_a_table_of_contents_and_the_plan() {
        let md = record().report_markdown("18/11/2026", false).unwrap();
        assert!(md.starts_with("# Point <projet> — rapport\n"));
        assert!(md.contains("- 1. Budget [00:00]\n- 2. Planning [06:00]\n"));
        assert!(md.contains("- [ ] Envoyer \"le\" devis — Paul, priorité normale [02:10]"));
        assert!(md.contains("## 2. Planning [06:00 – 10:00]"));
        assert!(md.contains("**Décisions**\n\n- Revoir le <planning> [06:40]"));
        // Sans la transcription.
        assert!(!md.contains("Bonjour à tous."));
    }

    #[test]
    fn no_report_no_document() {
        let record = MeetingRecord::new("id".into(), "t".into(), 0, MeetingKind::Reunion);
        assert!(record.report_html("", &HtmlOptions::default()).is_none());
        assert!(record.report_markdown("", false).is_none());
    }
}
