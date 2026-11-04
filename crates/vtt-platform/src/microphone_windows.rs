//! Programmes qui utilisent un micro en ce moment (sessions audio actives
//! des périphériques d'enregistrement), pour la détection d'appel.
//!
//! Seuls les noms des programmes sont lus : jamais l'audio, jamais le titre
//! des fenêtres.

use std::collections::HashMap;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eCapture,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::core::Interface;

/// Programmes qui hébergent l'audio d'une autre application (WebView2 :
/// Teams, WhatsApp…) : on remonte à leur parent.
const HOSTS: &[&str] = &["msedgewebview2.exe"];
/// Au plus quelques niveaux de parenté.
const MAX_PARENTS: usize = 4;

/// Processus : nom de l'exécutable (minuscules) et parent.
fn processes() -> HashMap<u32, (String, u32)> {
    let mut found = HashMap::new();
    // SAFETY: instantané parcouru avec une structure de la bonne taille,
    // fermé à la fin.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return found;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            found.insert(entry.th32ProcessID, (name, entry.th32ParentProcessID));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

/// Programme qui a vraiment ouvert le micro (hôte WebView2 → son parent).
fn owner(pid: u32, processes: &HashMap<u32, (String, u32)>) -> Option<String> {
    let mut pid = pid;
    for _ in 0..=MAX_PARENTS {
        let (name, parent) = processes.get(&pid)?;
        if !HOSTS.contains(&name.as_str()) {
            return Some(name.clone());
        }
        pid = *parent;
    }
    None
}

/// PID des sessions actives sur les micros.
fn active_capture_pids() -> windows::core::Result<Vec<u32>> {
    let mut pids = Vec::new();
    // SAFETY: COM initialisé pour ce thread (plusieurs appels sans effet) ;
    // interfaces libérées à la fin de leur portée.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            for j in 0..sessions.GetCount()? {
                let Ok(session) = sessions.GetSession(j) else {
                    continue;
                };
                let Ok(session) = session.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if session.GetState().ok() != Some(AudioSessionStateActive) {
                    continue;
                }
                if let Ok(pid) = session.GetProcessId()
                    && pid != 0
                {
                    pids.push(pid);
                }
            }
        }
    }
    Ok(pids)
}

/// Noms (minuscules) des programmes qui enregistrent un micro en ce moment,
/// hors VoiceToText lui-même.
pub fn microphone_apps() -> Vec<String> {
    let pids = match active_capture_pids() {
        Ok(pids) => pids,
        Err(e) => {
            log::debug!("sessions audio : {e}");
            return Vec::new();
        }
    };
    let own = std::process::id();
    let processes = processes();
    let mut apps: Vec<String> = pids
        .into_iter()
        .filter(|&pid| pid != own)
        .filter_map(|pid| owner(pid, &processes))
        .collect();
    apps.sort();
    apps.dedup();
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview_hosts_resolve_to_their_application() {
        let processes = HashMap::from([
            (10, ("ms-teams.exe".to_owned(), 1)),
            (11, ("msedgewebview2.exe".to_owned(), 10)),
            (12, ("msedgewebview2.exe".to_owned(), 11)),
            (20, ("chrome.exe".to_owned(), 1)),
        ]);
        assert_eq!(owner(12, &processes).as_deref(), Some("ms-teams.exe"));
        assert_eq!(owner(20, &processes).as_deref(), Some("chrome.exe"));
        assert_eq!(owner(99, &processes), None);
    }
}
