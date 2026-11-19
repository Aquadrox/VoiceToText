//! Historique des dictées : enregistrement après chaque dictée, lecture et
//! effacement depuis la fenêtre principale.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::State;
use vtt_core::config::{HistoryConfig, Pricing};
use vtt_core::history::{Entry, History};
use vtt_core::{AudioClip, Outcome, Target};

/// Nombre de dictées affichées dans la fenêtre.
const SHOWN: usize = 500;

/// Partagé entre le thread du pipeline (écriture) et la fenêtre (lecture).
/// `None` si l'historique est désactivé dans config.toml.
pub type HistoryStore = Arc<Mutex<Option<History>>>;

/// Active, désactive ou change la taille de l'historique sans redémarrer.
/// Désactiver ne supprime pas le fichier (comme au démarrage).
pub fn reconfigure(store: &HistoryStore, file: &Path, old: &HistoryConfig, new: &HistoryConfig) {
    if old == new {
        return;
    }
    *store.lock().unwrap_or_else(|e| e.into_inner()) = new
        .enabled
        .then(|| History::open(file.to_path_buf(), new.max_entries));
    log::info!(
        "historique : {}",
        if new.enabled {
            "activé"
        } else {
            "désactivé"
        }
    );
}

pub fn record(
    store: &HistoryStore,
    clip: &AudioClip,
    target: &Target,
    outcome: &Outcome,
    pricing: &Pricing,
) {
    let Outcome::Inserted {
        raw,
        text,
        post_process_error,
        snippet,
        command,
        usage,
        fallback,
        translated,
    } = outcome
    else {
        return;
    };
    let entry = Entry {
        at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
        duration_ms: clip.duration_ms(),
        raw: raw.clone(),
        text: text.clone(),
        snippet: *snippet,
        cleanup_failed: post_process_error.is_some(),
        command: *command,
        // Exécutable seulement : le titre de la fenêtre peut être personnel.
        app: target.app.clone(),
        usage: *usage,
        cost: usage.map(|u| pricing.cost(&u)),
        currency: usage.map(|_| pricing.currency.clone()),
        fallback: fallback.clone(),
        translated: translated.clone(),
    };
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(history) = guard.as_mut()
        && let Err(e) = history.append(&entry)
    {
        // Jamais bloquant : la dictée a déjà été insérée.
        log::warn!("historique : {e}");
    }
}

#[derive(serde::Serialize)]
pub struct HistoryView {
    enabled: bool,
    entries: Vec<Entry>,
}

fn read_history_view(store: &HistoryStore) -> Result<HistoryView, String> {
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        None => Ok(HistoryView {
            enabled: false,
            entries: Vec::new(),
        }),
        Some(history) => Ok(HistoryView {
            enabled: true,
            entries: history.recent(SHOWN).map_err(|e| e.to_string())?,
        }),
    }
}

fn clear_history_store(store: &HistoryStore) -> Result<(), String> {
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(history) = guard.as_mut() {
        history.clear().map_err(|e| e.to_string())?;
        log::info!("historique effacé");
    }
    Ok(())
}

#[tauri::command]
pub fn history_view(store: State<'_, HistoryStore>) -> Result<HistoryView, String> {
    read_history_view(&store)
}

#[tauri::command]
pub fn clear_history(store: State<'_, HistoryStore>) -> Result<(), String> {
    clear_history_store(&store)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use vtt_core::usage::Usage;

    use super::*;

    fn temp_history_file(name: &str) -> (PathBuf, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-history-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history.jsonl");
        (dir, file)
    }

    fn sample_inserted() -> Outcome {
        Outcome::Inserted {
            raw: "bonjour tout le monde".into(),
            text: "Bonjour tout le monde.".into(),
            post_process_error: Some("timeout".into()),
            snippet: false,
            usage: Some(Usage {
                input_tokens: 1_000,
                cached_input_tokens: 0,
                output_tokens: 500,
            }),
            command: true,
            fallback: Some("Secours".into()),
            translated: Some("anglais".into()),
        }
    }

    #[test]
    fn record_ignores_non_inserted_and_persists_inserted_without_window_title() {
        let (dir, file) = temp_history_file("record");
        let store: HistoryStore = Arc::new(Mutex::new(Some(History::open(file.clone(), 50))));
        let clip = AudioClip {
            samples: vec![0.0; 16_000],
        };
        let target = Target {
            app: Some("slack.exe".into()),
            title: Some("Conversation confidentielle".into()),
            ..Target::default()
        };
        let pricing = Pricing {
            input_per_million: 1.0,
            cached_input_per_million: 0.0,
            output_per_million: 2.0,
            currency: "€".into(),
        };

        record(&store, &clip, &target, &Outcome::Skipped, &pricing);
        record(&store, &clip, &target, &Outcome::Undone, &pricing);
        let empty = read_history_view(&store).unwrap();
        assert!(empty.enabled);
        assert!(empty.entries.is_empty());

        record(&store, &clip, &target, &sample_inserted(), &pricing);
        let view = read_history_view(&store).unwrap();
        assert!(view.enabled);
        assert_eq!(view.entries.len(), 1);
        let entry = &view.entries[0];
        assert_eq!(entry.duration_ms, 1000);
        assert_eq!(entry.raw, "bonjour tout le monde");
        assert_eq!(entry.text, "Bonjour tout le monde.");
        assert!(entry.cleanup_failed);
        assert!(entry.command);
        assert!(!entry.snippet);
        assert_eq!(entry.app.as_deref(), Some("slack.exe"));
        assert_eq!(entry.currency.as_deref(), Some("€"));
        assert!((entry.cost.unwrap() - 0.002).abs() < 1e-9);
        assert_eq!(entry.fallback.as_deref(), Some("Secours"));
        assert_eq!(entry.translated.as_deref(), Some("anglais"));

        let on_disk = std::fs::read_to_string(&file).unwrap();
        assert!(!on_disk.contains("Conversation confidentielle"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reconfigure_toggles_and_preserves_file_on_disk() {
        let (dir, file) = temp_history_file("reconfigure");
        let enabled_cfg = HistoryConfig {
            enabled: true,
            max_entries: 10,
        };
        let disabled_cfg = HistoryConfig {
            enabled: false,
            max_entries: 10,
        };
        let store: HistoryStore = Arc::new(Mutex::new(Some(History::open(file.clone(), 10))));
        let clip = AudioClip {
            samples: vec![0.0; 8_000],
        };
        record(
            &store,
            &clip,
            &Target::default(),
            &sample_inserted(),
            &Pricing::default(),
        );

        // Identique : aucun changement.
        reconfigure(&store, &file, &enabled_cfg, &enabled_cfg);
        assert_eq!(read_history_view(&store).unwrap().entries.len(), 1);

        // Désactivation : la vue est vide mais le fichier reste sur disque.
        reconfigure(&store, &file, &enabled_cfg, &disabled_cfg);
        let disabled_view = read_history_view(&store).unwrap();
        assert!(!disabled_view.enabled);
        assert!(disabled_view.entries.is_empty());
        assert!(file.exists());

        // Réactivation : l'entrée existante réapparaît.
        reconfigure(&store, &file, &disabled_cfg, &enabled_cfg);
        let reenabled_view = read_history_view(&store).unwrap();
        assert!(reenabled_view.enabled);
        assert_eq!(reenabled_view.entries.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_history_store_empties_entries_and_is_noop_when_disabled() {
        let (dir, file) = temp_history_file("clear");
        let disabled: HistoryStore = Arc::new(Mutex::new(None));
        clear_history_store(&disabled).unwrap();
        assert!(!read_history_view(&disabled).unwrap().enabled);

        let store: HistoryStore = Arc::new(Mutex::new(Some(History::open(file, 10))));
        let clip = AudioClip {
            samples: vec![0.0; 16_000],
        };
        record(
            &store,
            &clip,
            &Target::default(),
            &sample_inserted(),
            &Pricing::default(),
        );
        assert_eq!(read_history_view(&store).unwrap().entries.len(), 1);

        clear_history_store(&store).unwrap();
        let cleared = read_history_view(&store).unwrap();
        assert!(cleared.enabled);
        assert!(cleared.entries.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }
}
