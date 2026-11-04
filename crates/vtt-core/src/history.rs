//! Historique des dictées, conservé localement dans un fichier JSON Lines
//! (une dictée par ligne, la plus récente en dernier).
//!
//! Le fichier reste sur le PC : il n'est jamais envoyé nulle part. Une ligne
//! illisible (fichier tronqué, ancienne version) est ignorée, pas fatale.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::usage::Usage;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Horodatage Unix en millisecondes.
    pub at_ms: u64,
    /// Durée de l'enregistrement.
    pub duration_ms: u64,
    /// Transcription Whisper.
    pub raw: String,
    /// Texte inséré.
    pub text: String,
    #[serde(default)]
    pub snippet: bool,
    /// Le nettoyage a échoué : le texte brut a été inséré.
    #[serde(default)]
    pub cleanup_failed: bool,
    /// Command Mode : `raw` est la consigne, `text` le résultat.
    #[serde(default)]
    pub command: bool,
    /// Application de destination, ex. "slack.exe".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Jetons facturés par le LLM pour cette dictée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Coût estimé au tarif en vigueur au moment de la dictée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Profil de secours qui a nettoyé le texte à la place du principal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

pub struct History {
    path: PathBuf,
    max_entries: usize,
    /// Nombre de lignes du fichier, pour savoir quand l'élaguer sans le relire.
    lines: usize,
}

impl History {
    pub fn open(path: PathBuf, max_entries: usize) -> Self {
        let lines = fs::read_to_string(&path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        Self {
            path,
            max_entries: max_entries.max(1),
            lines,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&mut self, entry: &Entry) -> io::Result<()> {
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(line.as_bytes())?;
        self.lines += 1;
        // Élaguer par paquets de 10 % évite de réécrire le fichier à chaque dictée.
        if self.lines > self.max_entries + self.max_entries / 10 {
            self.prune()?;
        }
        Ok(())
    }

    /// Les `limit` dictées les plus récentes, la plus récente en premier.
    pub fn recent(&self, limit: usize) -> io::Result<Vec<Entry>> {
        let mut entries = self.read_all()?;
        entries.reverse();
        entries.truncate(limit);
        Ok(entries)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        self.lines = 0;
        Ok(())
    }

    fn read_all(&self) -> io::Result<Vec<Entry>> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    /// Ne garde que les `max_entries` dernières dictées. Écriture dans un
    /// fichier temporaire puis renommage : une coupure ne corrompt rien.
    fn prune(&mut self) -> io::Result<()> {
        let entries = self.read_all()?;
        let kept = &entries[entries.len().saturating_sub(self.max_entries)..];
        let mut content = String::new();
        for entry in kept {
            content.push_str(&serde_json::to_string(entry).map_err(io::Error::other)?);
            content.push('\n');
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &self.path)?;
        self.lines = kept.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(i: u64) -> Entry {
        Entry {
            at_ms: i,
            duration_ms: 1000,
            raw: format!("brut {i}"),
            text: format!("propre {i}"),
            snippet: false,
            cleanup_failed: false,
            command: false,
            app: None,
            usage: None,
            cost: None,
            currency: None,
            fallback: None,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtt-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("history.jsonl")
    }

    #[test]
    fn append_then_recent_newest_first() {
        let path = temp_path("recent");
        let mut history = History::open(path.clone(), 100);
        for i in 0..3 {
            history.append(&entry(i)).unwrap();
        }
        let recent = History::open(path, 100).recent(2).unwrap();
        assert_eq!(recent, [entry(2), entry(1)]);
    }

    #[test]
    fn prune_keeps_last_entries() {
        let path = temp_path("prune");
        let mut history = History::open(path.clone(), 10);
        for i in 0..25 {
            history.append(&entry(i)).unwrap();
        }
        let all = history.recent(usize::MAX).unwrap();
        assert!(all.len() <= 11, "{} entrées", all.len());
        assert_eq!(all[0], entry(24));
        assert!(!path.with_extension("jsonl.tmp").exists());
    }

    #[test]
    fn unreadable_lines_are_skipped() {
        let path = temp_path("corrupt");
        let good = serde_json::to_string(&entry(1)).unwrap();
        fs::write(&path, format!("{good}\n{{tronqué\n")).unwrap();
        let history = History::open(path, 10);
        assert_eq!(history.recent(10).unwrap(), [entry(1)]);
    }

    #[test]
    fn clear_removes_everything() {
        let path = temp_path("clear");
        let mut history = History::open(path.clone(), 10);
        history.append(&entry(1)).unwrap();
        history.clear().unwrap();
        history.clear().unwrap();
        assert!(history.recent(10).unwrap().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn missing_file_is_empty() {
        let history = History::open(temp_path("missing"), 10);
        assert!(history.recent(10).unwrap().is_empty());
    }
}
