//! Journal dans un fichier : seul moyen de diagnostiquer l'application
//! résidente, qui n'a pas de console en version publiée.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger, format_description,
};

const LOG_FILE: &str = "voicetotext.log";
/// Au-delà de cette taille, le journal est purgé…
const MAX_LOG_BYTES: u64 = 3 * 1024 * 1024;
/// … en ne gardant que ses dernières lignes.
const KEEP_LINES: usize = 100;

pub fn init(logs_dir: &Path) {
    let path = logs_dir.join(LOG_FILE);

    let mut builder = ConfigBuilder::new();
    let _ = builder.set_time_offset_to_local();
    builder.set_time_format_custom(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second]"
    ));
    let config = builder.build();

    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Info,
        config.clone(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];
    match LogFile::open(path.clone()) {
        Ok(file) => loggers.push(WriteLogger::new(LevelFilter::Info, config, file)),
        Err(e) => eprintln!("ouverture du journal {} : {e}", path.display()),
    }
    let _ = CombinedLogger::init(loggers);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panique : {info}");
        default_hook(info);
    }));
}

/// Fichier journal qui se purge lui-même quand il devient trop gros, y compris
/// pendant que l'application tourne.
struct LogFile {
    path: PathBuf,
    file: File,
    len: u64,
}

impl LogFile {
    fn open(path: PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata()?.len();
        let mut log = Self { path, file, len };
        if log.len > MAX_LOG_BYTES {
            log.purge()?;
        }
        Ok(log)
    }

    fn purge(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let content = fs::read(&self.path)?;
        let kept = last_lines(&content, KEEP_LINES).to_vec();
        fs::write(&self.path, &kept)?;
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.len = kept.len() as u64;
        Ok(())
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.len > MAX_LOG_BYTES {
            // Une purge ratée ne doit pas faire perdre les messages suivants.
            if let Err(e) = self.purge() {
                eprintln!("purge du journal : {e}");
                self.len = 0;
            }
        }
        let written = self.file.write(buf)?;
        self.len += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Les `n` dernières lignes de `content`.
fn last_lines(content: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &[];
    }
    let body = content.strip_suffix(b"\n").unwrap_or(content);
    let start = body
        .iter()
        .enumerate()
        .rev()
        .filter(|&(_, &b)| b == b'\n')
        .nth(n - 1)
        .map_or(0, |(i, _)| i + 1);
    &content[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_lines_keeps_the_end() {
        assert_eq!(last_lines(b"a\nb\nc\n", 2), b"b\nc\n");
        assert_eq!(last_lines(b"a\nb\nc", 2), b"b\nc");
        assert_eq!(last_lines(b"a\nb\n", 5), b"a\nb\n");
        assert_eq!(last_lines(b"a\nb\n", 0), b"");
        assert_eq!(last_lines(b"", 3), b"");
    }

    #[test]
    fn purges_when_too_big() {
        let dir = std::env::temp_dir().join(format!("vtt-log-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LOG_FILE);
        let line = format!("{}\n", "x".repeat(99));
        let lines = (MAX_LOG_BYTES as usize / line.len()) + 10;
        fs::write(&path, line.repeat(lines)).unwrap();

        let mut log = LogFile::open(path.clone()).unwrap();
        assert_eq!(log.len, (KEEP_LINES * line.len()) as u64);
        log.write_all(b"fin\n").unwrap();
        log.flush().unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), KEEP_LINES + 1);
        assert!(content.ends_with("fin\n"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
