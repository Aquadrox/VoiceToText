//! Intégration système : capture micro (toutes plateformes), hook clavier et
//! insertion de texte (Windows, Linux sous X11 ; pas encore les autres OS).

pub mod audio;
pub mod capture;
mod clipboard;

#[cfg(windows)]
mod autostart_windows;
#[cfg(windows)]
pub use autostart_windows::{autostart_enabled, set_autostart};
#[cfg(windows)]
mod clipboard_windows;
#[cfg(windows)]
mod context_windows;
#[cfg(windows)]
pub use context_windows::focused_field_context;
#[cfg(windows)]
mod media_windows;
#[cfg(windows)]
pub use media_windows::MediaPauser;
#[cfg(windows)]
mod microphone_windows;
#[cfg(windows)]
pub use microphone_windows::microphone_apps;
#[cfg(windows)]
mod secrets_windows;
#[cfg(windows)]
pub use secrets_windows::{
    delete_secret, list_secrets, load_secret, protect, store_secret, unprotect,
};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    ClickRecorder, ClipboardPaster, ascii_path, companion_keys, copy_selection, copy_text,
    exclude_from_capture, foreground_is_shell_chrome, foreground_rect, foreground_target,
    fullscreen_app_running, key_code, key_is_down, modifier_of, modifiers_down, paste_image,
    play_sound, pressed_keys, screenshot, shortcut_key, spawn_keyboard_hook,
    suppress_menu_activation,
};

#[cfg(target_os = "linux")]
mod autostart_linux;
#[cfg(target_os = "linux")]
pub use autostart_linux::{autostart_enabled, set_autostart};
#[cfg(target_os = "linux")]
mod clipboard_linux;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    ClipboardPaster, ascii_path, companion_keys, copy_selection, copy_text, focused_field_context,
    foreground_is_shell_chrome, foreground_rect, foreground_target, fullscreen_app_running,
    is_steam_deck, key_code, key_is_down, modifier_of, modifiers_down, pressed_keys,
    session_report, shortcut_key, spawn_keyboard_hook, suppress_menu_activation,
};
#[cfg(target_os = "linux")]
mod gamemode_linux;
#[cfg(target_os = "linux")]
pub use gamemode_linux::{Note, run_game_mode_probe};
#[cfg(target_os = "linux")]
mod kwin;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod screenshot_linux;
#[cfg(target_os = "linux")]
pub use screenshot_linux::{ClickRecorder, exclude_from_capture, paste_image, screenshot};
#[cfg(target_os = "linux")]
mod shortcuts_linux;
#[cfg(target_os = "linux")]
pub use shortcuts_linux::set_global_keys;
#[cfg(target_os = "linux")]
mod microphone_linux;
#[cfg(target_os = "linux")]
pub use microphone_linux::microphone_apps;
#[cfg(target_os = "linux")]
mod media_linux;
#[cfg(target_os = "linux")]
pub use media_linux::MediaPauser;
#[cfg(target_os = "linux")]
mod secrets_linux;
#[cfg(target_os = "linux")]
mod uinput;
#[cfg(target_os = "linux")]
pub use secrets_linux::{
    delete_secret, list_secrets, load_secret, protect, store_secret, unprotect,
};
#[cfg(target_os = "linux")]
mod sound;
#[cfg(target_os = "linux")]
pub use sound::play_sound;

/// Steam Deck : seulement sous Linux.
#[cfg(not(target_os = "linux"))]
pub fn is_steam_deck() -> bool {
    false
}

/// Raccourcis déclarés au bureau : seulement sous Linux (Wayland).
#[cfg(not(target_os = "linux"))]
pub fn set_global_keys(
    _enabled: bool,
    _trigger: u32,
    _trigger_name: &str,
    _command: Option<u32>,
    _command_name: &str,
) {
}

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::{
    ClickRecorder, ClipboardPaster, MediaPauser, ascii_path, autostart_enabled, companion_keys,
    copy_selection, copy_text, delete_secret, exclude_from_capture, focused_field_context,
    foreground_is_shell_chrome, foreground_rect, foreground_target, fullscreen_app_running,
    key_code, key_is_down, list_secrets, load_secret, microphone_apps, modifier_of, modifiers_down,
    paste_image, play_sound, pressed_keys, protect, screenshot, set_autostart, shortcut_key,
    spawn_keyboard_hook, store_secret, suppress_menu_activation, unprotect,
};

/// Extrait du champ ayant le focus clavier (opt-in, jamais journalisé).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedField {
    /// Texte borné autour du curseur (`None` si indisponible ou protégé).
    pub text: Option<String>,
    /// `true` si le champ est un champ de mot de passe (lecture refusée).
    pub is_password: bool,
}

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Appui ou relâchement d'une touche physique.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    /// Code de touche de la plateforme (virtual-key sous Windows, code evdev
    /// du noyau sous Linux).
    pub key: u32,
    pub down: bool,
    /// Horodatage en millisecondes, même horloge que [`now_ms`].
    pub at_ms: u64,
}

/// Horloge monotone en millisecondes depuis le premier appel.
pub fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Attend qu'une touche du clavier soit enfoncée (détection de la touche de
/// dictée dans les réglages) et renvoie son code, ou `None` après `timeout`.
/// Les touches déjà enfoncées au départ ne comptent qu'une fois relâchées
/// puis enfoncées de nouveau. Fonctionne sans le suivi du clavier : la
/// détection marche même si le modèle n'est pas encore chargé.
pub fn capture_key(timeout: Duration) -> Option<u32> {
    let started = Instant::now();
    let mut held = pressed_keys();
    while started.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(15));
        let now = pressed_keys();
        let new: Vec<u32> = now.iter().copied().filter(|k| !held.contains(k)).collect();
        if !new.is_empty() {
            // AltGr sous Windows : Ctrl gauche + Alt droit d'un coup ; on
            // garde la touche qui a un nom.
            return Some(
                new.iter()
                    .copied()
                    .find(|&k| named_key(k).is_some())
                    .unwrap_or(new[0]),
            );
        }
        held = now;
    }
    None
}

/// Nom de la configuration pour un code de touche : « AltGr », « F13 »… ou
/// le code brut (`#183`) pour une touche sans nom.
pub fn key_name(code: u32) -> String {
    named_key(code).map_or_else(|| format!("#{code}"), str::to_owned)
}

fn named_key(code: u32) -> Option<&'static str> {
    vtt_core::config::HOTKEY_NAMES
        .iter()
        .copied()
        .find(|name| key_code(name) == Some(code))
}

/// Touche de modification d'un raccourci global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

impl Modifier {
    /// Nom dans la syntaxe des raccourcis (« Alt+Shift+V »).
    fn token(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Super => "Super",
        }
    }
}

/// Attend une combinaison (touches de modification puis une touche) et la
/// renvoie dans la syntaxe des raccourcis globaux : « Alt+Shift+V »,
/// « Ctrl+F9 »… Échap seule annule. Une lettre, un chiffre ou Espace sans
/// modificateur est refusé : le raccourci empêcherait de les taper.
pub fn capture_shortcut(timeout: Duration) -> Result<String, String> {
    let started = Instant::now();
    let mut held = pressed_keys();
    while started.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(15));
        let now = pressed_keys();
        let pressed = now
            .iter()
            .copied()
            .find(|k| !held.contains(k) && modifier_of(*k).is_none());
        if let Some(code) = pressed {
            let mut modifiers: Vec<Modifier> = now.iter().filter_map(|&k| modifier_of(k)).collect();
            modifiers.sort();
            modifiers.dedup();
            return shortcut_text(&modifiers, code);
        }
        held = now;
    }
    Err("aucun raccourci détecté en 10 secondes".into())
}

fn shortcut_text(modifiers: &[Modifier], code: u32) -> Result<String, String> {
    if modifiers.is_empty() && key_code("Escape") == Some(code) {
        return Err("détection annulée".into());
    }
    let key = shortcut_key(code).ok_or(
        "touche non utilisable dans un raccourci : essayez une lettre, un chiffre ou F1 à F24",
    )?;
    let typing = key.len() == 1 || key == "Space" || key == "Enter";
    if modifiers.is_empty() && typing {
        return Err(format!(
            "« {key} » seule empêcherait de taper ce caractère : ajoutez Ctrl, Alt ou Maj"
        ));
    }
    let mut parts: Vec<&str> = modifiers.iter().map(|m| m.token()).collect();
    parts.push(&key);
    Ok(parts.join("+"))
}

/// Fréquence d'observation de la fenêtre active pendant la détection par clic.
const APP_CAPTURE_POLL: Duration = Duration::from_millis(40);

/// Durée pendant laquelle l'application ciblée doit rester au premier plan
/// (sans modificateur Alt/Super maintenu) pour éviter de capter un passage
/// transitoire (Alt+Tab, clic sur la barre des tâches).
const APP_CAPTURE_STABLE: Duration = Duration::from_millis(180);

/// Instantané observé à chaque tour de boucle de [`capture_window_app`].
struct WindowSample {
    target: vtt_core::Target,
    is_shell: bool,
    escape_down: bool,
    nav_modifier_down: bool,
    elapsed: Duration,
}

/// Attend que l'utilisateur clique sur une autre fenêtre (ou bascule vers elle
/// par la barre des tâches / Alt+Tab) et renvoie son application cible.
///
/// Ignore la fenêtre de VoiceToText elle-même, les éléments du bureau (barre
/// des tâches, menu Démarrer, sélecteur Alt+Tab) ainsi que les fenêtres
/// survolées tant que Alt ou Super reste enfoncé. Échap annule la détection.
pub fn capture_window_app(timeout: Duration) -> Result<vtt_core::Target, String> {
    let mut self_keys = vec!["voicetotext".to_owned()];
    if let Some(app) = foreground_target().app.as_deref() {
        let key = vtt_core::context::normalize_app_key(app);
        if !key.is_empty() && !self_keys.contains(&key) {
            self_keys.push(key);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(name) = exe.file_name().and_then(|n| n.to_str())
    {
        let key = vtt_core::context::normalize_app_key(name);
        if !key.is_empty() && !self_keys.contains(&key) {
            self_keys.push(key);
        }
    }

    let escape = key_code("Escape");
    let started = Instant::now();
    capture_window_app_with(
        timeout,
        &self_keys,
        || {
            let keys = pressed_keys();
            let escape_down = escape.is_some_and(|code| keys.contains(&code));
            let nav_modifier_down = keys
                .iter()
                .filter_map(|&code| modifier_of(code))
                .any(|m| matches!(m, Modifier::Alt | Modifier::Super));
            WindowSample {
                target: foreground_target(),
                is_shell: foreground_is_shell_chrome(),
                escape_down,
                nav_modifier_down,
                elapsed: started.elapsed(),
            }
        },
        || std::thread::sleep(APP_CAPTURE_POLL),
    )
}

fn capture_window_app_with(
    timeout: Duration,
    self_keys: &[String],
    mut poll: impl FnMut() -> WindowSample,
    mut wait_step: impl FnMut(),
) -> Result<vtt_core::Target, String> {
    let initial = poll();
    let mut escape_was_down = initial.escape_down;
    let mut candidate: Option<(String, Duration)> = None;

    loop {
        wait_step();
        let sample = poll();
        if sample.escape_down && !escape_was_down {
            return Err("détection annulée".into());
        }
        escape_was_down = sample.escape_down;

        if sample.elapsed >= timeout {
            return Err(format!(
                "aucune fenêtre détectée en {} secondes",
                timeout.as_secs()
            ));
        }

        if sample.nav_modifier_down || sample.is_shell {
            candidate = None;
            continue;
        }

        let Some(app) = sample.target.app.as_deref() else {
            candidate = None;
            continue;
        };
        let key = vtt_core::context::normalize_app_key(app);
        if key.is_empty() || self_keys.iter().any(|s| s == &key) {
            candidate = None;
            continue;
        }

        match &candidate {
            Some((prev, since)) if prev == &key => {
                if sample.elapsed.saturating_sub(*since) >= APP_CAPTURE_STABLE {
                    return Ok(sample.target);
                }
            }
            _ => {
                candidate = Some((key, sample.elapsed));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        for name in vtt_core::config::HOTKEY_NAMES {
            if let Some(code) = key_code(name) {
                assert_eq!(key_name(code), *name);
            }
        }
        assert_eq!(key_name(1000), "#1000");
        assert_eq!(key_code(&key_name(1000)), Some(1000));
    }

    #[test]
    fn shortcut_texts() {
        let escape = key_code("Escape");
        if let Some(escape) = escape {
            assert!(shortcut_text(&[], escape).is_err());
        }
        if let Some(f9) = key_code("F9") {
            assert_eq!(shortcut_text(&[], f9).unwrap(), "F9");
            assert_eq!(
                shortcut_text(&[Modifier::Ctrl, Modifier::Shift], f9).unwrap(),
                "Ctrl+Shift+F9"
            );
        }
    }

    fn sample(
        app: Option<&str>,
        title: Option<&str>,
        is_shell: bool,
        escape_down: bool,
        nav_modifier_down: bool,
        elapsed_ms: u64,
    ) -> WindowSample {
        WindowSample {
            target: vtt_core::Target {
                app: app.map(str::to_owned),
                title: title.map(str::to_owned),
                ..vtt_core::Target::default()
            },
            is_shell,
            escape_down,
            nav_modifier_down,
            elapsed: Duration::from_millis(elapsed_ms),
        }
    }

    #[test]
    fn capture_window_app_ignores_self_shell_and_alttab_then_returns_stable_target() {
        let mut samples = vec![
            // État initial : VoiceToText au premier plan.
            sample(
                Some("VoiceToText.exe"),
                Some("VoiceToText"),
                false,
                false,
                false,
                0,
            ),
            // Encore VoiceToText.
            sample(
                Some("voicetotext.exe"),
                Some("VoiceToText"),
                false,
                false,
                false,
                40,
            ),
            // Clic sur la barre des tâches (chrome shell).
            sample(Some("explorer.exe"), None, true, false, false, 80),
            // Passage par une autre fenêtre pendant Alt+Tab (Alt enfoncé).
            sample(Some("chrome.exe"), Some("Accueil"), false, false, true, 120),
            sample(Some("chrome.exe"), Some("Accueil"), false, false, true, 400),
            // L'utilisateur choisit son application bancaire et relâche Alt.
            sample(
                Some("Banque.exe"),
                Some("Comptes"),
                false,
                false,
                false,
                440,
            ),
            sample(
                Some("Banque.exe"),
                Some("Comptes"),
                false,
                false,
                false,
                520,
            ),
            sample(
                Some("Banque.exe"),
                Some("Comptes"),
                false,
                false,
                false,
                640,
            ),
        ]
        .into_iter();

        let target = capture_window_app_with(
            Duration::from_secs(15),
            &["voicetotext".into()],
            || samples.next().unwrap(),
            || {},
        )
        .unwrap();

        assert_eq!(target.app.as_deref(), Some("Banque.exe"));
        assert_eq!(target.title.as_deref(), Some("Comptes"));
    }

    #[test]
    fn capture_window_app_resets_stability_on_flicker() {
        let mut samples = vec![
            sample(Some("voicetotext"), None, false, false, false, 0),
            sample(Some("firefox"), None, false, false, false, 40),
            sample(Some("firefox"), None, false, false, false, 160),
            // Bascule avant 180 ms vers Signal.
            sample(Some("signal"), None, false, false, false, 200),
            sample(Some("signal"), None, false, false, false, 390),
        ]
        .into_iter();

        let target = capture_window_app_with(
            Duration::from_secs(15),
            &["voicetotext".into()],
            || samples.next().unwrap(),
            || {},
        )
        .unwrap();

        assert_eq!(target.app.as_deref(), Some("signal"));
    }

    #[test]
    fn capture_window_app_cancels_on_new_escape_press_only() {
        // Échap déjà tenu au départ : ne doit pas annuler tant qu'il n'est pas
        // relâché puis pressé de nouveau.
        let mut samples = vec![
            sample(Some("voicetotext"), None, false, true, false, 0),
            sample(Some("voicetotext"), None, false, true, false, 40),
            sample(Some("voicetotext"), None, false, false, false, 80),
            sample(Some("voicetotext"), None, false, true, false, 120),
        ]
        .into_iter();

        let err = capture_window_app_with(
            Duration::from_secs(15),
            &["voicetotext".into()],
            || samples.next().unwrap(),
            || {},
        )
        .unwrap_err();

        assert!(err.contains("annulée"), "message inattendu : {err}");
    }

    #[test]
    fn capture_window_app_times_out_when_no_other_window_is_selected() {
        let mut samples = vec![
            sample(Some("voicetotext"), None, false, false, false, 0),
            sample(Some("voicetotext"), None, false, false, false, 5000),
            sample(Some("voicetotext"), None, false, false, false, 15000),
        ]
        .into_iter();

        let err = capture_window_app_with(
            Duration::from_secs(15),
            &["voicetotext".into()],
            || samples.next().unwrap(),
            || {},
        )
        .unwrap_err();

        assert!(err.contains("15 secondes"), "message inattendu : {err}");
    }
}
