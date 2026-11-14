//! Clavier virtuel du noyau (uinput) pour les frappes simulées sous Wayland.
//!
//! Sous Wayland, XTest n'atteint que les applications X11 (XWayland) : un
//! Ctrl+V simulé n'arriverait pas dans une application Wayland. Un clavier
//! virtuel créé par `/dev/uinput` est vu par le compositeur comme un vrai
//! clavier : ses frappes vont à la fenêtre active, quelle qu'elle soit.
//!
//! Sur SteamOS, Steam a besoin de `/dev/uinput` pour Steam Input : la règle
//! udev de Steam l'ouvre à l'utilisateur de la session. Ailleurs, il faut
//! parfois l'autoriser (groupe `input`).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0;
const BUS_VIRTUAL: u16 = 0x06;
/// Codes de touche déclarés : tout le clavier (`KEY_ESC` à `KEY_MICMUTE`).
const MAX_KEY: i32 = 248;

// ioctl de uinput (`linux/uinput.h`), calculés pour x86_64 / ARM64.
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;

/// Nom du périphérique, visible dans `libinput list-devices`.
const DEVICE_NAME: &[u8] = b"VoiceToText virtual keyboard";

/// Laisse au compositeur le temps de découvrir le nouveau clavier.
const DISCOVERY_DELAY: Duration = Duration::from_millis(400);
/// Pause entre deux frappes : certaines applications ratent des frappes
/// trop rapprochées.
const KEY_DELAY: Duration = Duration::from_millis(8);

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

/// Clavier créé au premier usage, gardé jusqu'à la fin du programme.
static KEYBOARD: Mutex<Option<File>> = Mutex::new(None);

fn create() -> Result<File, String> {
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/uinput")
        .map_err(|e| format!("/dev/uinput : {e}"))?;
    let fd = file.as_raw_fd();
    let mut setup = UinputSetup {
        id: InputId {
            bustype: BUS_VIRTUAL,
            vendor: 0,
            product: 0,
            version: 1,
        },
        name: [0; 80],
        ff_effects_max: 0,
    };
    setup.name[..DEVICE_NAME.len()].copy_from_slice(DEVICE_NAME);
    // SAFETY: ioctl documentés de uinput, sur un descripteur ouvert ; la
    // structure a la disposition mémoire de `struct uinput_setup`.
    unsafe {
        check(libc::ioctl(fd, UI_SET_EVBIT, libc::c_int::from(EV_KEY)))?;
        for code in 1..=MAX_KEY {
            check(libc::ioctl(fd, UI_SET_KEYBIT, code))?;
        }
        check(libc::ioctl(fd, UI_DEV_SETUP, &setup as *const UinputSetup))?;
        check(libc::ioctl(fd, UI_DEV_CREATE))?;
    }
    log::info!("clavier virtuel créé (uinput) pour les frappes simulées");
    thread::sleep(DISCOVERY_DELAY);
    Ok(file)
}

fn check(result: libc::c_int) -> Result<(), String> {
    if result < 0 {
        Err(format!("uinput : {}", std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    // struct input_event : timeval (ignoré à l'écriture), type, code, value.
    let mut bytes = [0u8; 24];
    bytes[16..18].copy_from_slice(&kind.to_ne_bytes());
    bytes[18..20].copy_from_slice(&code.to_ne_bytes());
    bytes[20..24].copy_from_slice(&value.to_ne_bytes());
    bytes
}

/// Appuie sur `keys` (codes evdev) dans l'ordre, puis les relâche dans
/// l'ordre inverse : `[Ctrl, V]` → Ctrl+V.
pub fn send_chord(keys: &[u32]) -> Result<(), String> {
    let mut keyboard = KEYBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if keyboard.is_none() {
        *keyboard = Some(create()?);
    }
    let file = keyboard.as_mut().expect("clavier virtuel créé");
    let mut press = |code: u32, value: i32| -> Result<(), String> {
        let code = u16::try_from(code).map_err(|_| format!("code de touche {code} invalide"))?;
        file.write_all(&event(EV_KEY, code, value))
            .and_then(|()| file.write_all(&event(EV_SYN, SYN_REPORT, 0)))
            .map_err(|e| format!("uinput : {e}"))?;
        thread::sleep(KEY_DELAY);
        Ok(())
    };
    for &key in keys {
        press(key, 1)?;
    }
    for &key in keys.iter().rev() {
        press(key, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_layout() {
        let bytes = event(EV_KEY, 47, 1);
        assert_eq!(&bytes[..16], &[0u8; 16]);
        assert_eq!(u16::from_ne_bytes([bytes[16], bytes[17]]), EV_KEY);
        assert_eq!(u16::from_ne_bytes([bytes[18], bytes[19]]), 47);
        assert_eq!(i32::from_ne_bytes(bytes[20..24].try_into().unwrap()), 1);
    }

    /// Les ioctl sont ceux de `linux/uinput.h` (taille de `uinput_setup` : 92).
    #[test]
    fn setup_size() {
        assert_eq!(size_of::<UinputSetup>(), 92);
        assert_eq!((UI_DEV_SETUP >> 16) & 0x3fff, 92);
    }
}
