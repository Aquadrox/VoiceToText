//! Sonde du mode Jeu du Steam Deck (gamescope) : avant d'y faire marcher la
//! dictée, on observe ce qui est possible sur le vrai Deck.
//!
//! En mode Jeu, il n'y a plus KDE : l'écran est géré par gamescope, Steam
//! fait office de bureau, les jeux tournent dans un ou plusieurs serveurs
//! XWayland de gamescope. La sonde tourne en arrière-plan (service
//! utilisateur lancé depuis le mode Bureau), attend gamescope, puis note :
//! - la session : processus gamescope et Steam, leurs variables
//!   d'affichage, serveurs X, cibles systemd actives ;
//! - les propriétés gamescope des écrans X (application au premier plan…) ;
//! - les touches candidates vues (F13 à F24, Pause… et la touche de dictée
//!   réglée), par serveur X ; les autres touches sont seulement comptées,
//!   jamais notées ;
//! - à chaque appui sur la touche de dictée : une fenêtre de surimpression
//!   (`GAMESCOPE_EXTERNAL_OVERLAY`, comme MangoHud), puis « voicetotext »
//!   tapé touche par touche par le clavier virtuel, « xtest » tapé par XTest,
//!   puis « colle » collé.
//!
//! Rien n'est enregistré au micro, rien n'est envoyé.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::linux::X_KEYCODE_OFFSET;

/// Journal de la sonde (fourni par l'application).
pub type Note = Arc<dyn Fn(String) + Send + Sync>;

/// Variables d'affichage relevées dans les processus de gamescope.
const DISPLAY_VARS: [&str; 7] = [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "GAMESCOPE_WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
    "STEAM_GAMESCOPE_VIRTUAL_WHITE",
];

/// Lance la sonde ; ne rend jamais la main (le service l'arrête).
pub fn run_game_mode_probe(note: Note, trigger: u32, trigger_name: &str) -> ! {
    note(format!(
        "sonde du mode Jeu : touche de dictée réglée {trigger_name} (code {trigger})"
    ));
    report_input_devices(&note);
    report_uinput(&note);
    let mut was_present = false;
    let watched: Arc<Mutex<Vec<String>>> = Arc::default();
    loop {
        let session = gamescope_session();
        let present = session.is_some();
        if present != was_present {
            was_present = present;
            match &session {
                Some(session) => {
                    note("gamescope détecté (mode Jeu)".into());
                    for line in session {
                        note(format!("  {line}"));
                    }
                    report_systemd_targets(&note);
                }
                None => note("gamescope absent (mode Bureau ou transition) : attente".into()),
            }
        }
        if present {
            for display in x_displays() {
                let mut known = watched.lock().unwrap_or_else(|e| e.into_inner());
                if known.contains(&display) {
                    continue;
                }
                known.push(display.clone());
                drop(known);
                watch_display(Arc::clone(&note), display, trigger, Arc::clone(&watched));
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

/// Processus d'un nom donné : (pid, ligne de commande, variables).
fn processes(name: &str) -> Vec<(u32, String, HashMap<String, String>)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|comm| comm.trim() == name)
        })
        .map(|pid| {
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|raw| String::from_utf8_lossy(&raw).replace('\0', " "))
                .unwrap_or_default();
            let environ = std::fs::read(format!("/proc/{pid}/environ"))
                .map(|raw| {
                    raw.split(|&b| b == 0)
                        .filter_map(|pair| {
                            let pair = String::from_utf8_lossy(pair);
                            let (key, value) = pair.split_once('=')?;
                            DISPLAY_VARS
                                .contains(&key)
                                .then(|| (key.to_owned(), value.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            (pid, cmdline.trim().to_owned(), environ)
        })
        .collect()
}

/// Description de la session gamescope, ou `None` hors du mode Jeu.
fn gamescope_session() -> Option<Vec<String>> {
    let gamescope = processes("gamescope");
    if gamescope.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for (pid, cmdline, env) in gamescope.iter().chain(&processes("steam")) {
        lines.push(format!("processus {pid} : {cmdline}"));
        let mut vars: Vec<_> = env.iter().collect();
        vars.sort();
        for (key, value) in vars {
            lines.push(format!("  {key}={value}"));
        }
    }
    lines.push(format!("serveurs X : {}", x_displays().join(", ")));
    Some(lines)
}

/// Serveurs X de la machine (« :0 », « :1 »…).
fn x_displays() -> Vec<String> {
    let mut displays: Vec<String> = std::fs::read_dir("/tmp/.X11-unix")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| Some(format!(":{}", e.file_name().to_str()?.strip_prefix('X')?)))
                .collect()
        })
        .unwrap_or_default();
    displays.sort();
    displays
}

fn report_systemd_targets(note: &Note) {
    match std::process::Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "--type=target",
            "--state=active",
            "--no-legend",
            "--plain",
        ])
        .env_remove("LD_LIBRARY_PATH")
        .output()
    {
        Ok(output) => {
            let targets: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
                .collect();
            note(format!("cibles systemd actives : {}", targets.join(", ")));
        }
        Err(e) => note(format!("systemctl : {e}")),
    }
}

/// Périphériques d'entrée : nom et lisibilité directe (evdev).
fn report_input_devices(note: &Note) {
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return;
    };
    let mut lines: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let event = entry.file_name().to_str()?.to_owned();
            event.starts_with("event").then_some(())?;
            let name = std::fs::read_to_string(entry.path().join("device/name")).ok()?;
            let readable = std::fs::File::open(format!("/dev/input/{event}")).is_ok();
            Some(format!(
                "  {event} : {} ({})",
                name.trim(),
                if readable { "lisible" } else { "non lisible" }
            ))
        })
        .collect();
    lines.sort();
    note(format!("périphériques d'entrée :\n{}", lines.join("\n")));
}

fn report_uinput(note: &Note) {
    let access = match std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(_) => "accessible".to_owned(),
        Err(e) => format!("inaccessible ({e})"),
    };
    note(format!("clavier virtuel (/dev/uinput) : {access}"));
}

/// Touches candidates pour une combinaison Steam + gâchette : F13 à F24,
/// Pause, Arrêt défil., Inser, Menu (codes evdev).
fn candidate_name(code: u32) -> Option<String> {
    match code {
        183..=194 => Some(format!("F{}", code - 170)),
        119 => Some("Pause".into()),
        70 => Some("ScrollLock".into()),
        110 => Some("Insert".into()),
        127 => Some("Menu".into()),
        _ => None,
    }
}

/// Suit un serveur X : propriétés de gamescope, touches, tests à l'appui.
fn watch_display(note: Note, display: String, trigger: u32, watched: Arc<Mutex<Vec<String>>>) {
    thread::spawn(move || {
        let forget = |watched: &Arc<Mutex<Vec<String>>>| {
            watched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|d| *d != display);
        };
        let (conn, screen) = match x11rb::connect(Some(&display)) {
            Ok(connection) => connection,
            Err(e) => {
                note(format!("{display} : connexion impossible ({e})"));
                forget(&watched);
                return;
            }
        };
        let root = conn.setup().roots[screen].root;
        let size = (
            conn.setup().roots[screen].width_in_pixels,
            conn.setup().roots[screen].height_in_pixels,
        );
        note(format!(
            "{display} : connecté, écran {}x{} ; propriétés : {}",
            size.0,
            size.1,
            gamescope_properties(&conn, root)
        ));
        let subscribed = conn
            .xinput_xi_query_version(2, 2)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|_| {
                conn.xinput_xi_select_events(
                    root,
                    &[xinput::EventMask {
                        deviceid: 1,
                        mask: vec![
                            xinput::XIEventMask::RAW_KEY_PRESS
                                | xinput::XIEventMask::RAW_KEY_RELEASE,
                        ],
                    }],
                )
                .ok()?
                .check()
                .ok()
            });
        if subscribed.is_none() {
            note(format!(
                "{display} : touches non suivies (XInput2 indisponible)"
            ));
        }
        let others = AtomicU64::new(0);
        let mut last_summary = Instant::now();
        let mut last_props = Instant::now();
        loop {
            let event = match conn.poll_for_event() {
                Ok(Some(event)) => event,
                Ok(None) => {
                    if last_summary.elapsed() > Duration::from_secs(30) {
                        let count = others.swap(0, Ordering::Relaxed);
                        if count > 0 {
                            note(format!(
                                "{display} : {count} autres touches vues (non notées)"
                            ));
                        }
                        last_summary = Instant::now();
                    }
                    if last_props.elapsed() > Duration::from_secs(20) {
                        note(format!(
                            "{display} : propriétés : {}",
                            gamescope_properties(&conn, root)
                        ));
                        last_props = Instant::now();
                    }
                    thread::sleep(Duration::from_millis(15));
                    continue;
                }
                Err(e) => {
                    note(format!("{display} : connexion perdue ({e})"));
                    forget(&watched);
                    return;
                }
            };
            let (raw, down) = match event {
                Event::XinputRawKeyPress(raw) => (raw, true),
                Event::XinputRawKeyRelease(raw) => (raw, false),
                _ => continue,
            };
            let code = raw.detail.saturating_sub(X_KEYCODE_OFFSET);
            let name = if code == trigger {
                Some("touche de dictée".to_owned())
            } else {
                candidate_name(code)
            };
            let Some(name) = name else {
                if down {
                    others.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            };
            note(format!(
                "{display} : {name} (code {code}) {} — périphérique {}",
                if down { "appuyée" } else { "relâchée" },
                raw.sourceid
            ));
            if code == trigger && !down {
                run_tests(&note, &display, &conn, root, size);
            }
        }
    });
}

/// Propriétés GAMESCOPE_* et STEAM_* de la fenêtre racine (nombres seuls).
fn gamescope_properties(conn: &RustConnection, root: u32) -> String {
    let Ok(Ok(list)) = conn.list_properties(root).map(|c| c.reply()) else {
        return "illisibles".into();
    };
    let mut found = Vec::new();
    for atom in list.atoms {
        let Ok(Ok(name)) = conn.get_atom_name(atom).map(|c| c.reply()) else {
            continue;
        };
        let name = String::from_utf8_lossy(&name.name).into_owned();
        if !(name.starts_with("GAMESCOPE") || name.starts_with("STEAM")) {
            continue;
        }
        let value = conn
            .get_property(false, root, atom, AtomEnum::ANY, 0, 8)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|reply| {
                Some(
                    reply
                        .value32()?
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .unwrap_or_default();
        found.push(format!("{name}=[{value}]"));
    }
    if found.is_empty() {
        "aucune propriété gamescope".into()
    } else {
        found.sort();
        found.join(" ")
    }
}

/// À chaque relâchement de la touche de dictée : surimpression, frappe,
/// collage, à une seconde d'intervalle.
fn run_tests(note: &Note, display: &str, conn: &RustConnection, root: u32, size: (u16, u16)) {
    // 1. Fenêtre de surimpression (bandeau orange en haut à gauche).
    match overlay(conn, root, size) {
        Ok(window) => {
            note(format!(
                "{display} : surimpression affichée 3 s (bandeau orange en haut à gauche ?)"
            ));
            thread::sleep(Duration::from_secs(3));
            let _ = conn.destroy_window(window);
            let _ = conn.flush();
        }
        Err(e) => note(format!("{display} : surimpression impossible ({e})")),
    }
    // 2. « voicetotext » tapé touche par touche (clavier virtuel), d'après
    //    la disposition de ce serveur X.
    thread::sleep(Duration::from_secs(1));
    let mut typed = 0;
    for letter in "voicetotext".chars() {
        let Some(code) = evdev_for_letter(conn, letter) else {
            continue;
        };
        if crate::uinput::send_chord(&[code]).is_ok() {
            typed += 1;
        }
    }
    note(format!(
        "{display} : « voicetotext » tapé par le clavier virtuel ({typed}/11 touches envoyées)"
    ));
    // 2 bis. « xtest » tapé par XTest sur ce serveur X (frappes X11 : vues
    //        seulement par les applications de ce serveur).
    thread::sleep(Duration::from_secs(1));
    let mut sent = 0;
    for letter in "xtest".chars() {
        let Some(code) = evdev_for_letter(conn, letter) else {
            continue;
        };
        let keycode = (code + X_KEYCODE_OFFSET) as u8;
        let pressed = conn
            .xtest_fake_input(
                xproto::KEY_PRESS_EVENT,
                keycode,
                x11rb::CURRENT_TIME,
                root,
                0,
                0,
                0,
            )
            .and_then(|_| {
                conn.xtest_fake_input(
                    xproto::KEY_RELEASE_EVENT,
                    keycode,
                    x11rb::CURRENT_TIME,
                    root,
                    0,
                    0,
                    0,
                )
            });
        if pressed.is_ok() && conn.sync().is_ok() {
            sent += 1;
        }
        thread::sleep(Duration::from_millis(10));
    }
    note(format!(
        "{display} : « xtest » tapé par XTest ({sent}/5 touches envoyées)"
    ));
    // 3. Collage : texte dans le presse-papier de ce serveur, puis Ctrl+V.
    thread::sleep(Duration::from_secs(1));
    match paste_test(display) {
        Ok(()) => note(format!(
            "{display} : « colle » mis au presse-papier puis Ctrl+V envoyé"
        )),
        Err(e) => note(format!("{display} : collage impossible ({e})")),
    }
    note(format!(
        "{display} : tests terminés ; noter ce qui est apparu (bandeau, « voicetotext », « xtest », « colle »)"
    ));
}

fn overlay(conn: &RustConnection, root: u32, size: (u16, u16)) -> Result<u32, String> {
    let window = conn.generate_id().map_err(|e| e.to_string())?;
    let (width, height) = ((size.0 / 3).max(200), 48);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        16,
        16,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .background_pixel(0x00ff_7a1c)
            .override_redirect(1)
            .event_mask(EventMask::EXPOSURE),
    )
    .map_err(|e| e.to_string())?;
    let atom = conn
        .intern_atom(false, b"GAMESCOPE_EXTERNAL_OVERLAY")
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;
    conn.change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &[1])
        .map_err(|e| e.to_string())?;
    conn.map_window(window).map_err(|e| e.to_string())?;
    conn.sync().map_err(|e| e.to_string())?;
    Ok(window)
}

/// Code evdev de la touche qui produit `letter` sur ce serveur X.
fn evdev_for_letter(conn: &RustConnection, letter: char) -> Option<u32> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = conn
        .get_keyboard_mapping(min, max - min + 1)
        .ok()?
        .reply()
        .ok()?;
    let per = usize::from(mapping.keysyms_per_keycode);
    if per == 0 {
        return None;
    }
    let index = mapping
        .keysyms
        .chunks(per)
        .position(|syms| syms[0] == u32::from(letter))?;
    Some(u32::from(min) + index as u32 - X_KEYCODE_OFFSET)
}

fn paste_test(display: &str) -> Result<(), String> {
    // Presse-papier propre à ce serveur X : connexion arboard à part,
    // gardée le temps que l'application lise le contenu.
    let previous = std::env::var_os("DISPLAY");
    // SAFETY: la sonde ne lit pas l'environnement ailleurs pendant ce
    // court intervalle (ses autres fils attendent des événements X).
    unsafe { std::env::set_var("DISPLAY", display) };
    let clipboard = arboard::Clipboard::new();
    match previous {
        // SAFETY: idem.
        Some(previous) => unsafe { std::env::set_var("DISPLAY", previous) },
        None => unsafe { std::env::remove_var("DISPLAY") },
    }
    let mut clipboard = clipboard.map_err(|e| format!("presse-papier : {e}"))?;
    clipboard
        .set_text("colle")
        .map_err(|e| format!("presse-papier : {e}"))?;
    // Ctrl gauche + V (position QWERTY ; AZERTY : même touche).
    crate::uinput::send_chord(&[29, 47])?;
    thread::sleep(Duration::from_secs(2));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_keys() {
        assert_eq!(candidate_name(183).as_deref(), Some("F13"));
        assert_eq!(candidate_name(194).as_deref(), Some("F24"));
        assert_eq!(candidate_name(119).as_deref(), Some("Pause"));
        assert_eq!(candidate_name(30), None, "lettre : jamais notée");
    }
}
