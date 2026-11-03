//! Bouche-trou pour les OS pas encore pris en charge : le projet compile et
//! se teste partout, seules ces fonctions échouent à l'exécution.

use std::sync::mpsc::Sender;

use vtt_core::{Error, TextSink};

use crate::KeyEvent;

const MESSAGE: &str = "pas encore pris en charge sur cet OS";

pub fn key_code(_name: &str) -> Option<u32> {
    None
}

pub fn companion_keys(_trigger: u32) -> Vec<u32> {
    Vec::new()
}

pub fn spawn_keyboard_hook(_tx: Sender<KeyEvent>) -> Result<(), String> {
    Err(format!("hook clavier {MESSAGE}"))
}

pub fn key_is_down(_key: u32) -> bool {
    false
}

pub fn suppress_menu_activation(_trigger: u32) {}

pub fn foreground_target() -> vtt_core::Target {
    vtt_core::Target::default()
}

pub fn autostart_enabled() -> bool {
    false
}

pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err(format!("démarrage automatique {MESSAGE}"))
}

pub fn copy_selection() -> Option<String> {
    None
}

pub fn modifiers_down() -> bool {
    false
}

pub fn copy_text(_text: &str) -> Result<(), String> {
    Err(format!("presse-papier {MESSAGE}"))
}

pub struct ClipboardPaster;

impl TextSink for ClipboardPaster {
    fn insert(&self, _text: &str) -> Result<(), Error> {
        Err(Error::Sink(format!("insertion {MESSAGE}")))
    }
}

pub fn store_secret(_name: &str, _value: &str) -> Result<(), String> {
    Err(format!("stockage des secrets {MESSAGE}"))
}

pub fn load_secret(_name: &str) -> Option<String> {
    None
}

pub fn delete_secret(_name: &str) -> Result<(), String> {
    Err(format!("stockage des secrets {MESSAGE}"))
}
