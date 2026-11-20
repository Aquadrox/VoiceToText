//! Carnet de voix local (opt-in explicite, chiffré sur disque) : mémorise
//! l'empreinte vocale moyenne (WeSpeaker, normalisée L2) des personnes que
//! l'utilisateur choisit d'enregistrer, afin de les reconnaître automatiquement
//! d'une réunion à l'autre.
//!
//! Principes de confidentialité et de prudence :
//! - **Opt-in explicite uniquement** : aucune voix n'est mémorisée sans action
//!   volontaire de l'utilisateur (« Mémoriser la voix de … »).
//! - **Jamais en clair sur le disque** : [`VoiceBook::save`] et [`VoiceBook::load`]
//!   passent par le chiffrement local (`vtt_platform::protect` sur PC, stockage
//!   privé chiffré sur Android), exactement comme `voix.bin`.
//! - **Prudence anti-faux-positifs** : similarité cosinus minimale stricte
//!   ([`MATCH_THRESHOLD`]), écart minimal avec la deuxième voix la plus proche
//!   ([`MATCH_MARGIN`]), et affectation 1-pour-1 (deux intervenants d'une même
//!   réunion ne reçoivent jamais le même nom).
//! - **Oubli immédiat** : suppression d'une voix en un clic ([`VoiceBook::remove`]).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::diarize::SpeakerProfile;
use crate::meeting::same_person;
use crate::search::fold;

/// Nom du fichier local du carnet de voix (toujours chiffré sur PC).
pub const VOICEBOOK_FILE: &str = "voicebook.bin";

/// En-tête binaire permettant de détecter immédiatement un fichier corrompu
/// ou déchiffré avec une mauvaise clé.
const MAGIC: &[u8; 8] = b"VTTVB01\0";

/// Version du format sérialisé.
const FORMAT_VERSION: u32 = 1;

/// Seuil minimal de similarité cosinus pour reconnaître une voix connue entre
/// deux réunions (WeSpeaker ResNet34-LM : distance cosinus ≤ 0,38, nettement
/// sous les distances inter-locuteurs mesurées 0,63–0,98).
pub const MATCH_THRESHOLD: f32 = 0.62;

/// Écart minimal de similarité avec la deuxième voix la plus proche du carnet
/// pour éviter toute confusion entre deux voix proches.
pub const MATCH_MARGIN: f32 = 0.08;

/// Longueur maximale d'un nom enregistré dans le carnet.
pub const MAX_NAME_CHARS: usize = 60;

/// Plafond de poids des anciens échantillons lors d'une mise à jour, pour que
/// le profil puisse s'adapter doucement à un changement de micro ou de salle.
const MAX_ENROLLED_WEIGHT: u32 = 64;

fn format_version() -> u32 {
    FORMAT_VERSION
}

/// Empreinte vocale mémorisée pour une personne (opt-in explicite).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceEntry {
    pub id: String,
    pub name: String,
    /// Empreinte moyenne normalisée L2 (WeSpeaker).
    pub embedding: Vec<f32>,
    /// Nombre d'échantillons (fenêtres WeSpeaker) agrégés.
    pub samples: u32,
    /// Nombre de réunions ayant enrichi cette empreinte.
    pub meetings: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Vue publique d'une entrée du carnet (sans le vecteur biométrique brut),
/// destinée aux écrans PC et Android.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceEntryView {
    pub id: String,
    pub name: String,
    pub samples: u32,
    pub meetings: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl From<&VoiceEntry> for VoiceEntryView {
    fn from(entry: &VoiceEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            samples: entry.samples,
            meetings: entry.meetings,
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
        }
    }
}

/// Correspondance retenue entre un intervenant d'une réunion et une entrée
/// du carnet de voix.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceMatch {
    /// Numéro de l'intervenant dans la réunion (`0, 1, …`).
    pub speaker: usize,
    pub entry_id: String,
    pub name: String,
    /// Similarité cosinus (dans `[-1.0, 1.0]`, toujours ≥ seuil retenu).
    pub similarity: f32,
}

impl VoiceMatch {
    /// Score arrondi en pourcentage (`0..=100`) pour l'affichage.
    pub fn confidence_percent(&self) -> u8 {
        (self.similarity.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    pub fn to_recognition(&self) -> VoiceRecognition {
        VoiceRecognition {
            entry_id: self.entry_id.clone(),
            name: self.name.clone(),
            confidence: self.confidence_percent(),
        }
    }
}

/// Intervenant reconnu automatiquement grâce au carnet de voix local, stocké
/// dans `meeting.json` (facultatif, rétrocompatible).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRecognition {
    pub entry_id: String,
    pub name: String,
    /// Confiance arrondie en pourcentage (`0..=100`).
    pub confidence: u8,
}

/// Base locale d'empreintes vocales récurrentes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceBook {
    #[serde(default = "format_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<VoiceEntry>,
}

impl Default for VoiceBook {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            entries: Vec::new(),
        }
    }
}

impl VoiceBook {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: &str) -> Option<&VoiceEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Liste des voix mémorisées sans leurs vecteurs biométriques.
    pub fn views(&self) -> Vec<VoiceEntryView> {
        self.entries.iter().map(VoiceEntryView::from).collect()
    }

    /// Mémorise (ou enrichit si la personne existe déjà dans le carnet) une
    /// empreinte vocale par choix explicite de l'utilisateur.
    pub fn enroll(
        &mut self,
        name: &str,
        embedding: &[f32],
        samples: u32,
        now_ms: u64,
    ) -> Result<VoiceEntry, String> {
        let name = clean_name(name)?;
        let unit = normalize_checked(embedding)?;
        let weight = samples.max(1);

        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| same_person(&e.name, &name))
        {
            if existing.embedding.len() != unit.len() {
                return Err(format!(
                    "dimension d'empreinte incompatible ({} vs {})",
                    unit.len(),
                    existing.embedding.len()
                ));
            }
            let w_old = existing.samples.clamp(1, MAX_ENROLLED_WEIGHT) as f32;
            let w_new = weight.min(MAX_ENROLLED_WEIGHT) as f32;
            let merged: Vec<f32> = existing
                .embedding
                .iter()
                .zip(&unit)
                .map(|(&a, &b)| a * w_old + b * w_new)
                .collect();
            existing.embedding = normalize_checked(&merged)?;
            existing.name = name;
            existing.samples = existing.samples.saturating_add(weight);
            existing.meetings = existing.meetings.saturating_add(1);
            existing.updated_at_ms = now_ms;
            let saved = existing.clone();
            self.sort();
            return Ok(saved);
        }

        let id = self.next_id(now_ms);
        let entry = VoiceEntry {
            id,
            name,
            embedding: unit,
            samples: weight,
            meetings: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.entries.push(entry.clone());
        self.sort();
        Ok(entry)
    }

    /// Renomme une voix mémorisée (refuse le doublon avec une autre entrée).
    pub fn rename(&mut self, id: &str, new_name: &str, now_ms: u64) -> Result<(), String> {
        let clean = clean_name(new_name)?;
        if self
            .entries
            .iter()
            .any(|e| e.id != id && same_person(&e.name, &clean))
        {
            return Err("une voix porte déjà ce nom".into());
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| "voix introuvable".to_owned())?;
        entry.name = clean;
        entry.updated_at_ms = now_ms;
        self.sort();
        Ok(())
    }

    /// Supprime une voix mémorisée en un clic ; renvoie `true` si elle existait.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() < before
    }

    /// Efface l'ensemble du carnet de voix.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Reconnaît les intervenants connus avec les seuils prudents par défaut
    /// ([`MATCH_THRESHOLD`] et [`MATCH_MARGIN`]).
    pub fn match_speakers(&self, profiles: &[SpeakerProfile]) -> Vec<VoiceMatch> {
        self.match_speakers_with(profiles, MATCH_THRESHOLD, MATCH_MARGIN)
    }

    /// Reconnaît les intervenants connus avec un seuil de similarité et une
    /// marge d'ambiguïté donnés. Garantit une affectation 1-pour-1 (chaque
    /// personne du carnet est attribuée au plus à un intervenant).
    pub fn match_speakers_with(
        &self,
        profiles: &[SpeakerProfile],
        threshold: f32,
        margin: f32,
    ) -> Vec<VoiceMatch> {
        if self.entries.is_empty() || profiles.is_empty() {
            return Vec::new();
        }
        let mut candidates: Vec<(f32, usize, usize)> = Vec::new();
        for profile in profiles {
            let Ok(unit) = normalize_checked(&profile.embedding) else {
                continue;
            };
            let mut best: Option<(f32, usize)> = None;
            let mut second = -1.0f32;
            for (entry_idx, entry) in self.entries.iter().enumerate() {
                if entry.embedding.len() != unit.len() {
                    continue;
                }
                let sim = cosine_similarity(&unit, &entry.embedding);
                match best {
                    None => best = Some((sim, entry_idx)),
                    Some((best_sim, _)) if sim > best_sim => {
                        second = best_sim;
                        best = Some((sim, entry_idx));
                    }
                    Some(_) if sim > second => second = sim,
                    _ => {}
                }
            }
            if let Some((sim, entry_idx)) = best
                && sim >= threshold
                && sim - second >= margin
            {
                candidates.push((sim, profile.speaker, entry_idx));
            }
        }

        // Affectation gloutonne 1-pour-1 par score décroissant.
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut used_speakers = HashSet::new();
        let mut used_entries = HashSet::new();
        let mut matches = Vec::new();
        for (similarity, speaker, entry_idx) in candidates {
            if used_speakers.insert(speaker) && used_entries.insert(entry_idx) {
                let entry = &self.entries[entry_idx];
                matches.push(VoiceMatch {
                    speaker,
                    entry_id: entry.id.clone(),
                    name: entry.name.clone(),
                    similarity,
                });
            }
        }
        matches.sort_by_key(|m| m.speaker);
        matches
    }

    /// Sérialise le carnet (prêt à chiffrer avant écriture sur disque).
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let json = serde_json::to_vec(self).map_err(|e| format!("carnet de voix : {e}"))?;
        let mut out = Vec::with_capacity(MAGIC.len() + json.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&json);
        Ok(out)
    }

    /// Relit un carnet depuis ses octets déchiffrés.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let payload = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| "carnet de voix illisible (format inconnu)".to_owned())?;
        let mut book: Self =
            serde_json::from_slice(payload).map_err(|e| format!("carnet de voix : {e}"))?;
        for entry in &mut book.entries {
            entry.embedding = normalize_checked(&entry.embedding)?;
        }
        book.sort();
        Ok(book)
    }

    /// Enregistre le carnet de manière atomique (fichier temporaire puis
    /// renommage), chiffré par `protect`.
    pub fn save(
        &self,
        path: &Path,
        protect: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let plain = self.to_bytes()?;
        let data = protect(&plain)?;
        let tmp = path.with_extension("bin.tmp");
        fs::write(&tmp, data)
            .and_then(|()| fs::rename(&tmp, path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Charge le carnet depuis `path` (vide si le fichier n'existe pas encore).
    pub fn load(
        path: &Path,
        unprotect: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
    ) -> Result<Self, String> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let plain = unprotect(&data)?;
        Self::from_bytes(&plain)
    }

    /// Supprime le fichier du carnet s'il existe.
    pub fn delete_file(path: &Path) {
        let _ = fs::remove_file(path);
    }

    fn next_id(&self, now_ms: u64) -> String {
        let mut candidate = now_ms.max(1);
        loop {
            let id = candidate.to_string();
            if !self.entries.iter().any(|e| e.id == id) {
                return id;
            }
            candidate = candidate.saturating_add(1);
        }
    }

    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            fold(&a.name)
                .cmp(&fold(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
}

fn clean_name(raw: &str) -> Result<String, String> {
    let name: String = raw.trim().chars().take(MAX_NAME_CHARS).collect();
    if name.is_empty() {
        return Err("nom de l'intervenant vide".into());
    }
    Ok(name)
}

/// Normalise un vecteur L2 en vérifiant qu'il est non vide, fini et non nul.
pub fn normalize_checked(v: &[f32]) -> Result<Vec<f32>, String> {
    if v.is_empty() {
        return Err("empreinte vocale vide".into());
    }
    if !v.iter().all(|x| x.is_finite()) {
        return Err("empreinte vocale invalide".into());
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return Err("empreinte vocale nulle".into());
    }
    Ok(v.iter().map(|x| x / norm).collect())
}

/// Similarité cosinus entre deux vecteurs déjà normalisés L2.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x * y)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-voicebook-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join(VOICEBOOK_FILE)
    }

    fn profile(speaker: usize, embedding: Vec<f32>) -> SpeakerProfile {
        SpeakerProfile {
            speaker,
            embedding: normalize_checked(&embedding).unwrap(),
            samples: 5,
            clean_ms: 10_000,
        }
    }

    #[test]
    fn enroll_merges_same_person_renames_and_removes() {
        let mut book = VoiceBook::default();
        assert!(book.enroll("  ", &[1.0, 0.0], 4, 100).is_err());
        assert!(book.enroll("Marie", &[], 4, 100).is_err());
        assert!(book.enroll("Marie", &[0.0, 0.0], 4, 100).is_err());
        assert!(book.enroll("Marie", &[f32::NAN, 1.0], 4, 100).is_err());

        let alice = book
            .enroll("  Marie Dupont ", &[1.0, 0.0, 0.0], 4, 100)
            .unwrap();
        let paul = book.enroll("Paul", &[0.0, 1.0, 0.0], 3, 100).unwrap();
        assert_ne!(alice.id, paul.id);
        assert_eq!(book.len(), 2);

        // Deuxième mémorisation de la même personne (casse/accents ignorés) :
        // fusion pondérée et mise à jour du compteur de réunions.
        let updated = book
            .enroll("marie dupont", &[0.8, 0.6, 0.0], 4, 200)
            .unwrap();
        assert_eq!(updated.id, alice.id);
        assert_eq!(updated.meetings, 2);
        assert_eq!(updated.samples, 8);
        assert!(updated.embedding[0] > updated.embedding[1]);

        // Renommage + refus de doublon.
        assert!(book.rename(&paul.id, "MARIE DUPONT", 300).is_err());
        book.rename(&paul.id, "Paul Martin", 300).unwrap();
        assert_eq!(book.get(&paul.id).unwrap().name, "Paul Martin");

        // Suppression en un clic.
        assert!(book.remove(&alice.id));
        assert!(!book.remove(&alice.id));
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn matches_known_voices_strictly_with_margin_and_one_to_one() {
        let mut book = VoiceBook::default();
        book.enroll("Alice", &[1.0, 0.0, 0.0, 0.0], 6, 100).unwrap();
        book.enroll("Bob", &[0.0, 1.0, 0.0, 0.0], 6, 101).unwrap();

        let profiles = vec![
            // Proche d'Alice (cosinus ≈ 0,97)
            profile(0, vec![0.97, 0.2, 0.1, 0.0]),
            // Inconnu orthogonal (cosinus ≈ 0)
            profile(1, vec![0.0, 0.0, 1.0, 0.0]),
            // Proche de Bob (cosinus ≈ 0,95)
            profile(2, vec![0.1, 0.95, 0.2, 0.0]),
        ];
        let matches = book.match_speakers(&profiles);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].speaker, 0);
        assert_eq!(matches[0].name, "Alice");
        assert!(matches[0].confidence_percent() >= 90);
        assert_eq!(matches[1].speaker, 2);
        assert_eq!(matches[1].name, "Bob");

        // Ambiguïté : un intervenant à mi-chemin entre Alice et Bob (marge < 0,08)
        // ne doit PAS être attribué.
        let ambiguous = vec![profile(0, vec![0.73, 0.68, 0.0, 0.0])];
        assert!(book.match_speakers(&ambiguous).is_empty());

        // Affectation 1-pour-1 : si deux intervenants ressemblent tous deux à Alice,
        // seul le meilleur obtient le nom.
        let duplicates = vec![
            profile(0, vec![0.9, 0.0, 0.43, 0.0]),
            profile(1, vec![0.99, 0.0, 0.14, 0.0]),
        ];
        let assigned = book.match_speakers(&duplicates);
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].speaker, 1);
        assert_eq!(assigned[0].name, "Alice");
    }

    #[test]
    fn encrypted_save_load_and_corruption_handling() {
        let path = temp_path("persist");
        let xor = |data: &[u8]| Ok(data.iter().map(|b| b ^ 0x5A).collect());
        let bad_key = |data: &[u8]| Ok(data.iter().map(|b| b ^ 0xFF).collect());

        // Absent au départ : carnet vide sans erreur.
        assert!(VoiceBook::load(&path, xor).unwrap().is_empty());

        let mut book = VoiceBook::default();
        book.enroll("Élodie", &[0.6, 0.8, 0.0], 5, 1_000).unwrap();
        book.save(&path, xor).unwrap();

        // Jamais en clair sur le disque.
        let raw = fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Élodie"));

        let loaded = VoiceBook::load(&path, xor).unwrap();
        assert_eq!(loaded, book);
        assert_eq!(loaded.views().len(), 1);
        assert_eq!(loaded.views()[0].name, "Élodie");

        // Mauvaise clé / fichier altéré : erreur propre.
        assert!(VoiceBook::load(&path, bad_key).is_err());

        VoiceBook::delete_file(&path);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
