//! Démarrage avec Windows : valeur `VoiceToText` dans la clé `Run` de
//! l'utilisateur (pas de droits administrateur, désactivable depuis le
//! Gestionnaire des tâches comme n'importe quelle application).

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "VoiceToText";

pub fn autostart_enabled() -> bool {
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    };
    status == ERROR_SUCCESS
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if !enabled {
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(VALUE),
            )
        };
        // Déjà absente : rien à faire.
        return if status == ERROR_SUCCESS || !autostart_enabled() {
            Ok(())
        } else {
            Err(format!("démarrage avec Windows : {status:?}"))
        };
    }
    let exe = std::env::current_exe().map_err(|e| format!("chemin de l'application : {e}"))?;
    // Guillemets : le chemin peut contenir des espaces.
    let command: Vec<u16> = format!("\"{}\"", exe.display())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE),
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("démarrage avec Windows : {status:?}"))
    }
}
