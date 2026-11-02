//! Journal dans un fichier : seul moyen de diagnostiquer l'application
//! résidente, qui n'a pas de console en version publiée.

use std::fs::{self, OpenOptions};
use std::path::Path;

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};

const LOG_FILE: &str = "voicetotext.log";
/// Au-delà, le journal est renommé en `.old` au démarrage.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn init(logs_dir: &Path) {
    let path = logs_dir.join(LOG_FILE);
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }

    let mut builder = ConfigBuilder::new();
    let _ = builder.set_time_offset_to_local();
    let config = builder.build();

    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Info,
        config.clone(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];
    match OpenOptions::new().create(true).append(true).open(&path) {
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
