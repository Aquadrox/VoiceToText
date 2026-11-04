//! Fenêtre principale : accueil (statistiques), historique, réglages.
//!
//! L'application reste discrète : la fenêtre ne s'ouvre que depuis l'icône
//! de la zone de notification, jamais au démarrage.

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use vtt_core::usage::{AppTotals, Summary, Totals};

use crate::costs::{self, LedgerStore};
use crate::settings::{self, SharedConfig};

const LABEL: &str = "main";
/// Jours affichés dans le graphique d'activité.
const CHART_DAYS: i64 = 30;

/// Pages de la fenêtre (ancre de l'URL).
#[derive(Debug, Clone, Copy)]
pub enum Page {
    Home,
    History,
    Settings,
}

impl Page {
    fn anchor(self) -> &'static str {
        match self {
            Page::Home => "accueil",
            Page::History => "historique",
            Page::Settings => "reglages",
        }
    }
}

pub fn open(app: &AppHandle, page: Page) {
    if let Some(window) = app.get_webview_window(LABEL) {
        // Ancre issue d'une liste fixe : pas de texte extérieur dans le script.
        let _ = window.eval(format!("location.hash = '#{}'", page.anchor()));
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let url = WebviewUrl::App(format!("app.html#{}", page.anchor()).into());
    let built = WebviewWindowBuilder::new(app, LABEL, url)
        .title("VoiceToText")
        .inner_size(980.0, 720.0)
        .min_inner_size(720.0, 480.0)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("ouverture de la fenêtre principale : {e}");
    }
}

#[derive(Serialize)]
pub struct DayView {
    day: String,
    totals: Totals,
}

/// Activité par application (nom de l'exécutable seulement).
#[derive(Serialize)]
pub struct AppsView {
    month: Vec<AppTotals>,
    total: Vec<AppTotals>,
}

#[derive(Serialize)]
pub struct HomeView {
    summary: Summary,
    days: Vec<DayView>,
    apps: AppsView,
    currency: String,
}

#[tauri::command]
pub fn home_view(ledger: State<'_, LedgerStore>, config: State<'_, SharedConfig>) -> HomeView {
    HomeView {
        summary: costs::summary(&ledger),
        days: costs::last_days(&ledger, CHART_DAYS)
            .into_iter()
            .map(|(day, totals)| DayView { day, totals })
            .collect(),
        apps: {
            let (month, total) = costs::apps(&ledger);
            AppsView { month, total }
        },
        currency: settings::read(&config).llm.pricing.currency.clone(),
    }
}
