//! Fenêtre active sous Wayland, fournie par KWin (KDE).
//!
//! Wayland ne dit pas aux applications quelle fenêtre est au premier plan.
//! KWin, lui, accepte des scripts : on lui en confie un petit (même méthode
//! que kdotool) qui, à chaque changement de fenêtre active, de titre ou de
//! plein écran, appelle VoiceToText par D-Bus avec la classe de la fenêtre,
//! son titre, son état plein écran et sa position. Sert aux styles par
//! application, à la mémoire graphique libérée en plein écran et à la
//! capture commentée (zone « fenêtre »).

use std::sync::{Mutex, Once};

use zbus::blocking::Connection;

use crate::portal;

const SERVICE: &str = "fr.aquadrox.voicetotext.ActiveWindow";
const OBJECT: &str = "/ActiveWindow";
/// Nom du script dans KWin (un seul à la fois : l'ancien est retiré).
const PLUGIN: &str = "voicetotext-active-window";

/// Script KWin (JavaScript, API de Plasma 6, repli Plasma 5). Les valeurs
/// passent en texte : la conversion des nombres JavaScript vers D-Bus varie
/// selon les versions de Qt.
const SCRIPT: &str = r#"// VoiceToText : envoie la fenêtre active à VoiceToText (D-Bus).
var watched = {};
function send(cls, caption, fullscreen, rect) {
    callDBus("fr.aquadrox.voicetotext.ActiveWindow", "/ActiveWindow",
        "fr.aquadrox.voicetotext.ActiveWindow", "Update", cls, caption, fullscreen, rect);
}
function current() {
    return workspace.activeWindow !== undefined ? workspace.activeWindow : workspace.activeClient;
}
function report(w) {
    if (!w) {
        send("", "", "false", "");
        return;
    }
    var g = w.frameGeometry;
    send(String(w.resourceClass), String(w.caption), w.fullScreen ? "true" : "false",
        [Math.round(g.x), Math.round(g.y), Math.round(g.width), Math.round(g.height)].join(" "));
}
function activated(w) {
    if (w) {
        var id = String(w.internalId);
        if (!watched[id]) {
            watched[id] = true;
            var again = function () {
                if (current() === w) {
                    report(w);
                }
            };
            w.captionChanged.connect(again);
            w.fullScreenChanged.connect(again);
        }
    }
    report(w);
}
(workspace.windowActivated || workspace.clientActivated).connect(activated);
activated(current());
"#;

/// Fenêtre active vue par KWin.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveWindow {
    /// Classe de la fenêtre (ex. « firefox », « org.kde.dolphin »).
    pub class: String,
    pub title: String,
    pub fullscreen: bool,
    /// Gauche, haut, droite, bas.
    pub rect: Option<(i32, i32, i32, i32)>,
}

static ACTIVE: Mutex<Option<ActiveWindow>> = Mutex::new(None);
/// Connexion qui reçoit les appels du script : gardée tant que le programme
/// vit.
static RECEIVER: Mutex<Option<Connection>> = Mutex::new(None);

struct Receiver;

#[zbus::interface(name = "fr.aquadrox.voicetotext.ActiveWindow")]
impl Receiver {
    /// Appelé par le script KWin à chaque changement.
    fn update(&self, class: String, title: String, fullscreen: String, rect: String) {
        let window = parse(&class, &title, &fullscreen, &rect);
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = window;
    }
}

fn parse(class: &str, title: &str, fullscreen: &str, rect: &str) -> Option<ActiveWindow> {
    if class.is_empty() && title.is_empty() {
        return None;
    }
    let numbers: Vec<i32> = rect
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let rect = match numbers[..] {
        [x, y, width, height] if width > 0 && height > 0 => Some((x, y, x + width, y + height)),
        _ => None,
    };
    Some(ActiveWindow {
        class: class.to_owned(),
        title: title.to_owned(),
        fullscreen: fullscreen == "true",
        rect,
    })
}

/// Lance le suivi (une fois) : service D-Bus, puis script dans KWin. Sans
/// KWin (GNOME, X11…), rien ne se passe : les autres méthodes restent.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| match load() {
            Ok(()) => log::info!("fenêtre active fournie par KWin (script chargé)"),
            Err(e) => log::warn!("fenêtre active par KWin indisponible : {e}"),
        });
    });
}

fn load() -> Result<(), String> {
    let receiver = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(SERVICE))
        .and_then(|b| b.serve_at(OBJECT, Receiver))
        .and_then(|b| b.build())
        .map_err(|e| format!("service D-Bus : {e}"))?;
    *RECEIVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(receiver);

    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join("voicetotext-kwin.js");
    std::fs::write(&path, SCRIPT).map_err(|e| format!("{} : {e}", path.display()))?;
    let path = path
        .to_str()
        .ok_or("chemin du script illisible")?
        .to_owned();

    let connection = portal::session()?;
    let scripting = portal::proxy(
        connection,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .map_err(|e| e.to_string())?;
    // Un script laissé par un lancement précédent est remplacé.
    let _ = scripting.call::<_, _, bool>("unloadScript", &(PLUGIN,));
    let id: i32 = scripting
        .call("loadScript", &(path.as_str(), PLUGIN))
        .map_err(|e| format!("KWin absent ou refus : {e}"))?;
    if id < 0 {
        return Err(format!("KWin a refusé le script (code {id})"));
    }
    // Plasma 6 : /Scripting/ScriptN ; Plasma 5 : /N.
    let mut last = String::new();
    for object in [format!("/Scripting/Script{id}"), format!("/{id}")] {
        match portal::proxy(connection, "org.kde.KWin", &object, "org.kde.kwin.Script")
            .and_then(|script| script.call_method("run", &()).map(|_| ()))
        {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("lancement du script : {last}"))
}

/// Dernière fenêtre active signalée par KWin (Wayland seulement).
pub fn active_window() -> Option<ActiveWindow> {
    if !crate::linux::wayland_session() {
        return None;
    }
    start();
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_are_parsed() {
        assert_eq!(
            parse(
                "firefox",
                "Boîte de réception - Gmail",
                "false",
                "10 20 800 600"
            ),
            Some(ActiveWindow {
                class: "firefox".into(),
                title: "Boîte de réception - Gmail".into(),
                fullscreen: false,
                rect: Some((10, 20, 810, 620)),
            })
        );
        let game = parse("steam_app_123", "Jeu", "true", "0 0 1280 800").unwrap();
        assert!(game.fullscreen);
        assert_eq!(parse("x", "", "false", "").unwrap().rect, None);
        assert_eq!(parse("x", "", "false", "1 2 0 5").unwrap().rect, None);
        assert_eq!(parse("", "", "false", ""), None);
    }
}
