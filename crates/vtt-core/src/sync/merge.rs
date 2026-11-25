//! Fusion déterministe des réglages portables et différentiel des réunions.
//!
//! Principes de sécurité et de portabilité :
//! - **Jamais de clés API** : tout instantané passe par [`Config::for_export`]
//!   à l'émission et à la réception (les champs de `extra_body` ressemblant à
//!   un secret sont systématiquement retirés).
//! - **Réglages matériels gardés locaux** : raccourcis clavier, micro, modèle
//!   ASR, mises à jour, captures, historique et préférences d'enregistrement
//!   local ne sont jamais écrasés par un autre appareil (un PC RTX 4070 Ti et
//!   un téléphone Android ne partagent pas le même modèle ASR ni les mêmes
//!   raccourcis).
//! - **Convergence déterministe** : à horodatages et identifiants donnés, la
//!   fusion `(A, B)` côté A et `(B, A)` côté B aboutit aux mêmes ensembles
//!   (dictionnaire, snippets, styles, prompts, profils LLM).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::pairing::DeviceId;
use super::{Result, SyncError};
use crate::config::{
    Config, LlmProfile, PromptKind, PromptPreset, Snippet, Style, clean_dictionary,
    clean_llm_profiles, clean_prompts, clean_snippets, clean_styles,
};
use crate::meeting::MeetingRecord;
use crate::text::normalize_phrase;

/// Sous-ensemble portable de [`Config`] échangé entre appareils.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncableConfig {
    pub dictionary: Vec<String>,
    pub dictionary_ignored: Vec<String>,
    pub snippets: Vec<Snippet>,
    pub styles: Vec<Style>,
    pub prompts: Vec<PromptPreset>,
    pub llm_profiles: Vec<LlmProfile>,
}

impl SyncableConfig {
    /// Extrait la partie synchronisable d'une configuration en retirant tout
    /// champ ressemblant à un secret (`extra_body`).
    pub fn from_config(config: &Config) -> (Self, Vec<String>) {
        let (safe, removed) = config.for_export();
        let payload = Self {
            dictionary: safe.dictionary,
            dictionary_ignored: safe.dictionary_ignored,
            snippets: safe.snippets,
            styles: safe.styles,
            prompts: safe.prompts,
            llm_profiles: safe.llm_profiles,
        };
        (payload, removed)
    }

    /// Retire de nouveau tout champ ressemblant à un secret sur un instantané
    /// reçu du réseau (défense en profondeur).
    pub fn sanitized(&self) -> (Self, Vec<String>) {
        let probe = Config {
            dictionary: self.dictionary.clone(),
            dictionary_ignored: self.dictionary_ignored.clone(),
            snippets: self.snippets.clone(),
            styles: self.styles.clone(),
            prompts: self.prompts.clone(),
            llm_profiles: self.llm_profiles.clone(),
            ..Config::default()
        };
        Self::from_config(&probe)
    }
}

/// Bilan chiffré d'une fusion de configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeReport {
    pub dictionary_added: usize,
    pub snippets_added: usize,
    pub snippets_updated: usize,
    pub styles_added: usize,
    pub styles_updated: usize,
    pub prompts_added: usize,
    pub prompts_updated: usize,
    pub profiles_added: usize,
    pub profiles_updated: usize,
    /// Champs suspects retirés par sécurité d'un instantané distant.
    pub stripped_secrets: Vec<String>,
}

impl MergeReport {
    pub const fn total_changes(&self) -> usize {
        self.dictionary_added
            + self.snippets_added
            + self.snippets_updated
            + self.styles_added
            + self.styles_updated
            + self.prompts_added
            + self.prompts_updated
            + self.profiles_added
            + self.profiles_updated
    }
}

/// Détermine si la version distante l'emporte en cas de conflit sur un même
/// élément (horodatage le plus récent, départage déterministe par identifiant).
fn remote_wins(
    local_updated_at_ms: u64,
    local_id: &DeviceId,
    remote_updated_at_ms: u64,
    remote_id: &DeviceId,
) -> bool {
    (remote_updated_at_ms, remote_id.as_str()) > (local_updated_at_ms, local_id.as_str())
}

/// Fusionne `remote` dans `local` sans jamais altérer les réglages matériels
/// ou locaux ni importer de secrets.
pub fn merge_config(
    local: &Config,
    local_updated_at_ms: u64,
    local_id: &DeviceId,
    remote: &SyncableConfig,
    remote_updated_at_ms: u64,
    remote_id: &DeviceId,
) -> Result<(Config, MergeReport)> {
    let (remote_safe, stripped_secrets) = remote.sanitized();
    let prefer_remote = remote_wins(
        local_updated_at_ms,
        local_id,
        remote_updated_at_ms,
        remote_id,
    );
    let mut report = MergeReport {
        stripped_secrets,
        ..MergeReport::default()
    };

    let mut merged = local.clone();

    // 1. Dictionnaire : union insensible à la casse (départage déterministe).
    let (dict, added_dict) = merge_words(&local.dictionary, &remote_safe.dictionary, prefer_remote);
    report.dictionary_added = added_dict;
    merged.dictionary = clean_dictionary(&dict).map_err(SyncError::InvalidConfig)?;

    let (ignored, _) = merge_words(
        &local.dictionary_ignored,
        &remote_safe.dictionary_ignored,
        prefer_remote,
    );
    merged.dictionary_ignored = clean_dictionary(&ignored).map_err(SyncError::InvalidConfig)?;

    // 2. Snippets : fusion par déclencheur normalisé.
    let (snippets, added, updated) =
        merge_snippets(&local.snippets, &remote_safe.snippets, prefer_remote);
    report.snippets_added = added;
    report.snippets_updated = updated;
    merged.snippets = clean_snippets(&snippets).map_err(SyncError::InvalidConfig)?;

    // 3. Styles : fusion par signature d'applications et titres visés.
    let (styles, added, updated) = merge_styles(&local.styles, &remote_safe.styles, prefer_remote);
    report.styles_added = added;
    report.styles_updated = updated;
    merged.styles = clean_styles(&styles).map_err(SyncError::InvalidConfig)?;

    // 4. Prompts personnalisés : fusion par (catégorie, nom insensible à la casse).
    let (prompts, added, updated) =
        merge_prompts(&local.prompts, &remote_safe.prompts, prefer_remote);
    report.prompts_added = added;
    report.prompts_updated = updated;
    merged.prompts = clean_prompts(&prompts).map_err(SyncError::InvalidConfig)?;

    // 5. Profils LLM : fusion par nom insensible à la casse (sans secrets).
    let (profiles, added, updated) = merge_profiles(
        &local.llm_profiles,
        &remote_safe.llm_profiles,
        prefer_remote,
    );
    report.profiles_added = added;
    report.profiles_updated = updated;
    merged.llm_profiles = clean_llm_profiles(&profiles).map_err(SyncError::InvalidConfig)?;

    Ok((merged, report))
}

fn merge_words(local: &[String], remote: &[String], prefer_remote: bool) -> (Vec<String>, usize) {
    let mut by_key: BTreeMap<String, String> = BTreeMap::new();
    let mut local_keys = HashSet::new();
    for w in local.iter().map(|w| w.trim()).filter(|w| !w.is_empty()) {
        let key = w.to_lowercase();
        local_keys.insert(key.clone());
        by_key.entry(key).or_insert_with(|| w.to_owned());
    }
    let mut added = 0;
    for w in remote.iter().map(|w| w.trim()).filter(|w| !w.is_empty()) {
        let key = w.to_lowercase();
        if !local_keys.contains(&key) && !by_key.contains_key(&key) {
            added += 1;
            by_key.insert(key, w.to_owned());
        } else if prefer_remote {
            by_key.insert(key, w.to_owned());
        }
    }
    (by_key.into_values().collect(), added)
}

fn merge_snippets(
    local: &[Snippet],
    remote: &[Snippet],
    prefer_remote: bool,
) -> (Vec<Snippet>, usize, usize) {
    let mut map: BTreeMap<String, Snippet> = BTreeMap::new();
    for s in local {
        let key = normalize_phrase(&s.trigger);
        if !key.is_empty() {
            map.insert(key, s.clone());
        }
    }
    let mut added = 0;
    let mut updated = 0;
    for s in remote {
        let key = normalize_phrase(&s.trigger);
        if key.is_empty() {
            continue;
        }
        match map.get_mut(&key) {
            None => {
                added += 1;
                map.insert(key, s.clone());
            }
            Some(existing) if existing != s && prefer_remote => {
                updated += 1;
                *existing = s.clone();
            }
            _ => {}
        }
    }
    (map.into_values().collect(), added, updated)
}

fn style_signature(style: &Style) -> (Vec<String>, Vec<String>) {
    let mut apps: Vec<String> = style
        .apps
        .iter()
        .map(|a| {
            let trimmed = a.trim().to_lowercase();
            trimmed.strip_suffix(".exe").unwrap_or(&trimmed).to_owned()
        })
        .filter(|a| !a.is_empty())
        .collect();
    apps.sort();
    apps.dedup();

    let mut titles: Vec<String> = style
        .titles
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    titles.sort();
    titles.dedup();
    (apps, titles)
}

fn merge_styles(
    local: &[Style],
    remote: &[Style],
    prefer_remote: bool,
) -> (Vec<Style>, usize, usize) {
    let mut map: BTreeMap<(Vec<String>, Vec<String>), Style> = BTreeMap::new();
    for s in local {
        let sig = style_signature(s);
        if !sig.0.is_empty() || !sig.1.is_empty() {
            map.insert(sig, s.clone());
        }
    }
    let mut added = 0;
    let mut updated = 0;
    for s in remote {
        let sig = style_signature(s);
        if sig.0.is_empty() && sig.1.is_empty() {
            continue;
        }
        match map.get_mut(&sig) {
            None => {
                added += 1;
                map.insert(sig, s.clone());
            }
            Some(existing) if existing != s && prefer_remote => {
                updated += 1;
                *existing = s.clone();
            }
            _ => {}
        }
    }
    (map.into_values().collect(), added, updated)
}

fn prompt_kind_order(kind: PromptKind) -> u8 {
    match kind {
        PromptKind::Nettoyage => 0,
        PromptKind::Commande => 1,
        PromptKind::CompteRendu => 2,
        PromptKind::Conference => 3,
        PromptKind::Rapport => 4,
    }
}

fn merge_prompts(
    local: &[PromptPreset],
    remote: &[PromptPreset],
    prefer_remote: bool,
) -> (Vec<PromptPreset>, usize, usize) {
    let mut map: BTreeMap<(u8, String), PromptPreset> = BTreeMap::new();
    for p in local {
        let name = p.name.trim().to_lowercase();
        if !name.is_empty() {
            map.insert((prompt_kind_order(p.kind), name), p.clone());
        }
    }
    let mut added = 0;
    let mut updated = 0;
    for p in remote {
        let name = p.name.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        let key = (prompt_kind_order(p.kind), name);
        match map.get_mut(&key) {
            None => {
                added += 1;
                map.insert(key, p.clone());
            }
            Some(existing) if existing != p && prefer_remote => {
                updated += 1;
                *existing = p.clone();
            }
            _ => {}
        }
    }
    (map.into_values().collect(), added, updated)
}

fn merge_profiles(
    local: &[LlmProfile],
    remote: &[LlmProfile],
    prefer_remote: bool,
) -> (Vec<LlmProfile>, usize, usize) {
    let mut map: BTreeMap<String, LlmProfile> = BTreeMap::new();
    for p in local {
        let key = p.name.trim().to_lowercase();
        if !key.is_empty() {
            map.insert(key, p.clone());
        }
    }
    let mut added = 0;
    let mut updated = 0;
    for p in remote {
        let key = p.name.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        match map.get_mut(&key) {
            None => {
                added += 1;
                map.insert(key, p.clone());
            }
            Some(existing) if existing != p && prefer_remote => {
                updated += 1;
                *existing = p.clone();
            }
            _ => {}
        }
    }
    (map.into_values().collect(), added, updated)
}

/// Résumé léger d'une réunion échangé avant de transférer les fiches manquantes
/// ou plus récentes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingDigest {
    pub id: String,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub content_hash: String,
    pub has_summary: bool,
    pub has_report: bool,
}

/// Résultat de la comparaison des catalogues de réunions entre deux appareils.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingDiff {
    /// Identifiants des réunions locales à envoyer au pair distant.
    pub to_send: Vec<String>,
    /// Identifiants des réunions distantes à demander au pair distant.
    pub to_request: Vec<String>,
}

/// Prépare une fiche de réunion pour l'envoi sur un autre appareil (retire le
/// chemin absolu local `source_path` et les états transitoires en cours).
pub fn sanitize_meeting_for_sync(record: &MeetingRecord) -> MeetingRecord {
    let mut clean = record.clone();
    clean.source_path = None;
    clean.progress = None;
    clean.identifying = false;
    clean
}

/// Calcule l'empreinte et l'horodatage de mise à jour d'une fiche de réunion.
pub fn meeting_digest(record: &MeetingRecord) -> MeetingDigest {
    let portable = sanitize_meeting_for_sync(record);
    let json = serde_json::to_vec(&portable).unwrap_or_default();
    let digest = Sha256::digest(&json);
    let mut content_hash = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(content_hash, "{b:02x}");
    }
    let mut updated_at_ms = record.started_at_ms.saturating_add(record.duration_ms);
    if let Some(summary) = &record.summary {
        updated_at_ms = updated_at_ms.max(summary.at_ms);
    }
    if let Some(report) = &record.report {
        updated_at_ms = updated_at_ms.max(report.at_ms);
    }
    MeetingDigest {
        id: record.id.clone(),
        started_at_ms: record.started_at_ms,
        updated_at_ms,
        content_hash,
        has_summary: record.summary.is_some(),
        has_report: record.report.is_some(),
    }
}

/// Compare les catalogues de réunions local et distant de manière symétrique.
pub fn diff_meetings(local: &[MeetingDigest], remote: &[MeetingDigest]) -> MeetingDiff {
    let local_map: BTreeMap<&str, &MeetingDigest> =
        local.iter().map(|d| (d.id.as_str(), d)).collect();
    let remote_map: BTreeMap<&str, &MeetingDigest> =
        remote.iter().map(|d| (d.id.as_str(), d)).collect();

    let mut diff = MeetingDiff::default();
    for (id, l) in &local_map {
        match remote_map.get(id) {
            None => diff.to_send.push((*id).to_owned()),
            Some(r) if l.content_hash != r.content_hash => {
                if (l.updated_at_ms, l.content_hash.as_str())
                    > (r.updated_at_ms, r.content_hash.as_str())
                {
                    diff.to_send.push((*id).to_owned());
                } else {
                    diff.to_request.push((*id).to_owned());
                }
            }
            Some(_) => {}
        }
    }
    for id in remote_map.keys() {
        if !local_map.contains_key(id) {
            diff.to_request.push((*id).to_owned());
        }
    }
    diff
}

/// Messages applicatifs échangés à l'intérieur du canal chiffré [`super::SessionCipher`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncMessage {
    /// Remise de la nouvelle clé permanente dédiée au pair lors d'un appairage QR.
    PairGrant {
        #[serde(rename = "key")]
        key_b64: String,
    },
    /// Confirmation de l'appairage par l'initiateur.
    PairAck,
    /// Instantané de la configuration portable (sans clés API).
    ConfigSnapshot {
        updated_at_ms: u64,
        device_id: DeviceId,
        payload: SyncableConfig,
    },
    /// Catalogue des réunions disponibles sur cet appareil, accompagné des
    /// statistiques d'activité par appareil (facultatif pour rétrocompatibilité).
    MeetingIndex {
        digests: Vec<MeetingDigest>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<crate::usage::UsageBundle>,
    },
    /// Demande des fiches manquantes ou plus récentes.
    MeetingRequest { ids: Vec<String> },
    /// Transfert d'une fiche de réunion complète (sans chemin local).
    MeetingTransfer { record: Box<MeetingRecord> },
    /// Fin normale de la synchronisation.
    Done { merged_items: usize },
    /// Erreur applicative transmise au pair.
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::{LlmConfig, MeetingAudio};
    use crate::meeting::{MeetingKind, MeetingStatus, MeetingSummary, Output};

    fn sample_meeting(id: &str, started_at_ms: u64, title: &str) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: title.into(),
            started_at_ms,
            duration_ms: 60_000,
            status: MeetingStatus::Terminee,
            audio: MeetingAudio::Conserve,
            system: true,
            segments: Vec::new(),
            summary: None,
            summary_error: None,
            failed_chunks: 0,
            notes: Vec::new(),
            echo_filter: true,
            echo_chunks: 0,
            kind: MeetingKind::Reunion,
            output: Output::LesDeux,
            mic: true,
            source: None,
            source_path: Some("C:\\Users\\Alice\\appel.mp3".into()),
            progress: None,
            speaker_track: None,
            speaker_turns: Vec::new(),
            speaker_names: Vec::new(),
            speaker_guesses: Vec::new(),
            speaker_recognized: Vec::new(),
            speaker_count: None,
            marks: Vec::new(),
            conference: Some(false),
            report: None,
            report_error: None,
            identifying: false,
        }
    }

    #[test]
    fn syncable_config_strips_secret_extra_body_and_preserves_local_hardware_settings() {
        let mut local = Config::default();
        local.hotkey.key = "F13".into();
        local.audio.device = "Micro Shure MV7".into();
        local.asr.model = "ggml-large-v3-turbo-q8_0.bin".into();
        local.dictionary = vec!["Aquadrox".into()];

        let mut remote_cfg = Config::default();
        remote_cfg.hotkey.key = "CapsLock".into();
        remote_cfg.audio.device = "Micro Android".into();
        remote_cfg.asr.model = "parakeet-tdt-0.6b-v3-int8".into();
        remote_cfg.dictionary = vec!["aquadrox".into(), "Parakeet".into()];
        let mut prof_llm = LlmConfig::default();
        prof_llm
            .extra_body
            .insert("api_key".into(), json!("sk-never-sync"));
        prof_llm.extra_body.insert("temperature".into(), json!(0.1));
        remote_cfg.llm_profiles.push(LlmProfile {
            name: "Rapide".into(),
            llm: prof_llm,
        });

        // Simule même un pair qui n'aurait pas filtré `extra_body` avant envoi.
        let unstripped_remote = SyncableConfig {
            dictionary: remote_cfg.dictionary.clone(),
            dictionary_ignored: Vec::new(),
            snippets: Vec::new(),
            styles: Vec::new(),
            prompts: Vec::new(),
            llm_profiles: remote_cfg.llm_profiles.clone(),
        };

        let id_a = DeviceId::parse("11111111111111111111111111111111").unwrap();
        let id_b = DeviceId::parse("22222222222222222222222222222222").unwrap();

        let (merged, report) =
            merge_config(&local, 1000, &id_a, &unstripped_remote, 2000, &id_b).unwrap();

        // Réglages locaux intacts.
        assert_eq!(merged.hotkey.key, "F13");
        assert_eq!(merged.audio.device, "Micro Shure MV7");
        assert_eq!(merged.asr.model, "ggml-large-v3-turbo-q8_0.bin");

        // Secret retiré et signalé dans le rapport.
        assert!(!report.stripped_secrets.is_empty());
        assert_eq!(merged.llm_profiles.len(), 1);
        assert!(
            !merged.llm_profiles[0]
                .llm
                .extra_body
                .contains_key("api_key")
        );
        assert_eq!(
            merged.llm_profiles[0].llm.extra_body.get("temperature"),
            Some(&json!(0.1))
        );

        // Union dictionnaire.
        assert_eq!(report.dictionary_added, 1);
        assert_eq!(merged.dictionary.len(), 2);
        assert!(merged.dictionary.iter().any(|w| w == "Parakeet"));
    }

    #[test]
    fn merge_converges_identically_on_both_devices_even_on_conflicts_and_equal_timestamps() {
        let id_a = DeviceId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let id_b = DeviceId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        let cfg_a = Config {
            dictionary: vec!["Tauri".into(), "Whisper".into()],
            snippets: vec![
                Snippet {
                    trigger: "ma signature".into(),
                    text: "Cordialement, A".into(),
                },
                Snippet {
                    trigger: "adresse".into(),
                    text: "Paris".into(),
                },
            ],
            styles: vec![Style {
                apps: vec!["Slack.exe".into()],
                titles: Vec::new(),
                instructions: "Court A".into(),
                cleanup: true,
            }],
            prompts: vec![PromptPreset {
                name: "Pro".into(),
                kind: PromptKind::Nettoyage,
                text: "Prompt A".into(),
            }],
            ..Default::default()
        };

        let cfg_b = Config {
            dictionary: vec!["whisper".into(), "Parakeet".into()],
            snippets: vec![
                Snippet {
                    trigger: "Ma signature !".into(), // même déclencheur normalisé -> conflit
                    text: "Bien à vous, B".into(),
                },
                Snippet {
                    trigger: "tel".into(),
                    text: "06 00 00 00 00".into(),
                },
            ],
            styles: vec![Style {
                apps: vec!["slack".into()], // même signature sans .exe -> conflit
                titles: Vec::new(),
                instructions: "Court B".into(),
                cleanup: true,
            }],
            prompts: vec![PromptPreset {
                name: "pro".into(),
                kind: PromptKind::Nettoyage,
                text: "Prompt B".into(),
            }],
            ..Default::default()
        };

        let (snap_a, _) = SyncableConfig::from_config(&cfg_a);
        let (snap_b, _) = SyncableConfig::from_config(&cfg_b);

        // Même horodatage exact : le départage déterministe sur device_id (B > A) doit
        // donner exactement la même vue fusionnée des deux côtés.
        let (merged_on_a, _) = merge_config(&cfg_a, 5000, &id_a, &snap_b, 5000, &id_b).unwrap();
        let (merged_on_b, _) = merge_config(&cfg_b, 5000, &id_b, &snap_a, 5000, &id_a).unwrap();

        assert_eq!(merged_on_a.dictionary, merged_on_b.dictionary);
        assert_eq!(merged_on_a.snippets, merged_on_b.snippets);
        assert_eq!(merged_on_a.styles, merged_on_b.styles);
        assert_eq!(merged_on_a.prompts, merged_on_b.prompts);

        // B a gagné le conflit de signature et les deux snippets uniques sont conservés.
        assert_eq!(merged_on_a.snippets.len(), 3);
        let sig = merged_on_a
            .snippets
            .iter()
            .find(|s| normalize_phrase(&s.trigger) == "ma signature")
            .unwrap();
        assert_eq!(sig.text, "Bien à vous, B");
    }

    #[test]
    fn meeting_digest_ignores_local_source_path_and_diff_is_symmetric() {
        let m1_a = sample_meeting("m1", 1000, "Point produit");
        let mut m1_b = m1_a.clone();
        m1_b.source_path = Some("/data/user/0/appel.mp3".into());
        // Même réunion avec source_path différent -> même empreinte, rien à transférer.
        assert_eq!(
            meeting_digest(&m1_a).content_hash,
            meeting_digest(&m1_b).content_hash
        );

        let m2_local_only = sample_meeting("m2", 2000, "Locale");
        let m3_old = sample_meeting("m3", 3000, "Version ancienne");
        let mut m3_new = m3_old.clone();
        m3_new.summary = Some(MeetingSummary {
            text: "Résumé ajouté".into(),
            at_ms: 999_999,
            model: "deepseek-flash".into(),
            cost: None,
            currency: "USD".into(),
        });
        let m4_remote_only = sample_meeting("m4", 4000, "Distante");

        let local = vec![
            meeting_digest(&m1_a),
            meeting_digest(&m2_local_only),
            meeting_digest(&m3_old),
        ];
        let remote = vec![
            meeting_digest(&m1_b),
            meeting_digest(&m3_new),
            meeting_digest(&m4_remote_only),
        ];

        let diff_a = diff_meetings(&local, &remote);
        let diff_b = diff_meetings(&remote, &local);

        assert_eq!(diff_a.to_send, vec!["m2"]);
        assert_eq!(diff_a.to_request, vec!["m3", "m4"]);
        // Symétrie parfaite entre les deux appareils.
        assert_eq!(diff_a.to_send, diff_b.to_request);
        assert_eq!(diff_a.to_request, diff_b.to_send);
    }
}
