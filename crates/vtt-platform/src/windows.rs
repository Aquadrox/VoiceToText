//! Intégration Windows : hook clavier bas niveau et insertion par Ctrl+V.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use vtt_core::{Error, Target, TextSink};
use windows::Win32::Foundation::{CloseHandle, HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_RMENU,
    VK_V, VK_Z,
};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
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

/// Une application occupe l'écran en plein écran (jeu, vidéo, présentation) :
/// Windows suspend alors ses propres notifications.
pub fn fullscreen_app_running() -> bool {
    matches!(
        unsafe { SHQueryUserNotificationState() },
        Ok(state) if state == QUNS_BUSY
            || state == QUNS_RUNNING_D3D_FULL_SCREEN
            || state == QUNS_PRESENTATION_MODE
    )
}

/// Attente maximale de la copie après le Ctrl+C simulé.
const COPY_TIMEOUT: Duration = Duration::from_millis(500);

/// Texte sélectionné dans l'application active (Ctrl+C simulé), ou `None`
/// s'il n'y a pas de sélection. Le contenu texte précédent du presse-papier
/// est remis en place. Les touches de modification doivent être relâchées.
pub fn copy_selection() -> Option<String> {
    let previous = arboard::Clipboard::new().ok()?.get_text().ok();
    let before = unsafe { GetClipboardSequenceNumber() };
    if !send_keys(&[
        (VK_CONTROL, false),
        (VK_C, false),
        (VK_C, true),
        (VK_CONTROL, true),
    ]) {
        return None;
    }
    // Sans sélection, la plupart des applications ne touchent pas au
    // presse-papier : le numéro de séquence ne bouge pas.
    let started = std::time::Instant::now();
    while unsafe { GetClipboardSequenceNumber() } == before {
        if started.elapsed() > COPY_TIMEOUT {
            return None;
        }
        thread::sleep(Duration::from_millis(15));
    }
    // L'application peut écrire plusieurs formats à la suite : laisser finir.
    thread::sleep(Duration::from_millis(30));
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let selection = clipboard.get_text().ok().filter(|s| !s.trim().is_empty());
    let _ = match previous {
        Some(previous) => clipboard.set_text(previous),
        None => clipboard.clear(),
    };
    selection
}

/// Place le texte dans le presse-papier, sans coller.
pub fn copy_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|e| format!("presse-papier : {e}"))
}

/// Joue un son WAV en mémoire sans attendre la fin (le son précédent est
/// interrompu).
pub fn play_sound(wav: &'static [u8]) {
    // SAFETY: avec SND_MEMORY, le pointeur désigne l'image du fichier WAV,
    // qui doit rester valide pendant la lecture asynchrone : `'static`.
    let played = unsafe {
        PlaySoundW(
            windows::core::PCWSTR(wav.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        )
    };
    if !played.as_bool() {
        log::warn!("lecture du son impossible");
    }
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
                "Ctrl+V refusé par Windows (application lancée en administrateur ?) : \
                 texte laissé dans le presse-papier"
                    .into(),
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

    fn undo(&self) -> Result<(), Error> {
        let sent = send_keys(&[
            (VK_CONTROL, false),
            (VK_Z, false),
            (VK_Z, true),
            (VK_CONTROL, true),
        ]);
        if sent {
            Ok(())
        } else {
            Err(Error::Sink(
                "Ctrl+Z refusé par Windows (application lancée en administrateur ?)".into(),
            ))
        }
    }
}

/// Chemin utilisable par une bibliothèque qui ouvre ses fichiers avec des
/// chemins ANSI (Kaldi/Vosk) : nom court 8.3, en ASCII, si le chemin
/// contient des accents (`C:\Users\Frédéric\…`). Inchangé sinon, ou si les
/// noms courts sont désactivés sur le disque.
pub fn ascii_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    use windows::core::PCWSTR;

    if path.to_str().is_some_and(|p| p.is_ascii()) {
        return path.to_owned();
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: chaîne terminée par un zéro, tampon de la taille annoncée.
    let len = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
    if len == 0 || len > buffer.len() {
        return path.to_owned();
    }
    std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len]))
}

/// Clics de la souris (bouton gauche ou droit), envoyés en pixels de l'écran
/// tant que l'enregistreur vit (capture commentée).
static CLICK_SENDER: Mutex<Option<Sender<(i32, i32)>>> = Mutex::new(None);

pub struct ClickRecorder {
    thread_id: u32,
}

impl ClickRecorder {
    /// Installe un hook souris le temps de la capture ; aucun clic n'est bloqué.
    pub fn start(tx: Sender<(i32, i32)>) -> Result<Self, String> {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{UnhookWindowsHookEx, WH_MOUSE_LL};

        *CLICK_SENDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        thread::Builder::new()
            .name("mouse-hook".into())
            .spawn(move || unsafe {
                let module = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
                match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) {
                    Ok(hook) => {
                        let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                        let mut msg = MSG::default();
                        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                        let _ = UnhookWindowsHookEx(hook);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("installation du hook souris : {e}")));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let thread_id = ready_rx
            .recv()
            .map_err(|_| "thread du hook souris arrêté".to_owned())??;
        Ok(Self { thread_id })
    }
}

impl Drop for ClickRecorder {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        *CLICK_SENDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // SAFETY: fin de la boucle de messages du thread du hook.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        LLMHF_INJECTED, MSLLHOOKSTRUCT, WM_LBUTTONDOWN, WM_RBUTTONDOWN,
    };
    if code == HC_ACTION as i32 && matches!(wparam.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
        // SAFETY: pour WH_MOUSE_LL, lparam pointe vers un MSLLHOOKSTRUCT valide.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0
            && let Some(tx) = CLICK_SENDER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
        {
            let _ = tx.send((info.pt.x, info.pt.y));
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Rectangle (pixels de l'écran) de la fenêtre au premier plan, sans les
/// bordures invisibles de Windows 10/11.
pub fn foreground_rect() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    // SAFETY: rectangle de la taille annoncée.
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        let mut rect = RECT::default();
        let framed = DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        );
        if framed.is_err() {
            GetWindowRect(window, &mut rect).ok()?;
        }
        Some((rect.left, rect.top, rect.right, rect.bottom))
    }
}

/// Capture de l'écran principal (pixels physiques), en RGBA. Les fenêtres
/// marquées par [`exclude_from_capture`] (pastille, encre) n'y figurent pas.
pub fn screenshot() -> Result<vtt_core::annotation::Image, String> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ,
        ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    // SAFETY: contextes et bitmap créés ici, libérés avant de rendre la main ;
    // tampon de la taille annoncée à GetDIBits.
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("écran introuvable".into());
        }
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Négatif : lignes de haut en bas.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let lines = if copied.is_ok() {
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        copied.map_err(|e| format!("capture d'écran : {e}"))?;
        if lines != height {
            return Err("capture d'écran incomplète".into());
        }
        // BGRA → RGBA opaque.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(vtt_core::annotation::Image {
            width: width as u32,
            height: height as u32,
            rgba: pixels,
        })
    }
}

/// La fenêtre n'apparaît pas dans les captures d'écran (pastille, encre de
/// la capture commentée). Windows 10 2004 et plus récents.
pub fn exclude_from_capture(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    // SAFETY: simple changement d'attribut d'une fenêtre de l'application.
    if let Err(e) =
        unsafe { SetWindowDisplayAffinity(HWND(hwnd as *mut _), WDA_EXCLUDEFROMCAPTURE) }
    {
        log::warn!("fenêtre non exclue des captures : {e}");
    }
}

/// Colle une image dans l'application active (presse-papier + Ctrl+V).
pub fn paste_image(image: &vtt_core::annotation::Image) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("presse-papier : {e}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: std::borrow::Cow::Borrowed(&image.rgba),
        })
        .map_err(|e| format!("presse-papier : {e}"))?;
    drop(clipboard);
    let pasted = send_keys(&[
        (VK_CONTROL, false),
        (VK_V, false),
        (VK_V, true),
        (VK_CONTROL, true),
    ]);
    if pasted {
        Ok(())
    } else {
        Err("Ctrl+V refusé par Windows : image laissée dans le presse-papier".into())
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

    /// Chaque touche proposée dans les réglages est reconnue.
    #[test]
    fn every_settings_key_is_known() {
        for name in vtt_core::config::HOTKEY_NAMES {
            assert!(key_code(name).is_some(), "{name}");
        }
    }
}
