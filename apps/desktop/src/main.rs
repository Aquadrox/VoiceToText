// Pas de console en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod history;
mod logging;
mod model;
mod paths;
mod settings;
mod status;
mod tray;

use std::sync::{Arc, Mutex};

use tauri::{Manager, PhysicalPosition, WebviewWindow};
use vtt_core::Config;
use vtt_core::history::History;

use crate::history::HistoryStore;
use crate::paths::Paths;
use crate::status::{Status, StatusReporter};

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
        .invoke_handler(tauri::generate_handler![
            status::current_status,
            settings::settings_view,
            settings::save_api_key,
            settings::delete_api_key,
            settings::test_cleanup,
            history::history_view,
            history::clear_history,
        ])
        .setup(move |app| {
            let status_item = tray::build(app, &paths)?;
            let reporter = Arc::new(StatusReporter::new(app.handle().clone(), status_item));
            app.manage(Arc::clone(&reporter));
            let config = Arc::new(config);
            app.manage(Arc::clone(&config));
            let history: HistoryStore =
                Arc::new(Mutex::new(config.history.enabled.then(|| {
                    History::open(paths.history_file.clone(), config.history.max_entries)
                })));
            app.manage(Arc::clone(&history));

            if let Some(overlay) = app.get_webview_window("overlay") {
                show_overlay(&overlay)?;
            }

            match config_error {
                Some(e) => reporter.set(Status::Error(e)),
                None => reporter.set(Status::Loading("Démarrage…".into())),
            }
            controller::spawn(reporter, history, Config::clone(&config), paths);
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
