//! Modèles de transcription proposés sur le téléphone et leur téléchargement
//! (vérifié : taille et SHA-256). Whisper tient en un fichier ; Parakeet et
//! Vosk en un dossier. Leurs bibliothèques natives (ONNX Runtime, libvosk)
//! sont dans l'APK : Android n'autorise que le code livré avec l'application.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Whisper,
    Parakeet,
    Vosk,
}

/// Fichier distant, vérifié par sa taille et son SHA-256.
struct Remote {
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

enum Source {
    /// Whisper : un fichier, nommé comme le modèle.
    File(Remote),
    /// Dossier nommé comme le modèle : fichiers enregistrés sous ces noms.
    Files(&'static [(&'static str, Remote)]),
    /// Dossier nommé comme le modèle : archive zip extraite sans `prefix`.
    Zip {
        remote: Remote,
        prefix: &'static str,
    },
}

impl Source {
    fn size(&self) -> u64 {
        match self {
            Source::File(remote) | Source::Zip { remote, .. } => remote.size,
            Source::Files(files) => files.iter().map(|(_, r)| r.size).sum(),
        }
    }
}

pub struct KnownModel {
    pub id: &'static str,
    pub label: &'static str,
    pub engine: Engine,
    source: Source,
}

/// Fichier écrit en dernier dans un dossier téléchargé : sans lui, le
/// dossier est incomplet (téléchargement interrompu) et refait.
const COMPLETE: &str = ".complete";

/// Fichiers de Parakeet, à une révision figée du dépôt Hugging Face (comme
/// sur PC).
macro_rules! parakeet_url {
    ($file:literal) => {
        concat!(
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/",
            "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce/",
            $file
        )
    };
}

/// Tailles et empreintes relevées sur Hugging Face et alphacephei.com.
pub const MODELS: &[KnownModel] = &[
    KnownModel {
        id: "parakeet-tdt-0.6b-v3-int8",
        label: "Parakeet TDT v3, 8 bits (recommandé : le plus rapide et le plus fiable, ponctué ; ~1 Go de mémoire)",
        engine: Engine::Parakeet,
        source: Source::Files(&[
            (
                "encoder-model.int8.onnx",
                Remote {
                    url: parakeet_url!("encoder-model.int8.onnx"),
                    size: 652_183_999,
                    sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
                },
            ),
            (
                "decoder_joint-model.int8.onnx",
                Remote {
                    url: parakeet_url!("decoder_joint-model.int8.onnx"),
                    size: 18_202_004,
                    sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
                },
            ),
            (
                "vocab.txt",
                Remote {
                    url: parakeet_url!("vocab.txt"),
                    size: 93_939,
                    sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                },
            ),
        ]),
    },
    KnownModel {
        id: "ggml-small-q8_0.bin",
        label: "Whisper Small, 8 bits (rapide et précis, plus léger que Parakeet)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q8_0.bin",
            size: 264_464_607,
            sha256: "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
        }),
    },
    KnownModel {
        id: "ggml-base-q8_0.bin",
        label: "Whisper Base, 8 bits (très léger, moins précis)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q8_0.bin",
            size: 81_768_585,
            sha256: "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
        }),
    },
    KnownModel {
        id: "ggml-large-v3-turbo-q5_0.bin",
        label: "Whisper Large v3 Turbo, 5 bits (le plus précis, plus lent)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
            size: 574_041_195,
            sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        }),
    },
    KnownModel {
        id: "vosk-model-small-fr-0.22",
        label: "Vosk Small, français (ultra léger et instantané ; sans ponctuation, que le LLM rajoute)",
        engine: Engine::Vosk,
        source: Source::Zip {
            remote: Remote {
                url: "https://alphacephei.com/vosk/models/vosk-model-small-fr-0.22.zip",
                size: 42_233_323,
                sha256: "cabf6180e177eb9b3a9a9d43a437bd5e549f3a7d09525e5d69a3fed787be12ad",
            },
            prefix: "vosk-model-small-fr-0.22/",
        },
    },
];

/// Modèle par défaut sur le téléphone : Parakeet, le plus rapide et le
/// plus fiable aux essais.
pub const DEFAULT_MODEL: &str = "parakeet-tdt-0.6b-v3-int8";

pub fn known(id: &str) -> Option<&'static KnownModel> {
    MODELS.iter().find(|m| m.id == id)
}

#[derive(Serialize)]
pub struct ModelChoice {
    id: &'static str,
    label: &'static str,
    size_mb: u64,
    downloaded: bool,
}

pub fn catalog(dir: &Path) -> Vec<ModelChoice> {
    MODELS
        .iter()
        .map(|m| ModelChoice {
            id: m.id,
            label: m.label,
            size_mb: m.source.size() / 1_000_000,
            downloaded: local(dir, m.id).is_some(),
        })
        .collect()
}

/// Chemin du modèle (fichier ou dossier) s'il est présent et complet.
pub fn local(dir: &Path, id: &str) -> Option<PathBuf> {
    let model = known(id)?;
    let path = dir.join(id);
    let ready = match &model.source {
        Source::File(remote) => fs::metadata(&path).is_ok_and(|m| m.len() == remote.size),
        Source::Files(_) | Source::Zip { .. } => path.join(COMPLETE).is_file(),
    };
    ready.then_some(path)
}

/// Progression du téléchargement en cours (0–100), lue par l'interface.
pub static PROGRESS: AtomicU8 = AtomicU8::new(0);

struct Progress {
    total: u64,
    done: u64,
}

impl Progress {
    fn add(&mut self, n: u64) {
        self.done += n;
        PROGRESS.store(
            (self.done * 100 / self.total.max(1)).min(100) as u8,
            Ordering::Relaxed,
        );
    }
}

/// Télécharge le modèle s'il manque ; vérifie taille et SHA-256.
pub fn download(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if let Some(path) = local(dir, id) {
        return Ok(path);
    }
    let model = known(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(id);
    PROGRESS.store(0, Ordering::Relaxed);
    let mut progress = Progress {
        total: model.source.size(),
        done: 0,
    };
    match &model.source {
        Source::File(remote) => {
            let partial = path.with_extension("part");
            fetch(remote, &partial, &mut progress)?;
            fs::rename(&partial, &path).map_err(|e| format!("installation du modèle : {e}"))?;
        }
        source => install_dir(&path, source, &mut progress)?,
    }
    log::info!("modèle installé : {}", path.display());
    local(dir, id).ok_or_else(|| format!("modèle {id} incomplet après téléchargement"))
}

/// Remplit un dossier temporaire puis le renomme en `dest` une fois tout
/// vérifié : un dossier `dest` complet, ou rien.
fn install_dir(dest: &Path, source: &Source, progress: &mut Progress) -> Result<(), String> {
    let partial = dest.with_extension("part");
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let result = fill_dir(&partial, source, progress).and_then(|()| {
        let _ = fs::remove_dir_all(dest);
        fs::rename(&partial, dest).map_err(|e| format!("installation de {} : {e}", dest.display()))
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    result
}

fn fill_dir(dir: &Path, source: &Source, progress: &mut Progress) -> Result<(), String> {
    match source {
        Source::File(_) => unreachable!("un modèle en un fichier n'a pas de dossier"),
        Source::Files(files) => {
            for (name, remote) in files.iter() {
                fetch(remote, &dir.join(name), progress)?;
            }
        }
        Source::Zip { remote, prefix } => {
            let archive = dir.join(".archive");
            fetch(remote, &archive, progress)?;
            extract(&archive, dir, prefix)?;
            fs::remove_file(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
        }
    }
    fs::write(dir.join(COMPLETE), b"").map_err(|e| format!("{}: {e}", dir.display()))
}

fn fetch(remote: &Remote, path: &Path, progress: &mut Progress) -> Result<(), String> {
    log::info!("téléchargement de {}", remote.url);
    let mut response = ureq::get(remote.url)
        .call()
        .map_err(|e| format!("téléchargement : {e}"))?;
    // La taille attendue est connue : rien de plus n'est lu.
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(remote.size + 1)
        .reader();
    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| format!("téléchargement interrompu : {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        writer
            .write_all(&buffer[..n])
            .map_err(|e| format!("écriture du modèle : {e}"))?;
        received += n as u64;
        progress.add(n as u64);
    }
    writer
        .flush()
        .map_err(|e| format!("écriture du modèle : {e}"))?;
    drop(writer);
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if received != remote.size || digest != remote.sha256 {
        let _ = fs::remove_file(path);
        return Err(format!(
            "modèle corrompu ({received} octets) : réessayez le téléchargement"
        ));
    }
    Ok(())
}

/// Chemin relatif sûr dans l'archive, sans `prefix` ; `None` : entrée
/// ignorée (hors du préfixe, ou chemin suspect).
fn archive_target(name: &Path, prefix: &str) -> Option<PathBuf> {
    let relative = name.strip_prefix(prefix).ok()?;
    let safe = relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    (safe && !relative.as_os_str().is_empty()).then(|| relative.to_owned())
}

/// Extrait les fichiers ordinaires du zip ; liens et dossiers ignorés.
fn extract(archive: &Path, dest: &Path, prefix: &str) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("archive : {e}"))?;
    let mut extracted = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("archive : {e}"))?;
        if !entry.is_file() {
            continue;
        }
        let Some(target) = entry
            .enclosed_name()
            .and_then(|name| archive_target(&name, prefix))
        else {
            continue;
        };
        let target = dest.join(target);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut out = File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        io::copy(&mut entry, &mut out)
            .map_err(|e| format!("extraction de {} : {e}", target.display()))?;
        extracted += 1;
    }
    if extracted == 0 {
        return Err("archive du modèle vide".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtt-android-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_complete_known_models_are_local() {
        let dir = temp("models");
        let whisper = "ggml-small-q8_0.bin";
        assert_eq!(local(&dir, whisper), None);
        fs::write(dir.join(whisper), b"partiel").unwrap();
        assert_eq!(local(&dir, whisper), None, "taille incorrecte");
        assert_eq!(local(&dir, "perso.bin"), None, "hors catalogue");
        assert!(known(DEFAULT_MODEL).is_some());
        assert!(catalog(&dir).iter().all(|m| !m.downloaded));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_models_need_their_marker() {
        let dir = temp("folders");
        for id in ["parakeet-tdt-0.6b-v3-int8", "vosk-model-small-fr-0.22"] {
            fs::create_dir_all(dir.join(id)).unwrap();
            assert_eq!(local(&dir, id), None, "{id} : dossier interrompu");
            fs::write(dir.join(id).join(COMPLETE), b"").unwrap();
            assert_eq!(local(&dir, id), Some(dir.join(id)));
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn archive_entries_stay_inside_the_model() {
        let prefix = "vosk-model-small-fr-0.22/";
        assert_eq!(
            archive_target(Path::new("vosk-model-small-fr-0.22/am/final.mdl"), prefix),
            Some(PathBuf::from("am/final.mdl"))
        );
        assert_eq!(archive_target(Path::new("autre/final.mdl"), prefix), None);
        assert_eq!(
            archive_target(Path::new("vosk-model-small-fr-0.22/../x"), prefix),
            None
        );
        assert_eq!(
            archive_target(Path::new("vosk-model-small-fr-0.22/"), prefix),
            None
        );
    }

    #[test]
    fn engines_match_the_catalog() {
        assert_eq!(known(DEFAULT_MODEL).unwrap().engine, Engine::Parakeet);
        assert_eq!(
            known("ggml-small-q8_0.bin").unwrap().engine,
            Engine::Whisper
        );
        assert_eq!(
            known("parakeet-tdt-0.6b-v3-int8").unwrap().engine,
            Engine::Parakeet
        );
        assert_eq!(
            known("vosk-model-small-fr-0.22").unwrap().engine,
            Engine::Vosk
        );
    }
}
