//! Démarrage avec la session : fichier `.desktop` dans
//! `~/.config/autostart` (norme freedesktop, suivie par KDE, GNOME, XFCE…).

use std::path::PathBuf;

const FILE_NAME: &str = "voicetotext.desktop";

fn autostart_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("autostart").join(FILE_NAME))
}

/// Programme à relancer. Une AppImage s'exécute depuis un point de montage
/// temporaire (`/tmp/.mount_…`) : c'est le fichier `.AppImage` lui-même,
/// donné par la variable `APPIMAGE`, qu'il faut lancer.
fn executable() -> Result<PathBuf, String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(appimage));
    }
    std::env::current_exe().map_err(|e| format!("chemin du programme : {e}"))
}

/// Argument de la clé `Exec` : entre guillemets, avec les caractères
/// réservés échappés (spécification Desktop Entry).
fn exec_argument(path: &str) -> String {
    let mut quoted = String::from('"');
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            // `%` introduit un code de champ (%f, %u…).
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=VoiceToText\n\
         Comment=Dictée vocale\n\
         Exec={}\n\
         Icon=voicetotext\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_argument(exec)
    )
}

pub fn autostart_enabled() -> bool {
    autostart_file().is_some_and(|file| {
        std::fs::read_to_string(file).is_ok_and(|content| !content.contains("Hidden=true"))
    })
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let file = autostart_file().ok_or("dossier personnel introuvable")?;
    if !enabled {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("suppression de {} : {e}", file.display()))
            }
            _ => Ok(()),
        };
    }
    let exec = executable()?;
    let exec = exec
        .to_str()
        .ok_or("chemin du programme illisible (caractères non UTF-8)")?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("création de {} : {e}", dir.display()))?;
    }
    std::fs::write(&file, desktop_entry(exec))
        .map_err(|e| format!("écriture de {} : {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_is_quoted_and_escaped() {
        assert_eq!(
            exec_argument("/home/deck/Applications/VoiceToText.AppImage"),
            "\"/home/deck/Applications/VoiceToText.AppImage\""
        );
        assert_eq!(
            exec_argument("/home/a b/$x\"%.AppImage"),
            "\"/home/a b/\\$x\\\"%%.AppImage\""
        );
    }

    #[test]
    fn entry_has_exec_line() {
        let entry = desktop_entry("/opt/vtt");
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/opt/vtt\"\n"));
    }
}
