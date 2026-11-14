//! Raccourcis déclarés au bureau sous Wayland (portail « GlobalShortcuts »).
//!
//! Wayland ne laisse pas une application écouter le clavier : sans le
//! réglage de KDE pour les anciennes applications X11, la touche de dictée
//! n'est pas vue hors de VoiceToText. Le portail, lui, laisse déclarer des
//! raccourcis au bureau ; KDE les confirme une fois (fenêtre de KDE, touche
//! modifiable ensuite dans Configuration du système → Raccourcis) et
//! signale chaque appui (« Activated ») et relâchement (« Deactivated ») :
//! ils sont transmis comme les touches réelles.
//!
//! Option (`hotkey.system_shortcut`) : désactivée par défaut, pour ne pas
//! changer une configuration qui marche avec le réglage de KDE.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, Once};
use std::thread;
use std::time::Duration;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::portal;
use crate::{KeyEvent, now_ms};

const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
/// La première déclaration attend que l'utilisateur confirme dans KDE.
const BIND_TIMEOUT: Duration = Duration::from_secs(120);
/// Identifiant d'application annoncé au portail (celui du fichier .desktop).
const APP_ID: &str = "fr.aquadrox.voicetotext";

const DICTATION: &str = "dictee";
const COMMAND: &str = "commande";

/// Touches de la configuration auxquelles correspondent les raccourcis :
/// (dictée, commande). Mises à jour à chaque changement de réglage.
static KEYS: Mutex<(u32, Option<u32>)> = Mutex::new((0, None));
/// Touches tenues d'après le portail (pour l'état « enfoncée »).
static HELD: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
/// Envoi vers le suivi du clavier (remplacé s'il est relancé).
static SENDER: Mutex<Option<Sender<KeyEvent>>> = Mutex::new(None);
/// Option activée : une fois déclarés, les raccourcis restent connus du
/// bureau jusqu'à la fin du programme ; décocher l'option les ignore.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Mémorise où envoyer les touches (appelé par le suivi du clavier).
pub fn set_sender(tx: Sender<KeyEvent>) {
    *SENDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
}

/// Touche tenue d'après le portail.
pub fn is_held(key: u32) -> bool {
    HELD.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|held| held.contains(&key))
}

/// Touches de dictée et de commande actuelles ; la première fois avec
/// `declare`, déclare les raccourcis au bureau (Wayland seulement).
pub fn set_global_keys(
    enabled: bool,
    trigger: u32,
    trigger_name: &str,
    command: Option<u32>,
    command_name: &str,
) {
    *KEYS.lock().unwrap_or_else(|e| e.into_inner()) = (trigger, command);
    ENABLED.store(enabled, Ordering::SeqCst);
    if !enabled || !crate::linux::wayland_session() {
        return;
    }
    static STARTED: Once = Once::new();
    let names = (
        preferred_trigger(trigger_name),
        preferred_trigger(command_name),
    );
    STARTED.call_once(move || {
        thread::Builder::new()
            .name("global-shortcuts".into())
            .spawn(move || {
                if let Err(e) = run(names.0, names.1) {
                    log::warn!("raccourcis déclarés au bureau indisponibles : {e}");
                }
            })
            .ok();
    });
}

/// Touche de la configuration → nom XDG proposé à KDE (« F13 », « Pause »…).
/// AltGr, Ctrl ou Maj seuls ne peuvent pas être des raccourcis du bureau :
/// rien n'est proposé, l'utilisateur choisit dans la fenêtre de KDE.
fn preferred_trigger(name: &str) -> Option<String> {
    let name = name.trim();
    let lower = name.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f')
        && let Ok(n) = n.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{n}"));
    }
    let xdg = match lower.as_str() {
        "insert" => "Insert",
        "pause" => "Pause",
        "scrolllock" => "Scroll_Lock",
        "capslock" => "Caps_Lock",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Prior",
        "pagedown" => "Next",
        "numlock" => "Num_Lock",
        "menu" | "apps" => "Menu",
        _ => return None,
    };
    Some(xdg.to_owned())
}

fn shortcut(description: &str, preferred: Option<String>) -> HashMap<&'static str, Value<'static>> {
    let mut options = HashMap::from([("description", portal::text(description))]);
    if let Some(preferred) = preferred {
        options.insert("preferred_trigger", Value::from(preferred));
    }
    options
}

fn run(dictation: Option<String>, command: Option<String>) -> Result<(), String> {
    let connection = portal::session()?;
    // Identité de l'application (xdg-desktop-portal 1.19 et plus) ; sans
    // effet ailleurs.
    if let Ok(registry) = portal::proxy(
        connection,
        portal::DESTINATION,
        portal::PATH,
        "org.freedesktop.host.portal.Registry",
    ) {
        let options: HashMap<&str, Value> = HashMap::new();
        let _ = registry.call_method("Register", &(APP_ID, options));
    }
    let shortcuts = portal::proxy(connection, portal::DESTINATION, portal::PATH, INTERFACE)
        .map_err(|e| e.to_string())?;

    // Abonnements avant toute demande : aucun appui perdu.
    let activated = shortcuts
        .receive_signal("Activated")
        .map_err(|e| e.to_string())?;
    let deactivated = shortcuts
        .receive_signal("Deactivated")
        .map_err(|e| e.to_string())?;

    let session_token = portal::token();
    let created = portal::request(connection, Duration::from_secs(10), |token| {
        let options = HashMap::from([
            ("handle_token", portal::text(token)),
            ("session_handle_token", portal::text(&session_token)),
        ]);
        shortcuts.call_method("CreateSession", &(options,))?;
        Ok(())
    })?;
    let session = session_handle(created.get("session_handle"))
        .ok_or("session de raccourcis sans identifiant")?;

    let list = vec![
        (
            DICTATION,
            shortcut("VoiceToText : dicter (maintenir)", dictation),
        ),
        (
            COMMAND,
            shortcut("VoiceToText : Command Mode (avec la dictée)", command),
        ),
    ];
    let bound = portal::request(connection, BIND_TIMEOUT, |token| {
        let options = HashMap::from([("handle_token", portal::text(token))]);
        shortcuts.call_method("BindShortcuts", &(&session, list, "", options))?;
        Ok(())
    })?;
    log::info!(
        "raccourcis déclarés au bureau : {}",
        describe(bound.get("shortcuts")).unwrap_or_else(|| "sans touche".into())
    );

    let session_for_up = session.clone();
    thread::spawn(move || {
        for message in deactivated {
            if let Some(id) = shortcut_id(&message, &session_for_up) {
                forward(&id, false);
            }
        }
    });
    for message in activated {
        if let Some(id) = shortcut_id(&message, &session) {
            forward(&id, true);
        }
    }
    Err("signaux du portail interrompus".into())
}

fn session_handle(value: Option<&OwnedValue>) -> Option<OwnedObjectPath> {
    let value = value?;
    if let Ok(path) = OwnedObjectPath::try_from(value.try_clone().ok()?) {
        return Some(path);
    }
    let text = String::try_from(value.try_clone().ok()?).ok()?;
    ObjectPath::try_from(text).ok().map(OwnedObjectPath::from)
}

/// Raccourcis confirmés et leur touche, pour le journal.
fn describe(value: Option<&OwnedValue>) -> Option<String> {
    let list: Vec<(String, HashMap<String, OwnedValue>)> =
        Vec::try_from(value?.try_clone().ok()?).ok()?;
    Some(
        list.iter()
            .map(|(id, props)| {
                let trigger = props
                    .get("trigger_description")
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_else(|| "aucune touche".into());
                format!("{id} = {trigger}")
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Identifiant du raccourci d'un signal de notre session.
fn shortcut_id(message: &zbus::message::Message, session: &OwnedObjectPath) -> Option<String> {
    let (handle, id, _, _): (OwnedObjectPath, String, u64, HashMap<String, OwnedValue>) =
        message.body().deserialize().ok()?;
    (handle == *session).then_some(id)
}

fn forward(id: &str, down: bool) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let (trigger, command) = *KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let key = match id {
        DICTATION => trigger,
        COMMAND => match command {
            Some(key) => key,
            None => return,
        },
        _ => return,
    };
    {
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let held = held.get_or_insert_with(HashSet::new);
        if down {
            held.insert(key);
        } else {
            held.remove(&key);
        }
    }
    if let Some(sender) = SENDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = sender.send(KeyEvent {
            key,
            down,
            at_ms: now_ms(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_triggers() {
        assert_eq!(preferred_trigger("F13").as_deref(), Some("F13"));
        assert_eq!(preferred_trigger("f9").as_deref(), Some("F9"));
        assert_eq!(preferred_trigger("PageUp").as_deref(), Some("Prior"));
        assert_eq!(
            preferred_trigger("ScrollLock").as_deref(),
            Some("Scroll_Lock")
        );
        assert_eq!(preferred_trigger("AltGr"), None);
        assert_eq!(preferred_trigger("#183"), None);
        assert_eq!(preferred_trigger(""), None);
        assert_eq!(preferred_trigger("F25"), None);
    }
}
