//! Intégration Windows : hook clavier bas niveau et insertion par Ctrl+V.

use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use vtt_core::{Error, Target, TextSink};
use windows::Win32::Foundation::{CloseHandle, HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_RMENU, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowTextW, GetWindowThreadProcessId,
    HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, SetWindowsHookExW, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_SYSKEYDOWN,
};

use crate::{KeyEvent, now_ms};

/// Touche virtuelle non attribuée, utilisée pour « masquer » un appui sur Alt.
const VK_MENU_MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

/// Délai avant de remettre l'ancien contenu du presse-papier : l'application
/// cible lit le presse-papier de façon asynchrone après le Ctrl+V.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(600);

static HOOK_SENDER: OnceLock<Sender<KeyEvent>> = OnceLock::new();

/// Nom de touche de la configuration → code virtual-key.
pub fn key_code(name: &str) -> Option<u32> {
    let name = name.trim().to_ascii_lowercase();
    let code = match name.as_str() {
        "altgr" | "rightalt" | "ralt" => VK_RMENU.0,
        "rightctrl" | "rctrl" => 0xA3,
        "rightshift" | "rshift" => 0xA1,
        "capslock" => 0x14,
        "scrolllock" => 0x91,
        "pause" => 0x13,
        "insert" => 0x2D,
        "menu" | "apps" => 0x5D,
        "escape" | "esc" => 0x1B,
        _ => {
            let n: u16 = name.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            0x70 + n - 1
        }
    };
    Some(code as u32)
}

/// Touches que Windows émet en même temps que `trigger` et qu'il faut ignorer.
///
/// AltGr est transmis comme « Ctrl gauche + Alt droit » : sans ce filtre, le
/// faux Ctrl (répété tant qu'AltGr est maintenu) annulerait la dictée.
pub fn companion_keys(trigger: u32) -> Vec<u32> {
    if trigger == VK_RMENU.0 as u32 {
        vec![VK_LCONTROL.0 as u32]
    } else {
        Vec::new()
    }
}

/// Installe le hook clavier global dans un thread dédié avec sa boucle de messages.
///
/// Le hook n'avale aucune touche : il se contente de recopier les événements.
pub fn spawn_keyboard_hook(tx: Sender<KeyEvent>) -> Result<(), String> {
    HOOK_SENDER
        .set(tx)
        .map_err(|_| "hook clavier déjà installé".to_owned())?;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) {
                Ok(_) => {
                    let _ = ready_tx.send(Ok(()));
                    let mut msg = MSG::default();
                    // GetMessageW renvoie -1 en cas d'erreur : ne pas boucler dessus.
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("installation du hook clavier : {e}")));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    ready_rx
        .recv()
        .map_err(|_| "thread du hook arrêté".to_owned())?
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: pour WH_KEYBOARD_LL, lparam pointe vers un KBDLLHOOKSTRUCT valide.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Nos propres Ctrl+V simulés ne doivent pas perturber le raccourci.
        if info.flags.0 & LLKHF_INJECTED.0 == 0
            && let Some(tx) = HOOK_SENDER.get()
        {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let _ = tx.send(KeyEvent {
                key: info.vkCode,
                down,
                at_ms: now_ms(),
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// État physique de la touche, indépendamment du hook.
pub fn key_is_down(key: u32) -> bool {
    unsafe { GetAsyncKeyState(key as i32) as u16 & 0x8000 != 0 }
}

/// Une touche de modification (Ctrl, Maj, Alt, Windows) est enfoncée.
///
/// Un Ctrl+V simulé pendant que l'utilisateur tient encore Alt+Maj (son
/// raccourci) deviendrait Ctrl+Alt+Maj+V : il faut attendre le relâchement.
pub fn modifiers_down() -> bool {
    const MODIFIERS: [u32; 5] = [0x10, 0x11, 0x12, 0x5B, 0x5C];
    MODIFIERS.iter().any(|&key| key_is_down(key))
}

/// Un appui seul sur Alt active la barre de menus de l'application au
/// relâchement. Une frappe neutre pendant l'appui l'en empêche (technique
/// d'AutoHotkey).
pub fn suppress_menu_activation(trigger: u32) {
    if trigger == VK_RMENU.0 as u32 || trigger == VK_LMENU.0 as u32 {
        send_keys(&[(VK_MENU_MASK, false), (VK_MENU_MASK, true)]);
    }
}

fn send_keys(keys: &[(VIRTUAL_KEY, bool)]) -> bool {
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&(vk, up)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
        .collect();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// Fenêtre au premier plan : exécutable et titre. Champs vides si Windows
/// refuse l'accès (processus d'un autre utilisateur ou protégé).
pub fn foreground_target() -> Target {
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return Target::default();
    }
    let mut title = [0u16; 512];
    let len = unsafe { GetWindowTextW(window, &mut title) };
    let title = (len > 0).then(|| String::from_utf16_lossy(&title[..len as usize]));

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    Target {
        app: process_name(pid),
        title,
    }
}

/// Nom de l'exécutable d'un processus, ex. "slack.exe".
fn process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut path = [0u16; 1024];
    let mut len = path.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    queried.ok()?;
    let path = String::from_utf16_lossy(&path[..len as usize]);
    path.rsplit('\\').next().map(str::to_owned)
}

/// Place le texte dans le presse-papier, sans coller.
pub fn copy_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|e| format!("presse-papier : {e}"))
}

/// Insère le texte via le presse-papier et un Ctrl+V simulé, puis remet
/// l'ancien contenu texte du presse-papier.
pub struct ClipboardPaster;

impl TextSink for ClipboardPaster {
    fn insert(&self, text: &str) -> Result<(), Error> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| Error::Sink(format!("presse-papier : {e}")))?;
        let previous = clipboard.get_text().ok();
        clipboard
            .set_text(text)
            .map_err(|e| Error::Sink(format!("presse-papier : {e}")))?;
        drop(clipboard);

        let pasted = send_keys(&[
            (VK_CONTROL, false),
            (VK_V, false),
            (VK_V, true),
            (VK_CONTROL, true),
        ]);
        if !pasted {
            // Le texte reste dans le presse-papier : l'utilisateur peut coller à la main.
            return Err(Error::Sink(
                "Ctrl+V refusé par Windows (application lancée en administrateur ?)".into(),
            ));
        }

        if let Some(previous) = previous {
            let inserted = text.to_owned();
            thread::spawn(move || {
                thread::sleep(CLIPBOARD_RESTORE_DELAY);
                let Ok(mut clipboard) = arboard::Clipboard::new() else {
                    return;
                };
                // Ne pas écraser ce que l'utilisateur aurait copié entre-temps.
                if clipboard.get_text().ok().as_deref() == Some(inserted.as_str()) {
                    let _ = clipboard.set_text(previous);
                }
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_code("AltGr"), Some(0xA5));
        assert_eq!(key_code(" altgr "), Some(0xA5));
        assert_eq!(key_code("F13"), Some(0x7C));
        assert_eq!(key_code("F25"), None);
        assert_eq!(key_code("nimportequoi"), None);
    }
}
