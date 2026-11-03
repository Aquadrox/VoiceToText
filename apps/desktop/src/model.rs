//! Modèles Whisper proposés et leur téléchargement (au premier lancement ou
//! après un changement de modèle dans les réglages).

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

struct KnownModel {
    file: &'static str,
    label: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

/// Modèles proposés dans les réglages ; tailles et SHA-256 relevés sur
/// Hugging Face (le téléchargement est vérifié avec).
const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        file: "ggml-large-v3-turbo-q8_0.bin",
        label: "Large v3 Turbo, quantifié 8 bits (recommandé)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin",
        size: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
    },
    KnownModel {
        file: "ggml-large-v3-turbo.bin",
        label: "Large v3 Turbo, pleine précision",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
        size: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    },
    KnownModel {
        file: "ggml-large-v3-turbo-q5_0.bin",
        label: "Large v3 Turbo, quantifié 5 bits (plus léger)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
    KnownModel {
        file: "ggml-large-v3-q5_0.bin",
        label: "Large v3, quantifié 5 bits (plus précis, plus lent)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin",
        size: 1_081_140_203,
        sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
    },
    KnownModel {
        file: "ggml-small-q8_0.bin",
        label: "Small, quantifié 8 bits (PC sans carte graphique)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q8_0.bin",
        size: 264_464_607,
        sha256: "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
    },
];

#[derive(serde::Serialize)]
pub struct ModelChoice {
    file: &'static str,
    label: &'static str,
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
            file: m.file,
            label: m.label,
            size_mb: m.size / 1_000_000,
            downloaded: fs::metadata(dir.join(m.file)).is_ok_and(|meta| meta.len() == m.size),
        })
        .collect()
}

/// Renvoie le chemin du modèle, en le téléchargeant si besoin.
///
/// `progress` reçoit le pourcentage (0–100) à chaque point gagné.
pub fn ensure(dir: &Path, file: &str, mut progress: impl FnMut(u8)) -> Result<PathBuf, String> {
    let path = dir.join(file);
    let known = KNOWN_MODELS.iter().find(|m| m.file == file);
    let present_size = fs::metadata(&path).ok().map(|m| m.len());

    match (known, present_size) {
        (None, Some(_)) => return Ok(path),
        (None, None) => {
            return Err(format!(
                "modèle {file} introuvable : placez-le dans {}",
                dir.display()
            ));
        }
        (Some(model), Some(size)) if size == model.size => return Ok(path),
        (Some(model), _) => download(model, &path, &mut progress)?,
    }
    Ok(path)
}

fn download(model: &KnownModel, path: &Path, progress: &mut impl FnMut(u8)) -> Result<(), String> {
    log::info!("téléchargement de {}", model.url);
    let partial = path.with_extension("part");
    let mut response = ureq::get(model.url)
        .call()
        .map_err(|e| format!("téléchargement du modèle : {e}"))?;
    let mut reader = response.body_mut().as_reader();
    let file = File::create(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    let mut last_percent = u8::MAX;

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
        let percent = (received * 100 / model.size).min(100) as u8;
        if percent != last_percent {
            last_percent = percent;
            progress(percent);
        }
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
    if received != model.size || digest != model.sha256 {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "modèle corrompu ({received} octets, sha256 {digest}) : relancez l'application"
        ));
    }
    fs::rename(&partial, path).map_err(|e| format!("installation du modèle : {e}"))?;
    log::info!("modèle installé : {}", path.display());
    Ok(())
}
