//! Pont Rust Android pour la synchronisation locale Wi-Fi (`vtt_core::sync`).
//!
//! Utilise le même format disque que le PC (`config.toml`, `sync_peers.json`
//! et `<meetings>/<id>/meeting.json`). Les clés API Android (AndroidKeyStore)
//! ne quittent jamais le téléphone.

use std::path::Path;

use serde::Serialize;
use vtt_core::sync::{
    DEFAULT_CONNECT_TIMEOUT, DeviceId, DiskSyncStorage, PairingOffer, Peer, SyncOptions,
    SyncStorage, SyncSummary, pair_over_tcp, sync_with_peer,
};

#[derive(Debug, Clone, Serialize)]
pub struct PeerView {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub paired_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_addr: Option<String>,
    pub sync_settings: bool,
    pub sync_meetings: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncView {
    pub device_id: String,
    pub device_name: String,
    pub peers: Vec<PeerView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncSummaryView {
    pub peer_id: String,
    pub peer_name: String,
    pub config_changed: bool,
    pub meetings_sent: usize,
    pub meetings_received: usize,
    pub message: String,
    pub at_ms: u64,
}

fn open_storage(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
) -> DiskSyncStorage {
    let fallback = if default_name.trim().is_empty() {
        "Android"
    } else {
        default_name
    };
    DiskSyncStorage::new(
        config_path.to_path_buf(),
        peers_path.to_path_buf(),
        meetings_dir.to_path_buf(),
        fallback,
    )
}

fn peer_to_view(peer: &Peer) -> PeerView {
    PeerView {
        device_id: peer.device_id.as_str().to_owned(),
        device_name: peer.device_name.clone(),
        fingerprint: peer.fingerprint(),
        paired_at_ms: peer.paired_at_ms,
        last_sync_ms: peer.last_sync_ms,
        last_addr: peer.last_addr.clone(),
        sync_settings: peer.sync_settings,
        sync_meetings: peer.sync_meetings,
    }
}

/// État courant de la synchronisation : identité du téléphone et appareils appairés.
pub fn view(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
) -> Result<SyncView, String> {
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    let identity = storage.identity().map_err(|e| e.to_string())?;
    let peers = storage.peers().map_err(|e| e.to_string())?;
    Ok(SyncView {
        device_id: identity.device_id.as_str().to_owned(),
        device_name: identity.device_name,
        peers: peers.iter().map(peer_to_view).collect(),
    })
}

/// Renomme le téléphone dans le registre persistant.
pub fn rename_device(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    name: &str,
) -> Result<SyncView, String> {
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    storage
        .rename_local_device(name)
        .map_err(|e| e.to_string())?;
    view(config_path, peers_path, meetings_dir, default_name)
}

/// Appaire le téléphone avec un PC/Steam Deck à partir de son URI QR `vttsync://v1?...`.
pub fn pair_uri(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    uri: &str,
) -> Result<PeerView, String> {
    let offer = PairingOffer::from_uri(uri.trim()).map_err(|e| e.to_string())?;
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    let peer =
        pair_over_tcp(&storage, &offer, DEFAULT_CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    Ok(peer_to_view(&peer))
}

/// Lance une synchronisation chiffrée bidirectionnelle avec un appareil appairé.
///
/// `addr_override` (vide si absent) permet d'utiliser l'adresse découverte en
/// temps réel par `NsdManager` ou une adresse saisie manuellement `IP:port`.
pub fn sync_now(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    device_id: &str,
    addr_override: &str,
) -> Result<SyncSummaryView, String> {
    let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    let target = {
        let trimmed = addr_override.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };
    let summary = sync_with_peer(
        &storage,
        &id,
        target,
        SyncOptions::default(),
        DEFAULT_CONNECT_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;

    let message = format_summary_message(&summary);
    Ok(SyncSummaryView {
        peer_id: summary.peer_id.as_str().to_owned(),
        peer_name: summary.peer_name,
        config_changed: summary.config_changed,
        meetings_sent: summary.meetings_sent,
        meetings_received: summary.meetings_received,
        message,
        at_ms: summary.at_ms,
    })
}

/// Met à jour les préférences (réglages / réunions) d'un appareil appairé.
pub fn set_peer_prefs(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    device_id: &str,
    sync_settings: bool,
    sync_meetings: bool,
) -> Result<bool, String> {
    let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    storage
        .set_peer_preferences(&id, sync_settings, sync_meetings)
        .map_err(|e| e.to_string())
}

/// Oublie un appareil appairé et révoque sa clé.
pub fn remove_peer(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    device_id: &str,
) -> Result<bool, String> {
    let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
    let storage = open_storage(config_path, peers_path, meetings_dir, default_name);
    storage.remove_peer(&id).map_err(|e| e.to_string())
}

pub fn format_summary_message(summary: &SyncSummary) -> String {
    let mut parts = Vec::new();
    if summary.config_changed {
        parts.push("réglages fusionnés".to_owned());
    }
    if summary.meetings_received > 0 {
        parts.push(format!(
            "{} réunion{} reçue{}",
            summary.meetings_received,
            if summary.meetings_received > 1 {
                "s"
            } else {
                ""
            },
            if summary.meetings_received > 1 {
                "s"
            } else {
                ""
            },
        ));
    }
    if summary.meetings_sent > 0 {
        parts.push(format!(
            "{} réunion{} envoyée{}",
            summary.meetings_sent,
            if summary.meetings_sent > 1 { "s" } else { "" },
            if summary.meetings_sent > 1 { "s" } else { "" },
        ));
    }
    if parts.is_empty() {
        format!("{} : déjà à jour.", summary.peer_name)
    } else {
        format!("{} : {}.", summary.peer_name, parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use vtt_core::Config;
    use vtt_core::config::MeetingAudio;
    use vtt_core::meeting::{MeetingKind, MeetingRecord, MeetingStatus, Output};
    use vtt_core::sync::SyncServer;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "vtt_android_sync_{label}_{}_{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_meeting(id: &str, title: &str) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: title.into(),
            started_at_ms: 100_000,
            duration_ms: 30_000,
            status: MeetingStatus::Terminee,
            audio: MeetingAudio::Conserve,
            system: false,
            segments: Vec::new(),
            summary: None,
            summary_error: None,
            failed_chunks: 0,
            notes: Vec::new(),
            echo_filter: false,
            echo_chunks: 0,
            kind: MeetingKind::Reunion,
            output: Output::LesDeux,
            mic: true,
            source: None,
            source_path: None,
            progress: None,
            speaker_track: None,
            speaker_turns: Vec::new(),
            speaker_names: Vec::new(),
            speaker_guesses: Vec::new(),
            speaker_count: None,
            marks: Vec::new(),
            conference: Some(false),
            report: None,
            report_error: None,
            identifying: false,
        }
    }

    #[test]
    fn view_initializes_identity_and_renames_device() {
        let dir = temp_dir("rename");
        let cfg = dir.join("config.toml");
        let peers = dir.join("sync_peers.json");
        let meetings = dir.join("meetings");

        let v1 = view(&cfg, &peers, &meetings, "Xiaomi 17T Pro").unwrap();
        assert_eq!(v1.device_name, "Xiaomi 17T Pro");
        assert!(v1.peers.is_empty());

        let v2 = rename_device(&cfg, &peers, &meetings, "Android", "Mon téléphone").unwrap();
        assert_eq!(v2.device_id, v1.device_id);
        assert_eq!(v2.device_name, "Mon téléphone");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pair_and_sync_over_loopback_tcp_with_desktop_server() {
        let pc_dir = temp_dir("pc");
        let phone_dir = temp_dir("phone");

        let pc_storage = Arc::new(DiskSyncStorage::new(
            pc_dir.join("config.toml"),
            pc_dir.join("sync_peers.json"),
            pc_dir.join("meetings"),
            "PC Bureau",
        ));
        pc_storage
            .save_config(&Config {
                dictionary: vec!["Aquadrox".into()],
                ..Default::default()
            })
            .unwrap();
        pc_storage
            .save_meeting(&sample_meeting("pc_1", "Point hebdo"))
            .unwrap();

        let server =
            SyncServer::start("127.0.0.1:0".parse().unwrap(), pc_storage.clone(), None).unwrap();
        let offer = server
            .open_pairing(vec![server.local_addr().to_string()], Some(60_000))
            .unwrap();

        let phone_cfg = phone_dir.join("config.toml");
        let phone_peers = phone_dir.join("sync_peers.json");
        let phone_meetings = phone_dir.join("meetings");

        let paired = pair_uri(
            &phone_cfg,
            &phone_peers,
            &phone_meetings,
            "Xiaomi 17T Pro",
            &offer.to_uri(),
        )
        .unwrap();
        assert_eq!(paired.device_name, "PC Bureau");

        let summary = sync_now(
            &phone_cfg,
            &phone_peers,
            &phone_meetings,
            "Xiaomi 17T Pro",
            &paired.device_id,
            "",
        )
        .unwrap();
        assert!(summary.config_changed);
        assert_eq!(summary.meetings_received, 1);
        assert!(summary.message.contains("réglages fusionnés"));
        assert!(summary.message.contains("1 réunion reçue"));

        assert!(
            set_peer_prefs(
                &phone_cfg,
                &phone_peers,
                &phone_meetings,
                "Xiaomi 17T Pro",
                &paired.device_id,
                true,
                false,
            )
            .unwrap()
        );
        let after = view(&phone_cfg, &phone_peers, &phone_meetings, "Android").unwrap();
        assert_eq!(after.peers.len(), 1);
        assert!(!after.peers[0].sync_meetings);

        assert!(
            remove_peer(
                &phone_cfg,
                &phone_peers,
                &phone_meetings,
                "Android",
                &paired.device_id,
            )
            .unwrap()
        );
        assert!(
            view(&phone_cfg, &phone_peers, &phone_meetings, "Android")
                .unwrap()
                .peers
                .is_empty()
        );

        server.stop();
        let _ = fs::remove_dir_all(&pc_dir);
        let _ = fs::remove_dir_all(&phone_dir);
    }
}
