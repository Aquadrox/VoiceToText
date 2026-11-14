//! Intégration Linux, session X11 (mode Bureau du Steam Deck, KDE Plasma,
//! GNOME ou XFCE sous X11) : touches globales, frappes simulées, presse-papier
//! et fenêtre active.
//!
//! Équivalences avec Windows :
//! - hook clavier bas niveau → événements « bruts » d'XInput2, reçus quelle
//!   que soit la fenêtre active, sans rien intercepter ni droit particulier ;
//! - `SendInput` → extension XTest ;
//! - numéro de séquence du presse-papier → notification XFixes du
//!   changement de propriétaire de la sélection CLIPBOARD.
//!
//! Les codes de touche de la plateforme sont ceux du noyau Linux (evdev,
//! `KEY_*` de `linux/input-event-codes.h`) : X11 les décale de 8.
//!
//! Session Wayland (mode Bureau de SteamOS 3.8 et plus, Plasma 6) : tout
//! passe par XWayland, sauf les frappes simulées, envoyées par un clavier
//! virtuel du noyau (uinput) pour atteindre aussi les applications Wayland.
//! Wayland n'autorise pas l'écoute globale du clavier : XWayland ne reçoit les
//! touches tapées ailleurs que si KDE l'y autorise (Configuration du système →
//! prise en charge des applications X11 historiques → « Toujours »).

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use vtt_core::{Error, Target, TextSink};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, Keycode, Keysym, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::{KeyEvent, now_ms};

/// Décalage entre un code evdev et un keycode X11.
const X_KEYCODE_OFFSET: u32 = 8;

/// Délai avant de remettre l'ancien contenu du presse-papier : l'application
/// cible lit le presse-papier de façon asynchrone après le Ctrl+V.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(600);

/// Codes evdev utilisés ici.
mod key {
    pub const ESC: u32 = 1;
    pub const LEFTCTRL: u32 = 29;
    pub const LEFTSHIFT: u32 = 42;
    pub const RIGHTSHIFT: u32 = 54;
    pub const LEFTALT: u32 = 56;
    pub const CAPSLOCK: u32 = 58;
    pub const F1: u32 = 59;
    pub const F11: u32 = 87;
    pub const F12: u32 = 88;
    pub const SCROLLLOCK: u32 = 70;
    pub const RIGHTCTRL: u32 = 97;
    pub const RIGHTALT: u32 = 100;
    pub const INSERT: u32 = 110;
    pub const NUMLOCK: u32 = 69;
    pub const HOME: u32 = 102;
    pub const PAGEUP: u32 = 104;
    pub const END: u32 = 107;
    pub const PAGEDOWN: u32 = 109;
    pub const PAUSE: u32 = 119;
    pub const LEFTMETA: u32 = 125;
    pub const RIGHTMETA: u32 = 126;
    pub const COMPOSE: u32 = 127;
    pub const F13: u32 = 183;
    /// Positions QWERTY de C, V et Z (repli si la disposition est illisible).
    pub const C: u32 = 46;
    pub const V: u32 = 47;
    pub const Z: u32 = 44;
}

/// Keysyms X11 (`X11/keysymdef.h`).
mod keysym {
    pub const C: u32 = 0x63;
    pub const V: u32 = 0x76;
    pub const Z: u32 = 0x7a;
    pub const CONTROL_L: u32 = 0xffe3;
}

/// Nom de touche de la configuration → code evdev.
pub fn key_code(name: &str) -> Option<u32> {
    if let Some(code) = vtt_core::config::raw_key_code(name) {
        return Some(code);
    }
    let name = name.trim().to_ascii_lowercase();
    let code = match name.as_str() {
        "altgr" | "rightalt" | "ralt" => key::RIGHTALT,
        "rightctrl" | "rctrl" => key::RIGHTCTRL,
        "rightshift" | "rshift" => key::RIGHTSHIFT,
        "capslock" => key::CAPSLOCK,
        "scrolllock" => key::SCROLLLOCK,
        "pause" => key::PAUSE,
        "insert" => key::INSERT,
        "home" => key::HOME,
        "end" => key::END,
        "pageup" => key::PAGEUP,
        "pagedown" => key::PAGEDOWN,
        "numlock" => key::NUMLOCK,
        "menu" | "apps" => key::COMPOSE,
        "escape" | "esc" => key::ESC,
        _ => {
            let n: u32 = name.strip_prefix('f')?.parse().ok()?;
            match n {
                1..=10 => key::F1 + n - 1,
                11 => key::F11,
                12 => key::F12,
                13..=24 => key::F13 + n - 13,
                _ => return None,
            }
        }
    };
    Some(code)
}

/// Sous X11, AltGr arrive seul (pas de faux Ctrl comme sous Windows).
pub fn companion_keys(_trigger: u32) -> Vec<u32> {
    Vec::new()
}

/// Un appui seul sur Alt n'ouvre pas de menu dans les applications Linux.
pub fn suppress_menu_activation(_trigger: u32) {}

fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|e| {
        format!("affichage X11 introuvable ({e}) : VoiceToText demande une session X11")
    })
}

/// Connexion partagée pour les requêtes ponctuelles (état des touches,
/// frappes simulées, fenêtre active).
fn display() -> Option<&'static (RustConnection, usize)> {
    static DISPLAY: OnceLock<Option<(RustConnection, usize)>> = OnceLock::new();
    DISPLAY
        .get_or_init(|| connect().map_err(|e| log::warn!("{e}")).ok())
        .as_ref()
}

fn root(conn: &RustConnection, screen: usize) -> Window {
    conn.setup().roots[screen].root
}

/// Suit le clavier dans un thread dédié. Rien n'est intercepté : les
/// événements sont seulement recopiés.
pub fn spawn_keyboard_hook(tx: Sender<KeyEvent>) -> Result<(), String> {
    if wayland_session() {
        log::warn!(
            "session Wayland : le raccourci n'est vu hors des applications X11 que si KDE \
             l'autorise (Configuration du système → prise en charge des applications X11 \
             historiques → « Toujours »)"
        );
    }
    let (conn, screen) = connect()?;
    let version = conn
        .xinput_xi_query_version(2, 2)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("XInput2 indisponible : {e}"))?;
    if (version.major_version, version.minor_version) < (2, 1) {
        return Err(format!(
            "XInput {}.{} trop ancien (2.1 requis)",
            version.major_version, version.minor_version
        ));
    }
    let injected = xtest_devices(&conn);
    conn.xinput_xi_select_events(
        root(&conn, screen),
        &[xinput::EventMask {
            // XIAllMasterDevices.
            deviceid: 1,
            mask: vec![xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE],
        }],
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| format!("abonnement aux touches : {e}"))?;

    thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || {
            loop {
                let event = match conn.wait_for_event() {
                    Ok(event) => event,
                    Err(e) => {
                        log::error!("connexion X11 perdue : {e}");
                        return;
                    }
                };
                let (raw, down) = match event {
                    Event::XinputRawKeyPress(raw) => (raw, true),
                    Event::XinputRawKeyRelease(raw) => (raw, false),
                    _ => continue,
                };
                let key = raw.detail.saturating_sub(X_KEYCODE_OFFSET);
                // Nos propres Ctrl+V simulés ne doivent pas perturber le
                // raccourci : périphérique XTEST, ou touches du clavier
                // virtuel (uinput) que XWayland ne distingue pas d'un vrai.
                if injected.contains(&raw.sourceid) || recently_injected(key) {
                    continue;
                }
                let sent = tx.send(KeyEvent {
                    key,
                    down,
                    at_ms: now_ms(),
                });
                if sent.is_err() {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Périphériques virtuels d'XTest (frappes simulées, les nôtres comprises).
/// Ceux de Steam Input (manette transformée en clavier) n'en font pas
/// partie : ses touches sont bien vues.
fn xtest_devices(conn: &RustConnection) -> Vec<xinput::DeviceId> {
    let Ok(reply) = conn
        .xinput_xi_query_device(0u16)
        .map_err(|e| e.to_string())
        .and_then(|c| c.reply().map_err(|e| e.to_string()))
    else {
        return Vec::new();
    };
    reply
        .infos
        .iter()
        .filter(|info| String::from_utf8_lossy(&info.name).contains("XTEST"))
        .map(|info| info.deviceid)
        .collect()
}

/// Touches enfoncées (bitmap de 256 keycodes X11).
fn keymap() -> Option<[u8; 32]> {
    let (conn, _) = display()?;
    Some(conn.query_keymap().ok()?.reply().ok()?.keys)
}

/// Touches du clavier enfoncées, en codes evdev (détection d'une touche dans
/// les réglages).
pub fn pressed_keys() -> Vec<u32> {
    keymap().map_or_else(Vec::new, |keys| {
        (X_KEYCODE_OFFSET..256)
            .map(|keycode| keycode - X_KEYCODE_OFFSET)
            .filter(|&code| is_down(&keys, code))
            .collect()
    })
}

fn is_down(keys: &[u8; 32], code: u32) -> bool {
    let keycode = code + X_KEYCODE_OFFSET;
    keys.get(keycode as usize / 8)
        .is_some_and(|byte| byte & (1 << (keycode % 8)) != 0)
}

/// État physique de la touche, indépendamment du suivi des événements.
pub fn key_is_down(key: u32) -> bool {
    keymap().is_some_and(|keys| is_down(&keys, key))
}

/// Une touche de modification (Ctrl, Maj, Alt, AltGr, Super) est enfoncée.
pub fn modifiers_down() -> bool {
    const MODIFIERS: [u32; 8] = [
        key::LEFTCTRL,
        key::RIGHTCTRL,
        key::LEFTSHIFT,
        key::RIGHTSHIFT,
        key::LEFTALT,
        key::RIGHTALT,
        key::LEFTMETA,
        key::RIGHTMETA,
    ];
    keymap().is_some_and(|keys| MODIFIERS.iter().any(|&k| is_down(&keys, k)))
}

/// Keycode X11 qui produit `sym` sans modificateur dans la disposition
/// active (AZERTY, QWERTZ, Dvorak…), sinon la position QWERTY `fallback`.
fn keycode_for(conn: &RustConnection, sym: Keysym, fallback: u32) -> Keycode {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let found = conn
        .get_keyboard_mapping(min, max - min + 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|mapping| {
            let per = mapping.keysyms_per_keycode as usize;
            if per == 0 {
                return None;
            }
            mapping
                .keysyms
                .chunks(per)
                .position(|syms| syms[0] == sym)
                .map(|i| min + i as u8)
        });
    found.unwrap_or((fallback + X_KEYCODE_OFFSET) as Keycode)
}

/// Session Wayland (les applications X11 passent par XWayland).
pub(crate) fn wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty())
}

/// Touches simulées par le clavier virtuel, ignorées par le suivi du clavier
/// jusqu'à l'heure indiquée (`now_ms`).
static INJECTED: Mutex<(Vec<u32>, u64)> = Mutex::new((Vec::new(), 0));
/// Les frappes simulées reviennent par XWayland en quelques millisecondes.
const INJECTED_ECHO_MS: u64 = 400;

fn recently_injected(key: u32) -> bool {
    let injected = INJECTED.lock().unwrap_or_else(|e| e.into_inner());
    now_ms() <= injected.1 && injected.0.contains(&key)
}

/// Ctrl + la touche qui produit `sym` (C, V ou Z). Sous Wayland, par le
/// clavier virtuel (uinput) s'il est accessible, sinon par XTest.
fn send_ctrl(sym: Keysym, fallback: u32) -> Result<(), String> {
    if wayland_session() {
        // La disposition active est aussi celle de XWayland.
        let letter = display().map_or(fallback, |(conn, _)| {
            u32::from(keycode_for(conn, sym, fallback)).saturating_sub(X_KEYCODE_OFFSET)
        });
        let keys = [key::LEFTCTRL, letter];
        {
            let mut injected = INJECTED.lock().unwrap_or_else(|e| e.into_inner());
            *injected = (keys.to_vec(), now_ms() + INJECTED_ECHO_MS);
        }
        match crate::uinput::send_chord(&keys) {
            Ok(()) => return Ok(()),
            Err(e) => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    log::warn!(
                        "clavier virtuel indisponible ({e}) : frappes par XTest, qui \
                         n'atteignent que les applications X11"
                    );
                });
            }
        }
    }
    send_ctrl_xtest(sym, fallback)
}

/// Ctrl + la touche qui produit `sym`, simulés avec XTest.
fn send_ctrl_xtest(sym: Keysym, fallback: u32) -> Result<(), String> {
    let (conn, screen) = display().ok_or("affichage X11 introuvable")?;
    let ctrl = keycode_for(conn, keysym::CONTROL_L, key::LEFTCTRL);
    let letter = keycode_for(conn, sym, fallback);
    let root = root(conn, *screen);
    let keys = [
        (xproto::KEY_PRESS_EVENT, ctrl),
        (xproto::KEY_PRESS_EVENT, letter),
        (xproto::KEY_RELEASE_EVENT, letter),
        (xproto::KEY_RELEASE_EVENT, ctrl),
    ];
    for (type_, keycode) in keys {
        conn.xtest_fake_input(type_, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .map_err(|e| format!("frappe simulée refusée (XTest) : {e}"))?;
    }
    // Attend que le serveur ait traité les frappes (et signale une erreur).
    conn.sync()
        .map_err(|e| format!("frappe simulée refusée (XTest) : {e}"))
}

/// Presse-papier gardé ouvert pendant toute la vie du programme : sous X11,
/// c'est le programme qui a copié qui fournit le contenu à la demande. Un
/// presse-papier fermé trop tôt laisserait un Ctrl+V sans rien à coller.
fn with_clipboard<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Result<R, String> {
    static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
    let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| format!("presse-papier : {e}"))?);
    }
    Ok(f(guard.as_mut().expect("presse-papier ouvert")))
}

fn clipboard_text() -> Option<String> {
    with_clipboard(|c| c.get_text().ok()).ok().flatten()
}

fn set_clipboard_text(text: &str) -> Result<(), String> {
    with_clipboard(|c| c.set_text(text))?.map_err(|e| format!("presse-papier : {e}"))
}

/// Remet `previous` dans le presse-papier un peu plus tard, si personne n'y
/// a mis autre chose que `inserted` entre-temps.
fn restore_clipboard_later(previous: Option<String>, inserted: String) {
    let Some(previous) = previous else {
        return;
    };
    thread::spawn(move || {
        thread::sleep(CLIPBOARD_RESTORE_DELAY);
        if clipboard_text().as_deref() == Some(inserted.as_str()) {
            let _ = set_clipboard_text(&previous);
        }
    });
}

/// Place le texte dans le presse-papier, sans coller.
pub fn copy_text(text: &str) -> Result<(), String> {
    set_clipboard_text(text)
}

/// Attente maximale de la copie après le Ctrl+C simulé.
const COPY_TIMEOUT: Duration = Duration::from_millis(500);

/// Texte sélectionné dans l'application active (Ctrl+C simulé), ou `None`
/// s'il n'y a pas de sélection. Le contenu texte précédent du presse-papier
/// est remis en place. Les touches de modification doivent être relâchées.
pub fn copy_selection() -> Option<String> {
    // Connexion à part : elle seule reçoit la notification XFixes.
    let (conn, screen) = connect().map_err(|e| log::warn!("{e}")).ok()?;
    conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
    let clipboard = conn
        .intern_atom(false, b"CLIPBOARD")
        .ok()?
        .reply()
        .ok()?
        .atom;
    conn.xfixes_select_selection_input(
        root(&conn, screen),
        clipboard,
        SelectionEventMask::SET_SELECTION_OWNER,
    )
    .ok()?
    .check()
    .ok()?;

    let previous = clipboard_text();
    send_ctrl(keysym::C, key::C)
        .map_err(|e| log::warn!("{e}"))
        .ok()?;
    // Sans sélection, la plupart des applications ne touchent pas au
    // presse-papier : son propriétaire ne change pas.
    let started = Instant::now();
    loop {
        if let Some(Event::XfixesSelectionNotify(_)) = conn.poll_for_event().ok()? {
            break;
        }
        if started.elapsed() > COPY_TIMEOUT {
            return None;
        }
        thread::sleep(Duration::from_millis(15));
    }
    // L'application peut écrire plusieurs formats à la suite : laisser finir.
    thread::sleep(Duration::from_millis(30));
    let selection = clipboard_text().filter(|s| !s.trim().is_empty());
    let _ = match previous {
        Some(previous) => set_clipboard_text(&previous),
        None => with_clipboard(|c| {
            let _ = c.clear();
        }),
    };
    selection
}

/// Insère le texte via le presse-papier et un Ctrl+V simulé, puis remet
/// l'ancien contenu texte du presse-papier.
pub struct ClipboardPaster;

impl TextSink for ClipboardPaster {
    fn insert(&self, text: &str) -> Result<(), Error> {
        let previous = clipboard_text();
        set_clipboard_text(text).map_err(Error::Sink)?;
        if let Err(e) = send_ctrl(keysym::V, key::V) {
            // Le texte reste dans le presse-papier : l'utilisateur peut coller à la main.
            return Err(Error::Sink(format!(
                "{e} : texte laissé dans le presse-papier"
            )));
        }
        restore_clipboard_later(previous, text.to_owned());
        Ok(())
    }

    fn undo(&self) -> Result<(), Error> {
        send_ctrl(keysym::Z, key::Z).map_err(Error::Sink)
    }
}

fn atom(conn: &RustConnection, name: &str) -> Option<xproto::Atom> {
    Some(
        conn.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom,
    )
}

fn property(
    conn: &RustConnection,
    window: Window,
    name: &str,
    kind: impl Into<xproto::Atom>,
) -> Option<xproto::GetPropertyReply> {
    let reply = conn
        .get_property(false, window, atom(conn, name)?, kind, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    (reply.format != 0).then_some(reply)
}

fn active_window(conn: &RustConnection, screen: usize) -> Option<Window> {
    property(
        conn,
        root(conn, screen),
        "_NET_ACTIVE_WINDOW",
        AtomEnum::WINDOW,
    )?
    .value32()?
    .next()
    .filter(|&w| w != x11rb::NONE)
}

/// Fenêtre au premier plan : nom du programme et titre.
///
/// Le nom est la classe de la fenêtre (`WM_CLASS`, ex. "firefox", "Slack") :
/// les applications Flatpak, courantes sur le Steam Deck, annoncent un PID
/// pris dans leur bac à sable, inutilisable ici. Le nom de l'exécutable ne
/// sert que si la fenêtre n'a pas de classe.
pub fn foreground_target() -> Target {
    let Some((conn, screen)) = display() else {
        return Target::default();
    };
    let Some(window) = active_window(conn, *screen) else {
        return Target::default();
    };
    let title = atom(conn, "UTF8_STRING")
        .and_then(|utf8| property(conn, window, "_NET_WM_NAME", utf8))
        .or_else(|| property(conn, window, "WM_NAME", AtomEnum::STRING))
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .filter(|t| !t.is_empty());
    let app = property(conn, window, "WM_CLASS", AtomEnum::STRING)
        .and_then(|reply| window_class(&reply.value))
        .or_else(|| {
            property(conn, window, "_NET_WM_PID", AtomEnum::CARDINAL)
                .and_then(|reply| reply.value32()?.next())
                .and_then(process_name)
        });
    Target { app, title }
}

/// Classe d'une fenêtre : `WM_CLASS` contient « instance\0classe\0 ».
fn window_class(value: &[u8]) -> Option<String> {
    let mut parts = value.split(|&b| b == 0).filter(|p| !p.is_empty());
    let instance = parts.next()?;
    let class = parts.next().unwrap_or(instance);
    Some(String::from_utf8_lossy(class).into_owned())
}

/// Nom de l'exécutable d'un processus, ex. "firefox".
fn process_name(pid: u32) -> Option<String> {
    if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe"))
        && let Some(name) = exe.file_name().and_then(|n| n.to_str())
    {
        return Some(name.to_owned());
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_owned()).filter(|c| !c.is_empty())
}

/// La fenêtre active est en plein écran (jeu, vidéo, présentation).
pub fn fullscreen_app_running() -> bool {
    let Some((conn, screen)) = display() else {
        return false;
    };
    let (Some(window), Some(fullscreen)) = (
        active_window(conn, *screen),
        atom(conn, "_NET_WM_STATE_FULLSCREEN"),
    ) else {
        return false;
    };
    property(conn, window, "_NET_WM_STATE", AtomEnum::ATOM)
        .and_then(|reply| {
            reply
                .value32()
                .map(|mut states| states.any(|s| s == fullscreen))
        })
        .unwrap_or(false)
}

/// Description de la session pour le journal (diagnostic à distance) :
/// système, type de session, affichage, accès au clavier virtuel.
pub fn session_report() -> Vec<String> {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| "—".into());
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|release| {
            release.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "inconnu".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|k| k.trim().to_owned())
        .unwrap_or_else(|_| "inconnu".into());
    let x11 = match connect() {
        Ok((conn, _)) => match conn
            .xinput_xi_query_version(2, 2)
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()))
        {
            Ok(v) => format!("connecté, XInput {}.{}", v.major_version, v.minor_version),
            Err(e) => format!("connecté, XInput indisponible ({e})"),
        },
        Err(e) => e,
    };
    let uinput = match std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(_) => "accessible".to_owned(),
        Err(e) => format!("inaccessible ({e})"),
    };
    vec![
        format!("système : {os}, noyau {kernel}"),
        format!(
            "session : {} ({}), bureau {}",
            var("XDG_SESSION_TYPE"),
            if wayland_session() { "Wayland" } else { "X11" },
            var("XDG_CURRENT_DESKTOP")
        ),
        format!(
            "affichage : DISPLAY={}, WAYLAND_DISPLAY={}, GDK_BACKEND={}",
            var("DISPLAY"),
            var("WAYLAND_DISPLAY"),
            var("GDK_BACKEND")
        ),
        format!("X11 : {x11}"),
        format!("clavier virtuel (/dev/uinput) : {uinput}"),
        format!(
            "AppImage : {}",
            std::env::var_os("APPIMAGE").map_or("non".into(), |p| p.to_string_lossy().into_owned())
        ),
    ]
}

/// Sous Linux, les bibliothèques ouvrent les chemins tels quels (UTF-8).
pub fn ascii_path(path: &std::path::Path) -> std::path::PathBuf {
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_code("AltGr"), Some(100));
        assert_eq!(key_code(" altgr "), Some(100));
        assert_eq!(key_code("F1"), Some(59));
        assert_eq!(key_code("F10"), Some(68));
        assert_eq!(key_code("F11"), Some(87));
        assert_eq!(key_code("F12"), Some(88));
        assert_eq!(key_code("F13"), Some(183));
        assert_eq!(key_code("F24"), Some(194));
        assert_eq!(key_code("F25"), None);
        assert_eq!(key_code("#183"), Some(183));
        assert_eq!(key_code("PageDown"), Some(109));
        assert_eq!(key_code("F0"), None);
        assert_eq!(key_code("nimportequoi"), None);
    }

    /// Chaque touche proposée dans les réglages est reconnue.
    #[test]
    fn every_settings_key_is_known() {
        for name in vtt_core::config::HOTKEY_NAMES {
            assert!(key_code(name).is_some(), "{name}");
        }
    }

    #[test]
    fn window_classes() {
        assert_eq!(
            window_class(b"Navigator\0firefox\0").as_deref(),
            Some("firefox")
        );
        assert_eq!(window_class(b"slack\0Slack\0").as_deref(), Some("Slack"));
        assert_eq!(window_class(b"xterm\0").as_deref(), Some("xterm"));
        assert_eq!(window_class(b""), None);
    }

    #[test]
    fn keymap_bits() {
        let mut keys = [0u8; 32];
        // AltGr : evdev 100 → keycode X11 108 → octet 13, bit 4.
        keys[13] = 1 << 4;
        assert!(is_down(&keys, key::RIGHTALT));
        assert!(!is_down(&keys, key::RIGHTCTRL));
    }
}
