//! Icône de la zone de notification et son menu.

use std::path::Path;
use std::process::Command;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Wry};

use crate::controller::WorkerHandle;
use crate::main_window::{self, Page};
use crate::paths::Paths;
use crate::raw_paste::{self, LastRaw};
use crate::retry::{self, Delivery};

/// Construit l'icône ; renvoie la ligne d'état du menu pour la mettre à jour.
/// Entrées du menu mises à jour en cours de route.
pub struct TrayItems {
    pub status: MenuItem<Wry>,
    pub update: MenuItem<Wry>,
}

pub fn build(
    app: &App,
    paths: &Paths,
    last_raw: LastRaw,
    worker: WorkerHandle,
) -> tauri::Result<TrayItems> {
    let status = MenuItem::with_id(app, "status", "Démarrage…", false, None::<&str>)?;
    let open_main = MenuItem::with_id(app, "open", "Ouvrir VoiceToText", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "Historique…", true, None::<&str>)?;
    let copy_raw = MenuItem::with_id(
        app,
        "copy_raw",
        "Copier la dernière dictée brute",
        true,
        None::<&str>,
    )?;
    let retry = MenuItem::with_id(
        app,
        "retry",
        "Réessayer la dernière dictée (copie)",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Réglages…", true, None::<&str>)?;
    let config = MenuItem::with_id(
        app,
        "config",
        "Modifier la configuration",
        true,
        None::<&str>,
    )?;
    let logs = MenuItem::with_id(app, "logs", "Ouvrir les journaux", true, None::<&str>)?;
    let update = MenuItem::with_id(
        app,
        "update",
        "Rechercher une mise à jour",
        true,
        None::<&str>,
    )?;
    let restart = MenuItem::with_id(app, "restart", "Redémarrer", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open_main,
            &history,
            &copy_raw,
            &retry,
            &settings,
            &config,
            &logs,
            &update,
            &restart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let config_file = paths.config_file.clone();
    let logs_dir = paths.logs_dir.clone();
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("VoiceToText")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => main_window::open(app, Page::Home),
            "history" => main_window::open(app, Page::History),
            "copy_raw" => raw_paste::copy(&last_raw),
            "retry" => retry::send(&worker, Delivery::Clipboard),
            "settings" => main_window::open(app, Page::Settings),
            "config" => open_in_editor(&config_file),
            "logs" => open(&logs_dir),
            "update" => crate::updates::on_tray_click(app),
            "restart" => app.restart(),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(TrayItems { status, update })
}

fn open_in_editor(file: &Path) {
    let program = if cfg!(windows) { "notepad" } else { "xdg-open" };
    spawn(program, file);
}

fn open(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    spawn(program, path);
}

fn spawn(program: &str, path: &Path) {
    if let Err(e) = Command::new(program).arg(path).spawn() {
        log::error!("ouverture de {} : {e}", path.display());
    }
}
