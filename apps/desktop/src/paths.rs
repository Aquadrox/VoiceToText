//! Emplacements des fichiers de l'application.
//!
//! Windows : configuration dans `%APPDATA%\VoiceToText`, modèles,
//! journaux, historique et réunions dans `%LOCALAPPDATA%\VoiceToText` (non synchronisé
//! avec le profil itinérant).

use std::fs;
use std::io;
use std::path::PathBuf;

use vtt_core::Config;

const APP_DIR: &str = "VoiceToText";

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub models_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub history_file: PathBuf,
    pub usage_file: PathBuf,
    /// Une réunion par dossier (transcription, compte rendu, audio chiffré).
    pub meetings_dir: PathBuf,
    /// Captures commentées gardées (si l'utilisateur le demande).
    pub captures_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR);
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR);
        Self {
            config_file: config_dir.join("config.toml"),
            models_dir: data_dir.join("models"),
            logs_dir: data_dir.join("logs"),
            history_file: data_dir.join("history.jsonl"),
            usage_file: data_dir.join("usage.json"),
            meetings_dir: data_dir.join("meetings"),
            captures_dir: data_dir.join("captures"),
        }
    }

    pub fn create_dirs(&self) -> io::Result<()> {
        if let Some(dir) = self.config_file.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::create_dir_all(&self.models_dir)?;
        fs::create_dir_all(&self.meetings_dir)?;
        fs::create_dir_all(&self.logs_dir)
    }

    /// Lit la configuration ; la crée avec les valeurs par défaut si absente.
    ///
    /// En cas de fichier invalide, renvoie les valeurs par défaut et le
    /// message d'erreur à afficher.
    pub fn load_config(&self) -> (Config, Option<String>) {
        match fs::read_to_string(&self.config_file) {
            Ok(source) => match Config::from_toml(&source) {
                Ok(mut config) => {
                    let changes = config.migrate();
                    if !changes.is_empty() {
                        log::info!("configuration mise à jour : {}", changes.join(", "));
                        self.save_config(&config);
                    }
                    (config, None)
                }
                Err(e) => {
                    log::error!("{} invalide : {e}", self.config_file.display());
                    let message = "config.toml invalide : valeurs par défaut utilisées";
                    (Config::default(), Some(message.into()))
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let config = Config::default();
                self.save_config(&config);
                (config, None)
            }
            Err(e) => {
                log::error!("lecture de {} : {e}", self.config_file.display());
                (
                    Config::default(),
                    Some(format!("lecture de config.toml : {e}")),
                )
            }
        }
    }

    fn save_config(&self, config: &Config) {
        if let Err(e) = self.write_config(config) {
            log::error!("{e}");
        }
    }

    /// Réécrit config.toml (les commentaires ajoutés à la main sont perdus).
    pub fn write_config(&self, config: &Config) -> Result<(), String> {
        let toml = config
            .to_toml()
            .map_err(|e| format!("configuration non enregistrable : {e}"))?;
        fs::write(&self.config_file, toml)
            .map_err(|e| format!("écriture de la configuration : {e}"))?;
        log::info!("configuration écrite : {}", self.config_file.display());
        Ok(())
    }
}
