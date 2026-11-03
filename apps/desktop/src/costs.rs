//! Activité par jour : dictées et consommation du LLM (jetons, coût estimé).

use std::sync::{Arc, Mutex};

use time::macros::format_description;
use time::{Duration, OffsetDateTime};
use vtt_core::config::Pricing;
use vtt_core::usage::{Ledger, Summary, Totals, Usage};

/// Partagé entre le thread du pipeline, le test des réglages et l'affichage.
pub type LedgerStore = Arc<Mutex<Ledger>>;

pub fn record(store: &LedgerStore, usage: &Usage, pricing: &Pricing) {
    let mut ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = ledger.record(&today(), usage, pricing) {
        // Jamais bloquant : seul le suivi du coût est perdu.
        log::warn!("suivi de la consommation : {e}");
    }
}

pub fn record_dictation(store: &LedgerStore, words: u64, audio_ms: u64, command: bool) {
    let mut ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = ledger.record_dictation(&today(), words, audio_ms, command) {
        log::warn!("statistiques : {e}");
    }
}

/// Les `count` derniers jours, du plus ancien à aujourd'hui.
pub fn last_days(store: &LedgerStore, count: i64) -> Vec<(String, Totals)> {
    let ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    let today = local_now();
    (0..count)
        .rev()
        .map(|ago| {
            let day = format_day(today - Duration::days(ago));
            let totals = ledger.day(&day);
            (day, totals)
        })
        .collect()
}

pub fn summary(store: &LedgerStore) -> Summary {
    let ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    ledger.summary(&today())
}

/// Heure locale (UTC si le fuseau est introuvable).
fn local_now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

fn format_day(at: OffsetDateTime) -> String {
    at.format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

/// Date locale `AAAA-MM-JJ`.
fn today() -> String {
    format_day(local_now())
}
