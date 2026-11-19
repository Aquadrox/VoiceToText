//! Activité par jour : dictées et consommation du LLM (jetons, coût estimé).

use std::sync::{Arc, Mutex};

use time::macros::format_description;
use time::{Duration, OffsetDateTime};
use vtt_core::config::Pricing;
use vtt_core::usage::{AppTotals, Ledger, Summary, Totals, Usage};

/// Partagé entre le thread du pipeline, le test des réglages et l'affichage.
pub type LedgerStore = Arc<Mutex<Ledger>>;

/// `app` : application de destination (`None` : test depuis les réglages).
pub fn record(store: &LedgerStore, app: Option<&str>, usage: &Usage, pricing: &Pricing) {
    let mut ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = ledger.record(&today(), app, usage, pricing) {
        // Jamais bloquant : seul le suivi du coût est perdu.
        log::warn!("suivi de la consommation : {e}");
    }
}

pub fn record_dictation(
    store: &LedgerStore,
    app: Option<&str>,
    words: u64,
    audio_ms: u64,
    command: bool,
) {
    let mut ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = ledger.record_dictation(&today(), app, words, audio_ms, command) {
        log::warn!("statistiques : {e}");
    }
}

/// Totaux par application : ce mois-ci, et depuis le début.
pub fn apps(store: &LedgerStore) -> (Vec<AppTotals>, Vec<AppTotals>) {
    let ledger = store.lock().unwrap_or_else(|e| e.into_inner());
    let today = today();
    let month = today.get(..7).unwrap_or(&today);
    (ledger.apps(Some(month)), ledger.apps(None))
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
pub fn today() -> String {
    format_day(local_now())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn temp_usage_file(name: &str) -> (PathBuf, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-costs-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("usage.json");
        (dir, file)
    }

    #[test]
    fn today_uses_iso_date_format() {
        let day = today();
        let bytes = day.as_bytes();
        assert_eq!(bytes.len(), 10);
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[7], b'-');
        assert!(day[..4].chars().all(|c| c.is_ascii_digit()));
        assert!(day[5..7].chars().all(|c| c.is_ascii_digit()));
        assert!(day[8..10].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn record_usage_and_dictation_update_summary_apps_and_persist() {
        let (dir, file) = temp_usage_file("record");
        let store: LedgerStore = Arc::new(Mutex::new(Ledger::open(file.clone())));
        let usage = Usage {
            input_tokens: 1_000,
            cached_input_tokens: 0,
            output_tokens: 500,
        };
        let pricing = Pricing {
            input_per_million: 1.0,
            cached_input_per_million: 0.0,
            output_per_million: 2.0,
            currency: "€".into(),
        };

        record(&store, Some("Slack.exe"), &usage, &pricing);
        record_dictation(&store, Some("Slack.exe"), 12, 4_200, false);
        record_dictation(&store, None, 5, 1_500, true);

        let sum = summary(&store);
        assert_eq!(sum.today.requests, 1);
        assert_eq!(sum.today.input_tokens, 1_000);
        assert_eq!(sum.today.output_tokens, 500);
        assert!((sum.today.cost - 0.002).abs() < 1e-9);
        assert_eq!(sum.today.dictations, 2);
        assert_eq!(sum.today.commands, 1);
        assert_eq!(sum.today.words, 17);
        assert_eq!(sum.today.audio_ms, 5_700);
        assert_eq!(sum.total.dictations, 2);

        let (month_apps, total_apps) = apps(&store);
        assert_eq!(month_apps.len(), 1);
        assert_eq!(total_apps.len(), 1);
        assert_eq!(month_apps[0].app, "slack.exe");
        assert_eq!(month_apps[0].totals.requests, 1);
        assert_eq!(month_apps[0].totals.dictations, 1);
        assert_eq!(month_apps[0].totals.words, 12);

        // Relit depuis le disque : les totaux sont conservés.
        let reloaded: LedgerStore = Arc::new(Mutex::new(Ledger::open(file)));
        let persisted = summary(&reloaded);
        assert_eq!(persisted.total.dictations, 2);
        assert_eq!(persisted.total.requests, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn last_days_returns_chronological_window_ending_today() {
        let (dir, file) = temp_usage_file("days");
        let store: LedgerStore = Arc::new(Mutex::new(Ledger::open(file)));
        record_dictation(&store, Some("code.exe"), 8, 2_000, false);

        assert!(last_days(&store, 0).is_empty());

        let days = last_days(&store, 7);
        assert_eq!(days.len(), 7);
        for pair in days.windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
        let (last_day, last_totals) = days.last().unwrap();
        assert_eq!(last_day, &today());
        assert_eq!(last_totals.dictations, 1);
        assert_eq!(last_totals.words, 8);
        assert!(days[..6].iter().all(|(_, totals)| totals.dictations == 0));

        let _ = std::fs::remove_dir_all(dir);
    }
}
