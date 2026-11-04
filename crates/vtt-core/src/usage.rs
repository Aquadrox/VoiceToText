//! Activité par jour : dictées (nombre, mots, durée parlée) et consommation
//! du LLM (jetons, coût), pour les statistiques et le suivi du coût.
//!
//! Le fichier des totaux ne contient que des compteurs, jamais de texte
//! dicté : il survit à l'effacement de l'historique. Un second fichier
//! (`usage-apps.json`) tient les mêmes compteurs par application et par
//! mois : nom de l'exécutable seulement, jamais le titre de la fenêtre.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Pricing;

/// Jetons facturés pour une requête.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Jetons envoyés, y compris ceux servis par le cache du fournisseur.
    pub input_tokens: u64,
    /// Part de `input_tokens` servie par le cache (facturée moins cher).
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

impl Pricing {
    pub fn cost(&self, usage: &Usage) -> f64 {
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let uncached = usage.input_tokens - cached;
        (uncached as f64 * self.input_per_million
            + cached as f64 * self.cached_input_per_million
            + usage.output_tokens as f64 * self.output_per_million)
            / 1_000_000.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Totals {
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    /// Dans la devise du tarif configuré au moment de chaque requête.
    pub cost: f64,
    /// Dictées insérées (commandes comprises).
    pub dictations: u64,
    /// Commandes (Command Mode) parmi les dictées.
    pub commands: u64,
    /// Mots prononcés (transcription brute).
    pub words: u64,
    /// Durée parlée.
    pub audio_ms: u64,
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost += other.cost;
        self.dictations += other.dictations;
        self.commands += other.commands;
        self.words += other.words;
        self.audio_ms += other.audio_ms;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Summary {
    pub today: Totals,
    pub month: Totals,
    pub total: Totals,
}

/// Totaux d'une application.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppTotals {
    /// Nom de l'exécutable en minuscules, ex. « slack.exe ».
    pub app: String,
    pub totals: Totals,
}

/// Application → mois (`AAAA-MM`) → totaux.
type AppMonths = BTreeMap<String, BTreeMap<String, Totals>>;

/// Totaux par jour (`AAAA-MM-JJ`), et par application et par mois, dans de
/// petits fichiers JSON.
pub struct Ledger {
    path: PathBuf,
    days: BTreeMap<String, Totals>,
    apps_path: PathBuf,
    apps: AppMonths,
}

/// Fichier JSON ; absent ou illisible : valeur par défaut (réécrite au
/// prochain enregistrement).
fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Écriture atomique : une coupure ne corrompt pas les totaux.
fn write_json<T: Serialize>(path: &PathBuf, value: &T) -> io::Result<()> {
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

impl Ledger {
    /// Un fichier absent ou illisible repart de zéro (et sera réécrit).
    /// Les totaux par application sont à côté : `usage-apps.json`.
    pub fn open(path: PathBuf) -> Self {
        let apps_path = path.with_file_name("usage-apps.json");
        Self {
            days: read_json(&path),
            apps: read_json(&apps_path),
            path,
            apps_path,
        }
    }

    /// `app` : application de destination, si elle est connue.
    pub fn record(
        &mut self,
        day: &str,
        app: Option<&str>,
        usage: &Usage,
        pricing: &Pricing,
    ) -> io::Result<()> {
        self.add(
            day,
            app,
            &Totals {
                requests: 1,
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                output_tokens: usage.output_tokens,
                cost: pricing.cost(usage),
                ..Totals::default()
            },
        )
    }

    pub fn record_dictation(
        &mut self,
        day: &str,
        app: Option<&str>,
        words: u64,
        audio_ms: u64,
        command: bool,
    ) -> io::Result<()> {
        self.add(
            day,
            app,
            &Totals {
                dictations: 1,
                commands: command.into(),
                words,
                audio_ms,
                ..Totals::default()
            },
        )
    }

    /// Totaux d'un jour (`AAAA-MM-JJ`), à zéro s'il n'y a rien eu.
    pub fn day(&self, day: &str) -> Totals {
        self.days.get(day).copied().unwrap_or_default()
    }

    fn add(&mut self, day: &str, app: Option<&str>, totals: &Totals) -> io::Result<()> {
        self.days.entry(day.to_owned()).or_default().add(totals);
        write_json(&self.path, &self.days)?;
        let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) else {
            return Ok(());
        };
        let month = day.get(..7).unwrap_or(day);
        self.apps
            .entry(app.to_lowercase())
            .or_default()
            .entry(month.to_owned())
            .or_default()
            .add(totals);
        write_json(&self.apps_path, &self.apps)
    }

    /// Totaux par application, du plus de mots au moins : pour le mois
    /// `AAAA-MM`, ou depuis le début (`None`). Les applications sans
    /// dictée sur la période sont omises.
    pub fn apps(&self, month: Option<&str>) -> Vec<AppTotals> {
        let mut found: Vec<AppTotals> = self
            .apps
            .iter()
            .map(|(app, months)| {
                let mut totals = Totals::default();
                for (_, t) in months.iter().filter(|(m, _)| month.is_none_or(|w| *m == w)) {
                    totals.add(t);
                }
                AppTotals {
                    app: app.clone(),
                    totals,
                }
            })
            .filter(|a| a.totals.dictations > 0)
            .collect();
        found.sort_by(|a, b| {
            b.totals
                .words
                .cmp(&a.totals.words)
                .then(b.totals.dictations.cmp(&a.totals.dictations))
                .then_with(|| a.app.cmp(&b.app))
        });
        found
    }

    /// `today` au format `AAAA-MM-JJ` ; le mois est son préfixe `AAAA-MM`.
    pub fn summary(&self, today: &str) -> Summary {
        let month = today.get(..7).unwrap_or(today);
        let mut summary = Summary::default();
        for (day, totals) in &self.days {
            summary.total.add(totals);
            if day.starts_with(month) {
                summary.month.add(totals);
            }
            if day == today {
                summary.today.add(totals);
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> Pricing {
        Pricing {
            input_per_million: 1.0,
            cached_input_per_million: 0.1,
            output_per_million: 2.0,
            currency: "USD".into(),
        }
    }

    fn usage(input: u64, cached: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: output,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtt-usage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("usage.json")
    }

    #[test]
    fn cost_bills_cached_tokens_at_cache_price() {
        let cost = pricing().cost(&usage(1_000_000, 400_000, 500_000));
        assert!((cost - (0.6 + 0.04 + 1.0)).abs() < 1e-9, "{cost}");
        // Un cache annoncé plus grand que l'entrée ne rend pas le coût négatif.
        assert!(pricing().cost(&usage(10, 50, 0)) >= 0.0);
    }

    #[test]
    fn summary_splits_today_month_and_total() {
        let path = temp_path("summary");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record("2026-10-31", None, &usage(100, 0, 10), &pricing())
            .unwrap();
        ledger
            .record("2026-11-02", None, &usage(200, 0, 20), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(300, 0, 30), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(300, 0, 30), &pricing())
            .unwrap();

        let summary = Ledger::open(path).summary("2026-11-03");
        assert_eq!(summary.today.requests, 2);
        assert_eq!(summary.today.input_tokens, 600);
        assert_eq!(summary.month.requests, 3);
        assert_eq!(summary.total.requests, 4);
        assert_eq!(summary.total.output_tokens, 90);
    }

    #[test]
    fn dictations_and_costs_share_daily_totals() {
        let path = temp_path("activity");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record_dictation("2026-11-03", None, 12, 4000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-03", None, 5, 2000, true)
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(100, 0, 10), &pricing())
            .unwrap();

        let day = Ledger::open(path).day("2026-11-03");
        assert_eq!((day.dictations, day.commands, day.words), (2, 1, 17));
        assert_eq!(day.audio_ms, 6000);
        assert_eq!(day.requests, 1);
        assert_eq!(
            Ledger::open(temp_path("empty")).day("2026-11-03"),
            Totals::default()
        );
    }

    #[test]
    fn apps_are_ranked_by_words_per_month_or_overall() {
        let path = temp_path("apps");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record_dictation("2026-10-30", Some("Slack.exe"), 50, 9000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-02", Some("code.exe"), 20, 4000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-03", Some("slack.exe"), 10, 2000, false)
            .unwrap();
        ledger
            .record(
                "2026-11-03",
                Some("slack.exe"),
                &usage(100, 0, 10),
                &pricing(),
            )
            .unwrap();
        // Application inconnue : comptée dans le total du jour seulement.
        ledger
            .record_dictation("2026-11-03", None, 7, 1000, false)
            .unwrap();

        let ledger = Ledger::open(path);
        let names = |apps: Vec<AppTotals>| -> Vec<(String, u64)> {
            apps.into_iter().map(|a| (a.app, a.totals.words)).collect()
        };
        assert_eq!(
            names(ledger.apps(Some("2026-11"))),
            [("code.exe".into(), 20), ("slack.exe".into(), 10)]
        );
        assert_eq!(
            names(ledger.apps(None)),
            [("slack.exe".into(), 60), ("code.exe".into(), 20)]
        );
        let slack = &ledger.apps(None)[0].totals;
        assert_eq!((slack.dictations, slack.requests), (2, 1));
        assert!(ledger.apps(Some("2026-09")).is_empty());
        assert_eq!(ledger.day("2026-11-03").words, 17);
    }

    #[test]
    fn older_file_without_activity_fields_still_loads() {
        let path = temp_path("v030");
        fs::write(
            &path,
            r#"{"2026-11-03":{"requests":2,"input_tokens":800,"cached_input_tokens":0,"output_tokens":20,"cost":0.0003}}"#,
        )
        .unwrap();
        let day = Ledger::open(path).day("2026-11-03");
        assert_eq!((day.requests, day.dictations), (2, 0));
    }

    #[test]
    fn unreadable_file_starts_from_zero() {
        let path = temp_path("corrupt");
        fs::write(&path, "{pas du json").unwrap();
        let mut ledger = Ledger::open(path.clone());
        assert_eq!(ledger.summary("2026-11-03"), Summary::default());
        ledger
            .record("2026-11-03", None, &usage(1, 0, 1), &pricing())
            .unwrap();
        assert_eq!(Ledger::open(path).summary("2026-11-03").total.requests, 1);
    }
}
