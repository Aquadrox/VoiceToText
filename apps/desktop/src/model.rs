//! Modèles de transcription proposés (Whisper, Parakeet, Vosk) et leur
//! téléchargement (au premier lancement ou après un changement de modèle
//! dans les réglages).
//!
//! Whisper : un fichier. Parakeet et Vosk : un dossier (fichiers ou archive
//! extraite), plus une bibliothèque native téléchargée à part (ONNX Runtime,
//! libvosk) : l'installeur n'embarque rien pour ceux qui ne s'en servent pas.
//! Tout est vérifié (taille et SHA-256) avant d'être utilisé.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use vtt_asr::{ParakeetTranscriber, VoskTranscriber, WhisperTranscriber};
use vtt_core::Transcriber;
use vtt_core::config::AsrConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
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

/// Contenu d'un dossier téléchargé.
enum Content {
    /// Fichiers enregistrés sous ces noms.
    Files(&'static [(&'static str, Remote)]),
    /// Archive (.zip ou .tgz) : fichiers placés sous `prefix`, tous ou
    /// seulement ceux de `only`, extraits sans ce préfixe.
    Archive {
        remote: Remote,
        prefix: &'static str,
        only: &'static [&'static str],
    },
}

impl Content {
    fn size(&self) -> u64 {
        match self {
            Content::Files(files) => files.iter().map(|(_, r)| r.size).sum(),
            Content::Archive { remote, .. } => remote.size,
        }
    }
}

/// Bibliothèque native d'un moteur, dans son propre dossier.
struct Runtime {
    dir: &'static str,
    content: Content,
    /// Fichier à charger, dans le dossier.
    library: &'static str,
}

enum Source {
    /// Whisper : un fichier, nommé comme le modèle.
    File(Remote),
    /// Dossier nommé comme le modèle.
    Dir(Content),
}

impl Source {
    fn size(&self) -> u64 {
        match self {
            Source::File(remote) => remote.size,
            Source::Dir(content) => content.size(),
        }
    }
}

struct KnownModel {
    /// Valeur de `asr.model`.
    id: &'static str,
    label: &'static str,
    engine: Engine,
    source: Source,
    runtime: Option<&'static Runtime>,
}

/// Fichier écrit en dernier dans un dossier téléchargé : sans lui, le
/// dossier est incomplet (téléchargement interrompu) et refait.
const COMPLETE: &str = ".complete";

/// Fichiers de Parakeet, à une révision figée du dépôt Hugging Face.
macro_rules! parakeet_url {
    ($file:literal) => {
        concat!(
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/",
            "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce/",
            $file
        )
    };
}

/// Parakeet TDT v3 : modèle par défaut sur le Steam Deck.
pub const PARAKEET_ID: &str = "parakeet-tdt-0.6b-v3-int8";

/// ONNX Runtime officiel (Microsoft), pour Parakeet.
#[cfg(windows)]
static ONNX_RUNTIME: Runtime = Runtime {
    dir: "onnxruntime-1.31.1",
    content: Content::Archive {
        remote: Remote {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip",
            size: 82_929_944,
            sha256: "5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c",
        },
        prefix: "onnxruntime-win-x64-1.31.1/lib/",
        only: &["onnxruntime.dll", "onnxruntime_providers_shared.dll"],
    },
    library: "onnxruntime.dll",
};
#[cfg(not(windows))]
static ONNX_RUNTIME: Runtime = Runtime {
    dir: "onnxruntime-1.31.1",
    content: Content::Archive {
        remote: Remote {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-1.31.1.tgz",
            size: 11_413_655,
            sha256: "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
        },
        prefix: "onnxruntime-linux-x64-1.31.1/lib/",
        only: &[
            "libonnxruntime.so.1.31.1",
            "libonnxruntime_providers_shared.so",
        ],
    },
    library: "libonnxruntime.so.1.31.1",
};

/// Bibliothèque Vosk officielle (Alpha Cephei), avec ses dépendances MinGW.
#[cfg(windows)]
static VOSK_RUNTIME: Runtime = Runtime {
    dir: "vosk-0.3.45",
    content: Content::Archive {
        remote: Remote {
            url: "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip",
            size: 14_882_445,
            sha256: "f1dcc9cca460630f81ea8f71794f69c80bed6556d2a4e6237b5785e1d2dff34b",
        },
        prefix: "vosk-win64-0.3.45/",
        only: &[
            "libvosk.dll",
            "libstdc++-6.dll",
            "libwinpthread-1.dll",
            "libgcc_s_seh-1.dll",
        ],
    },
    library: "libvosk.dll",
};
#[cfg(not(windows))]
static VOSK_RUNTIME: Runtime = Runtime {
    dir: "vosk-0.3.45",
    content: Content::Archive {
        remote: Remote {
            url: "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-linux-x86_64-0.3.45.zip",
            size: 7_185_065,
            sha256: "bbdc8ed85c43979f6443142889770ea95cbfbc56cffb5c5dcd73afa875c5fbb2",
        },
        prefix: "vosk-linux-x86_64-0.3.45/",
        only: &["libvosk.so"],
    },
    library: "libvosk.so",
};

/// Modèles de la diarisation (qui parle ?) : segmentation pyannote 3.0 (MIT)
/// et empreinte vocale WeSpeaker ResNet34-LM (CC BY 4.0), convertis en ONNX
/// par sherpa-onnx, à des révisions figées de Hugging Face. ONNX Runtime est
/// celui de Parakeet (téléchargé une fois pour les deux).
const DIARIZATION_DIR: &str = "diarisation-1";
static DIARIZATION: Content = Content::Files(&[
    (
        "segmentation.onnx",
        Remote {
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/9403a6902bb58e3d5ae8c7e77c3422de279db2e0/model.onnx",
            size: 5_992_913,
            sha256: "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079",
        },
    ),
    (
        "empreinte.onnx",
        Remote {
            url: "https://huggingface.co/csukuangfj/speaker-embedding-models/resolve/0743f301363dec56491a490f6d6cbc9d67f9a3bf/wespeaker_en_voxceleb_resnet34_LM.onnx",
            size: 26_530_550,
            sha256: "e9848563da86f263117134dfd7ad63c92355b37de492b55e325400c9d9c39012",
        },
    ),
]);

/// Modèles de la diarisation prêts à charger.
#[derive(Debug, Clone, PartialEq)]
pub struct DiarizationModels {
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
    pub runtime: PathBuf,
}

/// Modèles de la diarisation présents et complets.
pub fn diarization_local(dir: &Path) -> Option<DiarizationModels> {
    let models = dir.join(DIARIZATION_DIR);
    if !complete(&models) {
        return None;
    }
    Some(DiarizationModels {
        segmentation: models.join("segmentation.onnx"),
        embedding: models.join("empreinte.onnx"),
        runtime: local_runtime(dir, &ONNX_RUNTIME)?,
    })
}

/// Ce qu'il reste à télécharger pour la diarisation (Mo).
pub fn diarization_missing_mb(dir: &Path) -> u64 {
    let models = if complete(&dir.join(DIARIZATION_DIR)) {
        0
    } else {
        DIARIZATION.size()
    };
    let runtime = if local_runtime(dir, &ONNX_RUNTIME).is_some() {
        0
    } else {
        ONNX_RUNTIME.content.size()
    };
    (models + runtime).div_ceil(1_000_000)
}

/// Renvoie les modèles de la diarisation, en téléchargeant ce qui manque.
pub fn ensure_diarization(
    dir: &Path,
    mut progress: impl FnMut(u8),
) -> Result<DiarizationModels, String> {
    if let Some(models) = diarization_local(dir) {
        return Ok(models);
    }
    let models_missing = !complete(&dir.join(DIARIZATION_DIR));
    let runtime_missing = local_runtime(dir, &ONNX_RUNTIME).is_none();
    let mut progress = Progress {
        total: if models_missing {
            DIARIZATION.size()
        } else {
            0
        } + if runtime_missing {
            ONNX_RUNTIME.content.size()
        } else {
            0
        },
        done: 0,
        last: u8::MAX,
        report: &mut progress,
    };
    if runtime_missing {
        install_dir(
            &dir.join(ONNX_RUNTIME.dir),
            &ONNX_RUNTIME.content,
            &mut progress,
        )?;
    }
    if models_missing {
        install_dir(&dir.join(DIARIZATION_DIR), &DIARIZATION, &mut progress)?;
    }
    diarization_local(dir).ok_or_else(|| "modèles de la diarisation incomplets".to_owned())
}

/// Modèles proposés dans les réglages ; tailles et SHA-256 relevés sur
/// Hugging Face, GitHub et alphacephei.com (le téléchargement est vérifié
/// avec).
static KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        id: "ggml-large-v3-turbo-q8_0.bin",
        label: "Whisper Large v3 Turbo, quantifié 8 bits (recommandé avec carte graphique)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin",
            size: 874_188_075,
            sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        }),
        runtime: None,
    },
    KnownModel {
        id: "ggml-large-v3-turbo.bin",
        label: "Whisper Large v3 Turbo, pleine précision",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
            size: 1_624_555_275,
            sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        }),
        runtime: None,
    },
    KnownModel {
        id: "ggml-large-v3-turbo-q5_0.bin",
        label: "Whisper Large v3 Turbo, quantifié 5 bits (plus léger)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
            size: 574_041_195,
            sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        }),
        runtime: None,
    },
    KnownModel {
        id: "ggml-large-v3-q5_0.bin",
        label: "Whisper Large v3, quantifié 5 bits (plus précis, plus lent)",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin",
            size: 1_081_140_203,
            sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        }),
        runtime: None,
    },
    KnownModel {
        id: "ggml-small-q8_0.bin",
        label: "Whisper Small, quantifié 8 bits",
        engine: Engine::Whisper,
        source: Source::File(Remote {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q8_0.bin",
            size: 264_464_607,
            sha256: "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
        }),
        runtime: None,
    },
    KnownModel {
        id: PARAKEET_ID,
        label: "Parakeet TDT v3, quantifié 8 bits (PC sans carte graphique : rapide, ponctué)",
        engine: Engine::Parakeet,
        source: Source::Dir(Content::Files(&[
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
        ])),
        runtime: Some(&ONNX_RUNTIME),
    },
    KnownModel {
        id: "vosk-model-small-fr-0.22",
        label: "Vosk Small, français (très léger : vieux PC ; sans ponctuation)",
        engine: Engine::Vosk,
        source: Source::Dir(Content::Archive {
            remote: Remote {
                url: "https://alphacephei.com/vosk/models/vosk-model-small-fr-0.22.zip",
                size: 42_233_323,
                sha256: "cabf6180e177eb9b3a9a9d43a437bd5e549f3a7d09525e5d69a3fed787be12ad",
            },
            prefix: "vosk-model-small-fr-0.22/",
            only: &[],
        }),
        runtime: Some(&VOSK_RUNTIME),
    },
];

fn known(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS.iter().find(|m| m.id == id)
}

/// Moteur d'un modèle ; un fichier hors catalogue est un modèle Whisper.
pub fn engine(id: &str) -> Engine {
    known(id).map_or(Engine::Whisper, |m| m.engine)
}

#[derive(Serialize)]
pub struct ModelChoice {
    file: &'static str,
    label: &'static str,
    engine: Engine,
    /// Téléchargement complet (modèle et bibliothèque).
    size_mb: u64,
    downloaded: bool,
}

impl ModelChoice {
    pub fn file(&self) -> &str {
        self.file
    }
}

/// Modèles proposés, avec leur présence dans `dir`.
pub fn catalog(dir: &Path) -> Vec<ModelChoice> {
    KNOWN_MODELS
        .iter()
        .map(|m| ModelChoice {
            file: m.id,
            label: m.label,
            engine: m.engine,
            size_mb: (m.source.size() + m.runtime.map_or(0, |r| r.content.size())) / 1_000_000,
            downloaded: local(dir, m.id).is_some(),
        })
        .collect()
}

/// Modèle prêt à charger.
#[derive(Debug, PartialEq)]
pub struct Installed {
    /// Fichier (Whisper) ou dossier du modèle.
    pub path: PathBuf,
    /// Bibliothèque native du moteur (Parakeet, Vosk).
    pub runtime: Option<PathBuf>,
}

fn complete(dir: &Path) -> bool {
    dir.join(COMPLETE).is_file()
}

fn local_runtime(models_dir: &Path, runtime: &Runtime) -> Option<PathBuf> {
    let dir = models_dir.join(runtime.dir);
    complete(&dir).then(|| dir.join(runtime.library))
}

/// Le modèle lui-même (sans sa bibliothèque) est présent et complet.
fn model_ready(model: &KnownModel, path: &Path) -> bool {
    match &model.source {
        Source::File(remote) => fs::metadata(path).is_ok_and(|m| m.len() == remote.size),
        Source::Dir(_) => complete(path),
    }
}

/// Modèle déjà présent et complet (un fichier hors catalogue, placé à la
/// main, est pris tel quel comme modèle Whisper).
pub fn local(dir: &Path, id: &str) -> Option<Installed> {
    let path = dir.join(id);
    let Some(model) = known(id) else {
        return path.is_file().then_some(Installed {
            path,
            runtime: None,
        });
    };
    if !model_ready(model, &path) {
        return None;
    }
    let runtime = match model.runtime {
        Some(runtime) => Some(local_runtime(dir, runtime)?),
        None => None,
    };
    Some(Installed { path, runtime })
}

/// Progression globale d'un téléchargement en plusieurs fichiers.
struct Progress<'a> {
    total: u64,
    done: u64,
    last: u8,
    report: &'a mut dyn FnMut(u8),
}

impl Progress<'_> {
    fn add(&mut self, bytes: u64) {
        self.done += bytes;
        let percent = (self.done * 100 / self.total.max(1)).min(100) as u8;
        if percent != self.last {
            self.last = percent;
            (self.report)(percent);
        }
    }
}

/// Renvoie le modèle, en téléchargeant ce qui manque (modèle, bibliothèque).
///
/// `progress` reçoit le pourcentage (0–100) à chaque point gagné.
pub fn ensure(dir: &Path, id: &str, mut progress: impl FnMut(u8)) -> Result<Installed, String> {
    if let Some(installed) = local(dir, id) {
        return Ok(installed);
    }
    let Some(model) = known(id) else {
        return Err(format!(
            "modèle {id} introuvable : placez-le dans {}",
            dir.display()
        ));
    };
    let path = dir.join(id);
    let model_missing = !model_ready(model, &path);
    let runtime_missing = model.runtime.filter(|r| local_runtime(dir, r).is_none());
    let mut progress = Progress {
        total: runtime_missing.map_or(0, |r| r.content.size())
            + if model_missing {
                model.source.size()
            } else {
                0
            },
        done: 0,
        last: u8::MAX,
        report: &mut progress,
    };
    if let Some(runtime) = runtime_missing {
        install_dir(&dir.join(runtime.dir), &runtime.content, &mut progress)?;
    }
    if model_missing {
        match &model.source {
            Source::File(remote) => {
                let partial = path.with_extension("part");
                download(remote, &partial, &mut progress)?;
                fs::rename(&partial, &path).map_err(|e| format!("installation du modèle : {e}"))?;
            }
            Source::Dir(content) => install_dir(&path, content, &mut progress)?,
        }
        log::info!("modèle installé : {}", path.display());
    }
    local(dir, id).ok_or_else(|| format!("modèle {id} incomplet après téléchargement"))
}

/// Télécharge `content` dans un dossier temporaire puis le renomme en `dest`
/// une fois tout vérifié : un dossier `dest` complet, ou rien.
fn install_dir(dest: &Path, content: &Content, progress: &mut Progress) -> Result<(), String> {
    let partial = dest.with_extension("part");
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let result = fill_dir(&partial, content, progress).and_then(|()| {
        let _ = fs::remove_dir_all(dest);
        fs::rename(&partial, dest).map_err(|e| format!("installation de {} : {e}", dest.display()))
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    result?;
    log::info!("installé : {}", dest.display());
    Ok(())
}

fn fill_dir(dir: &Path, content: &Content, progress: &mut Progress) -> Result<(), String> {
    match content {
        Content::Files(files) => {
            for (name, remote) in files.iter() {
                download(remote, &dir.join(name), progress)?;
            }
        }
        Content::Archive {
            remote,
            prefix,
            only,
        } => {
            let archive = dir.join(".archive");
            download(remote, &archive, progress)?;
            extract(&archive, remote.url, dir, prefix, only)?;
            fs::remove_file(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
        }
    }
    fs::write(dir.join(COMPLETE), b"").map_err(|e| format!("{}: {e}", dir.display()))
}

fn download(remote: &Remote, path: &Path, progress: &mut Progress) -> Result<(), String> {
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
            .map_err(|e| format!("écriture de {} : {e}", path.display()))?;
        received += n as u64;
        progress.add(n as u64);
    }
    writer
        .flush()
        .map_err(|e| format!("écriture de {} : {e}", path.display()))?;
    drop(writer);

    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if received != remote.size || digest != remote.sha256 {
        let _ = fs::remove_file(path);
        return Err(format!(
            "fichier corrompu ({received} octets, sha256 {digest}) : {} ; réessayez",
            remote.url
        ));
    }
    Ok(())
}

/// Chemin relatif sûr dans l'archive, sans `prefix` ; `None` : entrée
/// ignorée (hors du préfixe, absente de `only`, ou chemin suspect).
fn archive_target(name: &Path, prefix: &str, only: &[&str]) -> Option<PathBuf> {
    let relative = name.strip_prefix(prefix).ok()?;
    let safe = relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !safe || relative.as_os_str().is_empty() {
        return None;
    }
    if !only.is_empty() && !only.iter().any(|o| relative == Path::new(o)) {
        return None;
    }
    Some(relative.to_owned())
}

fn write_entry(reader: &mut impl Read, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut file = File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    io::copy(reader, &mut file).map_err(|e| format!("extraction de {} : {e}", dest.display()))?;
    Ok(())
}

/// Extrait les fichiers ordinaires de l'archive (zip, ou tar.gz d'après
/// `url`) ; liens et dossiers sont ignorés.
fn extract(
    archive: &Path,
    url: &str,
    dest: &Path,
    prefix: &str,
    only: &[&str],
) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut extracted = 0;
    if url.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("archive : {e}"))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| format!("archive : {e}"))?;
            if !entry.is_file() {
                continue;
            }
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            if let Some(target) = archive_target(&name, prefix, only) {
                write_entry(&mut entry, &dest.join(target))?;
                extracted += 1;
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().map_err(|e| format!("archive : {e}"))? {
            let mut entry = entry.map_err(|e| format!("archive : {e}"))?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let name = entry
                .path()
                .map_err(|e| format!("archive : {e}"))?
                .into_owned();
            if let Some(target) = archive_target(&name, prefix, only) {
                write_entry(&mut entry, &dest.join(target))?;
                extracted += 1;
            }
        }
    }
    if extracted < only.len().max(1) {
        return Err(format!("archive incomplète : {url}"));
    }
    Ok(())
}

/// Charge le modèle installé avec le moteur qui lui correspond.
pub fn load(
    installed: &Installed,
    asr: &AsrConfig,
    dictionary: &[String],
) -> Result<Box<dyn Transcriber>, String> {
    let runtime = || {
        installed
            .runtime
            .as_deref()
            .ok_or_else(|| format!("bibliothèque du moteur absente pour {}", asr.model))
    };
    let transcriber: Box<dyn Transcriber> = match engine(&asr.model) {
        Engine::Whisper => Box::new(
            WhisperTranscriber::load(&installed.path, &asr.language, asr.use_gpu, dictionary)
                .map_err(|e| e.to_string())?,
        ),
        Engine::Parakeet => Box::new(
            ParakeetTranscriber::load(&installed.path, runtime()?).map_err(|e| e.to_string())?,
        ),
        Engine::Vosk => Box::new(
            VoskTranscriber::load(&vtt_platform::ascii_path(&installed.path), runtime()?)
                .map_err(|e| e.to_string())?,
        ),
    };
    Ok(transcriber)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtt-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn local_requires_a_complete_known_model() {
        let dir = temp_dir("local");
        let known = KNOWN_MODELS[0].id;
        assert_eq!(local(&dir, known), None, "absent");
        fs::write(dir.join(known), b"partiel").unwrap();
        assert_eq!(local(&dir, known), None, "taille incorrecte");
        fs::write(dir.join("perso.bin"), b"modele").unwrap();
        assert_eq!(
            local(&dir, "perso.bin"),
            Some(Installed {
                path: dir.join("perso.bin"),
                runtime: None
            }),
            "hors catalogue, pris tel quel"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_models_need_their_marker_and_runtime() {
        let dir = temp_dir("folders");
        let id = "vosk-model-small-fr-0.22";
        fs::create_dir_all(dir.join(id)).unwrap();
        assert_eq!(local(&dir, id), None, "dossier sans marqueur");
        fs::write(dir.join(id).join(COMPLETE), b"").unwrap();
        assert_eq!(local(&dir, id), None, "bibliothèque absente");
        fs::create_dir_all(dir.join(VOSK_RUNTIME.dir)).unwrap();
        fs::write(dir.join(VOSK_RUNTIME.dir).join(COMPLETE), b"").unwrap();
        assert_eq!(
            local(&dir, id),
            Some(Installed {
                path: dir.join(id),
                runtime: Some(dir.join(VOSK_RUNTIME.dir).join(VOSK_RUNTIME.library)),
            })
        );
        assert_eq!(engine(id), Engine::Vosk);
        assert_eq!(engine("perso.bin"), Engine::Whisper);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn archive_entries_stay_inside_their_folder() {
        let prefix = "vosk-model-small-fr-0.22/";
        let target = |name: &str, only: &[&str]| archive_target(Path::new(name), prefix, only);
        assert_eq!(
            target("vosk-model-small-fr-0.22/am/final.mdl", &[]),
            Some(PathBuf::from("am/final.mdl"))
        );
        assert_eq!(target("autre/am/final.mdl", &[]), None, "hors préfixe");
        assert_eq!(target("vosk-model-small-fr-0.22/../x", &[]), None);
        assert_eq!(target("vosk-model-small-fr-0.22/", &[]), None);
        assert_eq!(
            target("vosk-model-small-fr-0.22/README", &["am/final.mdl"]),
            None,
            "pas dans la liste"
        );
    }

    /// Vrai téléchargement des moteurs légers (≈ 750 Mo), puis chargement :
    /// `cargo test -p voicetotext -- --ignored light_engines`.
    #[test]
    #[ignore]
    fn light_engines_download_and_load() {
        let dir = temp_dir("download");
        for id in ["vosk-model-small-fr-0.22", "parakeet-tdt-0.6b-v3-int8"] {
            let installed = ensure(&dir, id, |_| {}).unwrap();
            let asr = AsrConfig {
                model: id.into(),
                ..AsrConfig::default()
            };
            let transcriber = load(&installed, &asr, &[]).unwrap();
            let silence = vtt_core::AudioClip {
                samples: vec![0.0; 16_000],
            };
            assert!(transcriber.transcribe(&silence).unwrap().len() < 5);
            assert_eq!(local(&dir, id), Some(installed));
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn diarization_needs_its_models_and_the_runtime() {
        let dir = temp_dir("diarization");
        assert_eq!(diarization_local(&dir), None);
        assert_eq!(
            diarization_missing_mb(&dir),
            (DIARIZATION.size() + ONNX_RUNTIME.content.size()).div_ceil(1_000_000)
        );
        fs::create_dir_all(dir.join(DIARIZATION_DIR)).unwrap();
        fs::write(dir.join(DIARIZATION_DIR).join(COMPLETE), b"").unwrap();
        assert_eq!(diarization_local(&dir), None, "ONNX Runtime absent");
        assert_eq!(
            diarization_missing_mb(&dir),
            ONNX_RUNTIME.content.size().div_ceil(1_000_000)
        );
        fs::create_dir_all(dir.join(ONNX_RUNTIME.dir)).unwrap();
        fs::write(dir.join(ONNX_RUNTIME.dir).join(COMPLETE), b"").unwrap();
        let models = diarization_local(&dir).unwrap();
        assert_eq!(
            models.runtime,
            dir.join(ONNX_RUNTIME.dir).join(ONNX_RUNTIME.library)
        );
        assert_eq!(
            models.embedding,
            dir.join(DIARIZATION_DIR).join("empreinte.onnx")
        );
        assert_eq!(diarization_missing_mb(&dir), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Vrai téléchargement des modèles de la diarisation (33 Mo, plus ONNX
    /// Runtime), puis chargement et analyse de 12 s de silence :
    /// `cargo test -p voicetotext -- --ignored diarization_download`.
    /// `VTT_ONNXRUNTIME` : bibliothèque ONNX Runtime 1.31.1 déjà présente
    /// (évite son téléchargement).
    #[test]
    #[ignore]
    fn diarization_download_and_load() {
        let dir = temp_dir("diarization-download");
        if let Ok(library) = std::env::var("VTT_ONNXRUNTIME") {
            let runtime = dir.join(ONNX_RUNTIME.dir);
            fs::create_dir_all(&runtime).unwrap();
            fs::copy(library, runtime.join(ONNX_RUNTIME.library)).unwrap();
            fs::write(runtime.join(COMPLETE), b"").unwrap();
        }
        let mut last = 0;
        let models = ensure_diarization(&dir, |p| last = p).unwrap();
        assert_eq!(last, 100);
        assert_eq!(diarization_local(&dir), Some(models.clone()));
        let mut diarizer =
            vtt_asr::Diarizer::load(&models.segmentation, &models.embedding, &models.runtime, 2)
                .unwrap();
        let turns = diarizer
            .diarize(&vec![0.0; 12 * 16_000], &Default::default(), &mut |_| {})
            .unwrap();
        assert!(turns.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn catalog_ids_are_unique_and_sized() {
        let mut ids: Vec<&str> = KNOWN_MODELS.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), KNOWN_MODELS.len());
        for choice in catalog(Path::new("/nulle-part")) {
            assert!(choice.size_mb > 0 && !choice.downloaded);
        }
    }
}
