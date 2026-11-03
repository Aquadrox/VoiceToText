//! Activité par jour : dictées (nombre, mots, durée parlée) et consommation
//! du LLM (jetons, coût), pour les statistiques et le suivi du coût.
//!
//! Le fichier des totaux ne contient que des compteurs, jamais de texte
//! dicté : il survit à l'effacement de l'historique.

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

/// Totaux par jour (`AAAA-MM-JJ`), dans un petit fichier JSON.
pub struct Ledger {
    path: PathBuf,
    days: BTreeMap<String, Totals>,
}

impl Ledger {
    /// Un fichier absent ou illisible repart de zéro (et sera réécrit).
    pub fn open(path: PathBuf) -> Self {
        let days = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, days }
    }

    pub fn record(&mut self, day: &str, usage: &Usage, pricing: &Pricing) -> io::Result<()> {
        self.add(
            day,
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
        words: u64,
        audio_ms: u64,
        command: bool,
    ) -> io::Result<()> {
        self.add(
            day,
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

    fn add(&mut self, day: &str, totals: &Totals) -> io::Result<()> {
        self.days.entry(day.to_owned()).or_default().add(totals);
        let json = serde_json::to_string_pretty(&self.days).map_err(io::Error::other)?;
        // Écriture atomique : une coupure ne corrompt pas les totaux.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)
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
            .record("2026-10-31", &usage(100, 0, 10), &pricing())
            .unwrap();
        ledger
            .record("2026-11-02", &usage(200, 0, 20), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", &usage(300, 0, 30), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", &usage(300, 0, 30), &pricing())
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
            .record_dictation("2026-11-03", 12, 4000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-03", 5, 2000, true)
            .unwrap();
        ledger
            .record("2026-11-03", &usage(100, 0, 10), &pricing())
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
            .record("2026-11-03", &usage(1, 0, 1), &pricing())
            .unwrap();
        assert_eq!(Ledger::open(path).summary("2026-11-03").total.requests, 1);
    }
}
