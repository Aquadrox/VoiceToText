// Pas de console en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_settings;
mod controller;
mod costs;
mod feedback;
mod history;
mod logging;
mod main_window;
mod model;
mod paths;
mod prompts;
mod raw_paste;
mod retry;
mod settings;
mod status;
mod tray;
mod updates;
mod vocabulary;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{Manager, PhysicalPosition, WebviewWindow};
use vtt_core::history::History;
use vtt_core::usage::Ledger;

use crate::controller::{Recorders, WorkerHandle};
use crate::costs::LedgerStore;
use crate::history::HistoryStore;
use crate::paths::Paths;
use crate::raw_paste::LastRaw;
use crate::settings::SharedConfig;
use crate::status::{Status, StatusReporter};
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

    let (config, config_error) = paths.load_config();

    tauri::Builder::default()
        // Une seconde instance installerait un second hook clavier : on l'interdit.
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            status::current_status,
            settings::settings_view,
            settings::save_api_key,
            settings::delete_api_key,
            settings::key_status,
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
            app_settings::app_settings_view,
            app_settings::save_app_settings,
            app_settings::set_autostart,
            updates::update_status,
            updates::install_update,
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
            raw_paste::register(app, &config.hotkey.paste_raw, Arc::clone(&last_raw));
            retry::register(app, &config.hotkey.retry, Arc::clone(&worker));
            let recorders = Recorders {
                history,
                ledger,
                last_raw,
            };
            let updates = Updates::new(app.handle(), tray_items.update);
            app.manage(Arc::clone(&updates));
            if config.updates.check {
                updates::spawn_checks(app.handle().clone(), updates);
            }
            controller::spawn(reporter, recorders, worker, config, paths);
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
    overlay.set_ignore_cursor_events(true)?;
    overlay.show()
}
