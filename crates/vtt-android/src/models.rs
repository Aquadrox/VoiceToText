//! Modèles Whisper proposés sur le téléphone et leur téléchargement
//! (vérifié : taille et SHA-256).

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub struct KnownModel {
    pub id: &'static str,
    pub label: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

/// Tailles et empreintes relevées sur Hugging Face (ggerganov/whisper.cpp).
pub const MODELS: &[KnownModel] = &[
    KnownModel {
        id: "ggml-small-q8_0.bin",
        label: "Small, 8 bits (recommandé : rapide et précis)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q8_0.bin",
        size: 264_464_607,
        sha256: "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
    },
    KnownModel {
        id: "ggml-base-q8_0.bin",
        label: "Base, 8 bits (très léger, moins précis)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q8_0.bin",
        size: 81_768_585,
        sha256: "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
    },
    KnownModel {
        id: "ggml-large-v3-turbo-q5_0.bin",
        label: "Large v3 Turbo, 5 bits (le plus précis, plus lent)",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
];

/// Modèle par défaut sur le téléphone.
pub const DEFAULT_MODEL: &str = "ggml-small-q8_0.bin";

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
            size_mb: m.size / 1_000_000,
            downloaded: local(dir, m.id).is_some(),
        })
        .collect()
}

/// Chemin du modèle s'il est présent et complet.
pub fn local(dir: &Path, id: &str) -> Option<PathBuf> {
    let model = known(id)?;
    let path = dir.join(id);
    fs::metadata(&path)
        .is_ok_and(|m| m.len() == model.size)
        .then_some(path)
}

/// Progression du téléchargement en cours (0–100), lue par l'interface.
pub static PROGRESS: AtomicU8 = AtomicU8::new(0);

/// Télécharge le modèle s'il manque ; vérifie taille et SHA-256.
pub fn download(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if let Some(path) = local(dir, id) {
        return Ok(path);
    }
    let model = known(id).ok_or_else(|| format!("modèle inconnu : {id}"))?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(id);
    let partial = path.with_extension("part");
    PROGRESS.store(0, Ordering::Relaxed);
    log::info!("téléchargement de {}", model.url);
    let mut response = ureq::get(model.url)
        .call()
        .map_err(|e| format!("téléchargement : {e}"))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(model.size + 1)
        .reader();
    let file = File::create(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
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
        PROGRESS.store(
            (received * 100 / model.size).min(100) as u8,
            Ordering::Relaxed,
        );
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
            "modèle corrompu ({received} octets) : réessayez le téléchargement"
        ));
    }
    fs::rename(&partial, &path).map_err(|e| format!("installation du modèle : {e}"))?;
    log::info!("modèle installé : {}", path.display());
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_known_models_are_local() {
        let dir = std::env::temp_dir().join(format!("vtt-android-models-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(local(&dir, DEFAULT_MODEL), None);
        fs::write(dir.join(DEFAULT_MODEL), b"partiel").unwrap();
        assert_eq!(local(&dir, DEFAULT_MODEL), None, "taille incorrecte");
        assert_eq!(local(&dir, "perso.bin"), None, "hors catalogue");
        assert!(known(DEFAULT_MODEL).is_some());
        assert!(catalog(&dir).iter().all(|m| !m.downloaded));
        fs::remove_dir_all(&dir).unwrap();
    }
}
