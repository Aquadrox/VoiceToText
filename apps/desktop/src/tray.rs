//! Icône de la zone de notification et son menu.

use std::path::Path;
use std::process::Command;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Wry};

use crate::paths::Paths;

/// Construit l'icône ; renvoie la ligne d'état du menu pour la mettre à jour.
pub fn build(app: &App, paths: &Paths) -> tauri::Result<MenuItem<Wry>> {
    let status = MenuItem::with_id(app, "status", "Démarrage…", false, None::<&str>)?;
    let config = MenuItem::with_id(
        app,
        "config",
        "Modifier la configuration",
        true,
        None::<&str>,
    )?;
    let logs = MenuItem::with_id(app, "logs", "Ouvrir les journaux", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "Redémarrer", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &config,
            &logs,
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
            "config" => open_in_editor(&config_file),
            "logs" => open(&logs_dir),
            "restart" => app.restart(),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(status)
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
