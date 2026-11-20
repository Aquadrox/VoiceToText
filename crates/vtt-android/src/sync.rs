//! Pont Rust Android pour la synchronisation locale Wi-Fi (`vtt_core::sync`).
//!
//! Utilise le même format disque que le PC (`config.toml`, `sync_peers.json`
//! et `<meetings>/<id>/meeting.json`). Les clés API Android (AndroidKeyStore)
//! ne quittent jamais le téléphone.
//!
//! Inclut aussi un serveur temporaire d'appairage direct par clic + code à
//! 6 chiffres (actif uniquement tant que l'écran Synchronisation est ouvert).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use vtt_core::sync::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_PORT, DeviceId, DirectDecisionHandle, DiskSyncStorage,
    PairingOffer, Peer, PendingDirectPairing, SyncEvent, SyncOptions, SyncServer, SyncStorage,
    SyncSummary, now_ms, pair_over_tcp, start_direct_pairing_over_tcp, sync_with_peer,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListenerInfo {
    pub port: u16,
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingDirectPairingView {
    pub session_id: String,
    pub remote_device_id: String,
    pub remote_device_name: String,
    pub code: String,
    pub formatted_code: String,
    pub expires_at_ms: u64,
    pub incoming: bool,
    pub local_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectStatusEvent {
    pub seq: u64,
    /// `"paired"`, `"synced"` ou `"ended"`.
    pub kind: String,
    pub message: String,
    pub config_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectStateView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listening_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingDirectPairingView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<DirectStatusEvent>,
}

struct OutgoingDirectSlot {
    pending: PendingDirectPairing,
    decision: DirectDecisionHandle,
    local_approved: bool,
}

#[derive(Default)]
struct DirectRuntime {
    server: Option<SyncServer>,
    outgoing: Option<OutgoingDirectSlot>,
    incoming_approved_session: Option<String>,
    last_event: Option<DirectStatusEvent>,
    next_seq: u64,
}

impl DirectRuntime {
    fn record_event(&mut self, kind: &str, message: String, config_changed: bool) {
        self.next_seq = self.next_seq.saturating_add(1);
        self.last_event = Some(DirectStatusEvent {
            seq: self.next_seq,
            kind: kind.to_owned(),
            message,
            config_changed,
        });
    }

    fn clear_expired_outgoing(&mut self) {
        if self
            .outgoing
            .as_ref()
            .is_some_and(|slot| now_ms() >= slot.pending.expires_at_ms)
            && let Some(old) = self.outgoing.take()
        {
            old.decision.reject();
        }
    }

    fn snapshot(&mut self) -> DirectStateView {
        self.clear_expired_outgoing();
        let listening_port = self.server.as_ref().map(SyncServer::port);
        let pending = if let Some(slot) = self.outgoing.as_ref() {
            Some(pending_to_view(&slot.pending, slot.local_approved))
        } else {
            self.server
                .as_ref()
                .and_then(SyncServer::pending_direct_pairing)
                .map(|p| {
                    let approved =
                        self.incoming_approved_session.as_deref() == Some(p.session_id.as_str());
                    pending_to_view(&p, approved)
                })
        };
        DirectStateView {
            listening_port,
            pending,
            last_event: self.last_event.clone(),
        }
    }
}

static DIRECT_RUNTIME: Mutex<Option<DirectRuntime>> = Mutex::new(None);

fn with_runtime<T>(f: impl FnOnce(&mut DirectRuntime) -> T) -> T {
    let mut guard = DIRECT_RUNTIME.lock().unwrap_or_else(|e| e.into_inner());
    let rt = guard.get_or_insert_with(DirectRuntime::default);
    f(rt)
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

fn pending_to_view(
    pending: &PendingDirectPairing,
    local_approved: bool,
) -> PendingDirectPairingView {
    PendingDirectPairingView {
        session_id: pending.session_id.clone(),
        remote_device_id: pending.remote_device_id.as_str().to_owned(),
        remote_device_name: pending.remote_device_name.clone(),
        code: pending.code.clone(),
        formatted_code: pending.formatted_code(),
        expires_at_ms: pending.expires_at_ms,
        incoming: pending.incoming,
        local_approved,
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

/// Démarre le serveur TCP temporaire d'appairage direct (uniquement tant que
/// l'écran Synchronisation Wi-Fi est affiché). Retourne le port d'écoute et
/// l'identité locale pour l'annonce `NsdManager`.
pub fn start_listener(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
) -> Result<ListenerInfo, String> {
    let storage = Arc::new(open_storage(
        config_path,
        peers_path,
        meetings_dir,
        default_name,
    ));
    let identity = storage.identity().map_err(|e| e.to_string())?;

    if let Some(port) = with_runtime(|rt| {
        if let Some(server) = rt.server.as_ref() {
            server.set_direct_pairing_enabled(true);
            return Some(server.port());
        }
        None
    }) {
        return Ok(ListenerInfo {
            port,
            device_id: identity.device_id.as_str().to_owned(),
            device_name: identity.device_name,
        });
    }

    let on_event: Arc<dyn Fn(SyncEvent) + Send + Sync> = Arc::new(|event: SyncEvent| {
        with_runtime(|rt| match event {
            SyncEvent::Paired { peer } => {
                rt.incoming_approved_session = None;
                rt.record_event(
                    "paired",
                    format!("Appareil « {} » appairé.", peer.device_name),
                    false,
                );
            }
            SyncEvent::Synced { summary } => {
                let msg = format_summary_message(&summary);
                rt.record_event("synced", msg, summary.config_changed);
            }
            SyncEvent::DirectPairingRequested { .. } => {}
            SyncEvent::DirectPairingEnded { session_id, reason } => {
                if rt.incoming_approved_session.as_deref() == Some(session_id.as_str()) {
                    rt.incoming_approved_session = None;
                }
                let msg = reason
                    .map(|r| format!("Appairage direct interrompu : {r}"))
                    .unwrap_or_else(|| "Appairage direct annulé.".into());
                rt.record_event("ended", msg, false);
            }
            SyncEvent::ConnectionFailed { reason } => {
                log::warn!("sync android: connexion entrante refusée : {reason}");
            }
        });
    });

    let dyn_storage: Arc<dyn SyncStorage> = storage;
    let preferred = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT);
    let server = SyncServer::start(
        preferred,
        Arc::clone(&dyn_storage),
        Some(Arc::clone(&on_event)),
    )
    .or_else(|_| {
        let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        SyncServer::start(fallback, dyn_storage, Some(on_event))
    })
    .map_err(|e| e.to_string())?;
    server.set_direct_pairing_enabled(true);
    let port = server.port();

    with_runtime(|rt| {
        if let Some(old) = rt.server.replace(server) {
            old.stop();
        }
    });

    Ok(ListenerInfo {
        port,
        device_id: identity.device_id.as_str().to_owned(),
        device_name: identity.device_name,
    })
}

/// Arrête immédiatement le serveur TCP temporaire et rejette toute demande
/// d'appairage direct en attente (appelé dès que la page Synchronisation est quittée).
pub fn stop_listener() {
    let old_server = with_runtime(|rt| {
        if let Some(slot) = rt.outgoing.take() {
            slot.decision.reject();
        }
        rt.incoming_approved_session = None;
        rt.server.take()
    });
    if let Some(server) = old_server {
        server.stop();
    }
}

/// Lance un appairage direct vers un PC / appareil détecté : retourne aussitôt
/// le code à 6 chiffres à vérifier visuellement sur les deux écrans.
pub fn start_direct_pairing(
    config_path: &Path,
    peers_path: &Path,
    meetings_dir: &Path,
    default_name: &str,
    device_id: &str,
    addr: &str,
) -> Result<PendingDirectPairingView, String> {
    let trimmed_addr = addr.trim();
    if trimmed_addr.is_empty() {
        return Err("adresse de l'appareil cible vide".into());
    }
    let expected_remote = {
        let trimmed_id = device_id.trim();
        if trimmed_id.is_empty() {
            None
        } else {
            Some(DeviceId::parse(trimmed_id).map_err(|e| e.to_string())?)
        }
    };

    let local_listen_port = with_runtime(|rt| {
        rt.clear_expired_outgoing();
        if rt.outgoing.is_some()
            || rt
                .server
                .as_ref()
                .and_then(SyncServer::pending_direct_pairing)
                .is_some()
        {
            return Err("un appairage direct est déjà en cours".to_owned());
        }
        Ok(rt.server.as_ref().map(SyncServer::port))
    })?;

    let storage: Arc<dyn SyncStorage> = Arc::new(open_storage(
        config_path,
        peers_path,
        meetings_dir,
        default_name,
    ));

    let handle = start_direct_pairing_over_tcp(
        storage,
        &[trimmed_addr.to_owned()],
        expected_remote.as_ref(),
        local_listen_port,
        DEFAULT_CONNECT_TIMEOUT,
        None,
    )
    .map_err(|e| e.to_string())?;

    let pending = handle.pending().clone();
    let wait_session_id = pending.session_id.clone();
    let decision = handle.decision_handle();

    with_runtime(|rt| {
        rt.outgoing = Some(OutgoingDirectSlot {
            pending: pending.clone(),
            decision,
            local_approved: false,
        });
    });

    if let Err(e) = thread::Builder::new()
        .name("vtt-android-direct-wait".into())
        .spawn(move || {
            let res = handle.wait();
            with_runtime(|rt| {
                if rt
                    .outgoing
                    .as_ref()
                    .is_some_and(|slot| slot.pending.session_id == wait_session_id)
                {
                    rt.outgoing = None;
                }
                match res {
                    Ok(peer) => rt.record_event(
                        "paired",
                        format!("Appareil « {} » appairé.", peer.device_name),
                        false,
                    ),
                    Err(e) => {
                        rt.record_event("ended", format!("Appairage direct annulé : {e}"), false)
                    }
                }
            });
        })
    {
        with_runtime(|rt| {
            if let Some(old) = rt.outgoing.take() {
                old.decision.reject();
            }
        });
        return Err(format!("impossible de suivre l'appairage direct : {e}"));
    }

    Ok(pending_to_view(&pending, false))
}

/// Approuve (`approve = true`) ou refuse (`approve = false`) la demande
/// d'appairage direct en cours (entrante ou sortante).
pub fn respond_direct_pairing(session_id: &str, approve: bool) -> Result<DirectStateView, String> {
    with_runtime(|rt| {
        rt.clear_expired_outgoing();
        if let Some(slot) = rt.outgoing.as_mut()
            && slot.pending.session_id == session_id
        {
            slot.decision.decide(approve);
            if approve {
                slot.local_approved = true;
            } else {
                rt.outgoing = None;
            }
            return Ok(rt.snapshot());
        }

        let server = rt
            .server
            .as_ref()
            .ok_or_else(|| "aucune demande d'appairage active".to_owned())?;
        server
            .respond_direct_pairing(session_id, approve)
            .map_err(|e| e.to_string())?;
        if approve {
            rt.incoming_approved_session = Some(session_id.to_owned());
        } else if rt.incoming_approved_session.as_deref() == Some(session_id) {
            rt.incoming_approved_session = None;
        }
        Ok(rt.snapshot())
    })
}

/// État mémoire courant de l'appairage direct (aucune E/S disque ni réseau).
pub fn direct_state() -> DirectStateView {
    with_runtime(DirectRuntime::snapshot)
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use vtt_core::Config;
    use vtt_core::config::MeetingAudio;
    use vtt_core::meeting::{MeetingKind, MeetingRecord, MeetingStatus, Output};
    use vtt_core::sync::SyncServer;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    static DIRECT_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn direct_pairing_works_in_both_directions_phone_to_pc_and_pc_to_phone() {
        let _lock = DIRECT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        stop_listener();

        // --- Sens 1 : téléphone clique sur le PC ---
        let pc_dir = temp_dir("direct_s1_pc");
        let phone_dir = temp_dir("direct_s1_phone");
        let pc_storage: Arc<dyn SyncStorage> = Arc::new(DiskSyncStorage::new(
            pc_dir.join("config.toml"),
            pc_dir.join("sync_peers.json"),
            pc_dir.join("meetings"),
            "PC Bureau",
        ));
        let (tx, rx) = mpsc::channel();
        let cb: Arc<dyn Fn(SyncEvent) + Send + Sync> = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let pc_server =
            SyncServer::start("127.0.0.1:0".parse().unwrap(), pc_storage.clone(), Some(cb))
                .unwrap();
        pc_server.set_direct_pairing_enabled(true);
        let pc_addr = pc_server.local_addr().to_string();
        let pc_id = pc_storage.identity().unwrap().device_id;

        let phone_cfg = phone_dir.join("config.toml");
        let phone_peers = phone_dir.join("sync_peers.json");
        let phone_meetings = phone_dir.join("meetings");

        let phone_pending = start_direct_pairing(
            &phone_cfg,
            &phone_peers,
            &phone_meetings,
            "Xiaomi 17T Pro",
            pc_id.as_str(),
            &pc_addr,
        )
        .unwrap();
        assert!(!phone_pending.incoming);
        assert_eq!(phone_pending.formatted_code.len(), 7);

        let pc_pending = match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
            SyncEvent::DirectPairingRequested { pending } => pending,
            other => panic!("attendu DirectPairingRequested, reçu {other:?}"),
        };
        assert_eq!(phone_pending.code, pc_pending.code);

        respond_direct_pairing(&phone_pending.session_id, true).unwrap();
        pc_server
            .respond_direct_pairing(&pc_pending.session_id, true)
            .unwrap();

        // Attend que l'événement "paired" arrive côté Android.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let st = direct_state();
            if st.last_event.as_ref().is_some_and(|e| e.kind == "paired") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "timeout paired");
            thread::sleep(Duration::from_millis(25));
        }
        let phone_view = view(&phone_cfg, &phone_peers, &phone_meetings, "Android").unwrap();
        assert_eq!(phone_view.peers.len(), 1);
        assert_eq!(phone_view.peers[0].device_id, pc_id.as_str());
        pc_server.stop();

        // --- Sens 2 : PC clique sur le téléphone (listener temporaire Android) ---
        let pc2_dir = temp_dir("direct_s2_pc");
        let phone2_dir = temp_dir("direct_s2_phone");
        let pc2_storage: Arc<dyn SyncStorage> = Arc::new(DiskSyncStorage::new(
            pc2_dir.join("config.toml"),
            pc2_dir.join("sync_peers.json"),
            pc2_dir.join("meetings"),
            "PC Salon",
        ));
        let p2_cfg = phone2_dir.join("config.toml");
        let p2_peers = phone2_dir.join("sync_peers.json");
        let p2_meetings = phone2_dir.join("meetings");

        let info = start_listener(&p2_cfg, &p2_peers, &p2_meetings, "Pixel 9").unwrap();
        assert!(info.port > 0);
        let phone_addr = format!("127.0.0.1:{}", info.port);
        let phone2_id = DeviceId::parse(&info.device_id).unwrap();

        let pc_handle = start_direct_pairing_over_tcp(
            Arc::clone(&pc2_storage),
            &[phone_addr],
            Some(&phone2_id),
            None,
            Duration::from_secs(3),
            Some(10_000),
        )
        .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let incoming_phone = loop {
            if let Some(p) = direct_state().pending {
                break p;
            }
            assert!(std::time::Instant::now() < deadline, "timeout incoming");
            thread::sleep(Duration::from_millis(25));
        };
        assert!(incoming_phone.incoming);
        assert_eq!(incoming_phone.code, pc_handle.pending().code);

        let pc_decision = pc_handle.decision_handle();
        let pc_wait = thread::spawn(move || pc_handle.wait());
        respond_direct_pairing(&incoming_phone.session_id, true).unwrap();
        pc_decision.approve();

        let paired_by_pc = pc_wait.join().unwrap().unwrap();
        assert_eq!(paired_by_pc.device_id, phone2_id);

        stop_listener();
        assert!(direct_state().listening_port.is_none());

        let _ = fs::remove_dir_all(&pc_dir);
        let _ = fs::remove_dir_all(&phone_dir);
        let _ = fs::remove_dir_all(&pc2_dir);
        let _ = fs::remove_dir_all(&phone2_dir);
    }
}
