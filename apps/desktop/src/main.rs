// Pas de console en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod annotate;
mod app_settings;
mod controller;
mod costs;
mod decode;
mod dictation_report;
mod export;
mod feedback;
mod history;
mod logging;
mod main_window;
mod meetings;
mod model;
mod paths;
mod prompts;
mod raw_paste;
mod retry;
mod settings;
mod shortcuts;
mod speakers;
mod status;
mod translation;
mod tray;
mod updates;
mod vocabulary;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{Manager, PhysicalPosition, WebviewWindow};
use vtt_core::history::History;
use vtt_core::usage::Ledger;

use crate::annotate::Annotator;
use crate::controller::{Controller, Recorders, WorkerHandle};
use crate::costs::LedgerStore;
use crate::history::HistoryStore;
use crate::meetings::{MeetingQueue, Meetings};
use crate::paths::Paths;
use crate::raw_paste::LastRaw;
use crate::settings::SharedConfig;
use crate::shortcuts::Shortcuts;
use crate::status::{Status, StatusReporter};
use crate::translation::Translation;
use crate::updates::Updates;

/// Marge entre la pastille et le bas de la zone de travail (au-dessus de la barre des tâches).
const OVERLAY_MARGIN: f64 = 24.0;

fn main() {
    let paths = Paths::resolve();
    if let Err(e) = paths.create_dirs() {
        eprintln!("création des dossiers : {e}");
    }
    logging::init(&paths.logs_dir);
    log::info!("VoiceToText {} démarre", env!("CARGO_PKG_VERSION"));

    // Réglages déjà là : l'application a déjà servi (mise à jour ou réinstallation).
    let existing_install = paths.config_file.exists();
    let (config, config_error) = paths.load_config();

    tauri::Builder::default()
        // Une seconde instance installerait un second hook clavier : on l'interdit.
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            status::current_status,
            settings::settings_view,
            settings::save_api_key,
            settings::delete_api_key,
            settings::key_status,
            settings::api_keys,
            settings::replace_api_key,
            settings::forget_api_key,
            settings::test_llm_settings,
            settings::apply_llm_settings,
            settings::save_llm_profiles,
            prompts::prompts_view,
            prompts::save_prompts,
            prompts::test_prompt,
            history::history_view,
            history::clear_history,
            main_window::home_view,
            vocabulary::vocabulary_view,
            vocabulary::save_vocabulary,
            vocabulary::dictionary_suggestions,
            vocabulary::ignore_suggestion,
            translation::translation_state,
            app_settings::app_settings_view,
            app_settings::save_app_settings,
            app_settings::set_autostart,
            export::export_settings,
            export::import_settings,
            meetings::meetings_view,
            meetings::meeting_detail,
            meetings::meeting_state,
            meetings::offer_view,
            annotate::annotate_state,
            annotate::annotate_stroke,
            annotate::annotate_zone,
            meetings::offer_answer,
            meetings::start_meeting,
            meetings::stop_meeting,
            meetings::start_conference,
            meetings::import_recording,
            meetings::search_meetings,
            meetings::ask_meetings,
            meetings::meeting_clip,
            meetings::rename_meeting,
            meetings::summarize_meeting,
            meetings::delete_meeting,
            meetings::retranscribe_meeting,
            meetings::copy_meeting,
            meetings::export_meeting,
            meetings::export_meeting_audio,
            meetings::save_meeting_settings,
            meetings::identify_speakers,
            meetings::set_speaker_count,
            meetings::rename_speaker,
            meetings::forget_speakers,
            meetings::voices_view,
            meetings::download_voices,
            updates::update_status,
            updates::check_update,
            updates::install_update,
            updates::changelog_view,
            updates::dismiss_update_news,
        ])
        .setup(move |app| {
            let last_raw = LastRaw::default();
            let worker = WorkerHandle::default();
            app.manage(Arc::clone(&worker));
            let tray_items = tray::build(app, &paths, Arc::clone(&last_raw), Arc::clone(&worker))?;
            let status_item = tray_items.status;
            app.manage(paths.clone());
            let reporter = Arc::new(StatusReporter::new(app.handle().clone(), status_item));
            app.manage(Arc::clone(&reporter));
            let shared: SharedConfig = Arc::new(RwLock::new(config.clone()));
            app.manage(shared);
            let history: HistoryStore =
                Arc::new(Mutex::new(config.history.enabled.then(|| {
                    History::open(paths.history_file.clone(), config.history.max_entries)
                })));
            app.manage(Arc::clone(&history));
            let ledger: LedgerStore = Arc::new(Mutex::new(Ledger::open(paths.usage_file.clone())));
            app.manage(Arc::clone(&ledger));

            if let Some(overlay) = app.get_webview_window("overlay") {
                show_overlay(&overlay)?;
            }

            match config_error {
                Some(e) => reporter.set(Status::Error(e)),
                None => reporter.set(Status::Loading("Démarrage…".into())),
            }
            let translation = Translation::new(
                app.handle().clone(),
                tray_items.translate,
                &config.translation.language,
            );
            app.manage(Arc::clone(&translation));
            let voices = speakers::Voices::new(app.handle().clone(), paths.models_dir.clone());
            app.manage(Arc::clone(&voices));
            if config.meetings.diarize {
                // Activé sans modèles (réglages importés, téléchargement
                // interrompu) : téléchargés maintenant.
                voices.download();
            }
            let meeting_jobs = MeetingQueue::default();
            let meetings = Meetings::new(
                app.handle().clone(),
                &paths,
                Arc::clone(&meeting_jobs),
                Arc::clone(&worker),
                tray_items.meeting,
            );
            app.manage(Arc::clone(&meetings));
            let annotator = Annotator::new(
                app.handle().clone(),
                paths.captures_dir.clone(),
                Arc::clone(&worker),
            );
            app.manage(Arc::clone(&annotator));
            let shortcuts = Shortcuts::new(
                Arc::clone(&last_raw),
                Arc::clone(&worker),
                Arc::clone(&translation),
                Arc::clone(&meetings),
                annotator,
            );
            shortcuts.apply(app.handle(), &config.hotkey);
            app.manage(shortcuts);
            let recorders = Recorders {
                history,
                ledger,
                last_raw,
                meeting_jobs,
            };
            let updates = Updates::new(
                app.handle(),
                tray_items.update,
                config.updates.check,
                paths.version_file.clone(),
                existing_install,
            );
            app.manage(Arc::clone(&updates));
            updates::spawn_checks(app.handle().clone(), updates);
            let controller = Controller::new(reporter, recorders, worker, paths, translation);
            controller.start(config);
            app.manage(controller);
            // Réunions interrompues par un arrêt : transcription reprise.
            meetings.recover();
            meetings.spawn_detection();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("démarrage de Tauri");
}

/// Place la pastille en bas au centre de l'écran principal, laisse passer
/// les clics à travers, et ne la masque plus jamais : l'afficher et la cacher
/// à chaque dictée risquerait de voler le focus de l'application cible.
fn show_overlay(overlay: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = overlay.primary_monitor()? {
        let area = monitor.work_area();
        let size = overlay.outer_size()?;
        let margin = (OVERLAY_MARGIN * monitor.scale_factor()) as i32;
        let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
        overlay.set_position(PhysicalPosition::new(x, y))?;
    }
    #[cfg(not(target_os = "linux"))]
    overlay.set_ignore_cursor_events(true)?;
    // La pastille ne doit pas apparaître sur les captures commentées.
    #[cfg(windows)]
    if let Ok(hwnd) = overlay.hwnd() {
        vtt_platform::exclude_from_capture(hwnd.0 as isize);
    }
    overlay.show()?;
    // GTK : une fenêtre ne laisse passer les clics qu'une fois affichée
    // (avant, tao s'arrête net sur une fenêtre GDK absente).
    #[cfg(target_os = "linux")]
    overlay.set_ignore_cursor_events(true)?;
    Ok(())
}
