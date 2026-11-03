//! Totaux de consommation du LLM (jetons et coût estimé), par jour.

use std::sync::{Arc, Mutex};

use time::OffsetDateTime;
use time::macros::format_description;
use vtt_core::config::Pricing;
use vtt_core::usage::{Ledger, Summary, Usage};

/// Partagé entre le thread du pipeline, le test des réglages et l'affichage.
pub type LedgerStore = Arc<Mutex<Ledger>>;

pub fn record(store: &LedgerStore, usage: &Usage, pricing: &Pricing) {
    let mut ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = ledger.record(&today(), usage, pricing) {
        // Jamais bloquant : seul le suivi du coût est perdu.
        log::warn!("suivi de la consommation : {e}");
    }
}

pub fn summary(store: &LedgerStore) -> Summary {
    let ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    ledger.summary(&today())
}

/// Date locale `AAAA-MM-JJ` (UTC si le fuseau est introuvable).
fn today() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}
