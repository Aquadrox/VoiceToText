//! Stockage local et orchestration des sessions d'appairage et de
//! synchronisation (configuration portable + fiches de réunions `meeting.json`).

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::meeting::{MeetingRecord, MeetingStatus, RECORD_FILE};

use super::channel::EncryptedChannel;
use super::crypto::SyncKey;
use super::merge::{
    MeetingDigest, MergeReport, SyncMessage, SyncableConfig, diff_meetings, meeting_digest,
    merge_config, sanitize_meeting_for_sync,
};
use super::pairing::{
    DeviceId, LocalIdentity, PairingOffer, Peer, PeerStore, sanitize_device_name,
};
use super::{Result, SyncError};

/// Garde-fou sur le nombre de fiches transférées en une seule session.
const MAX_MEETINGS_PER_SESSION: usize = 10_000;

/// Vérifie qu'un identifiant de réunion ne peut jamais sortir du dossier
/// `meetings_dir` (protection contre la traversée de chemin).
pub fn valid_meeting_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Options de portée d'une session de synchronisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOptions {
    pub sync_settings: bool,
    pub sync_meetings: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            sync_settings: true,
            sync_meetings: true,
        }
    }
}

impl SyncOptions {
    /// Croise les options demandées avec les préférences enregistrées pour le pair.
    pub const fn intersect_with_peer(self, peer: &Peer) -> Self {
        Self {
            sync_settings: self.sync_settings && peer.sync_settings,
            sync_meetings: self.sync_meetings && peer.sync_meetings,
        }
    }
}

/// Compte rendu structuré d'une synchronisation réussie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSummary {
    pub peer_id: DeviceId,
    pub peer_name: String,
    pub config_changed: bool,
    pub merge_report: Option<MergeReport>,
    pub meetings_sent: usize,
    pub meetings_received: usize,
    pub at_ms: u64,
}

/// Interface de stockage utilisée par les sessions d'appairage et de
/// synchronisation. Permet de brancher le disque ([`DiskSyncStorage`]) ou un
/// stockage en mémoire (tests / pont applicatif).
pub trait SyncStorage: Send + Sync {
    fn identity(&self) -> Result<LocalIdentity>;
    /// Charge la configuration locale ainsi que son horodatage de dernière
    /// modification (en millisecondes depuis l'époque Unix).
    fn load_config(&self) -> Result<(Config, u64)>;
    fn save_config(&self, config: &Config) -> Result<()>;
    fn list_meeting_digests(&self) -> Result<Vec<MeetingDigest>>;
    fn load_meeting(&self, id: &str) -> Result<Option<MeetingRecord>>;
    fn save_meeting(&self, record: &MeetingRecord) -> Result<()>;
    fn get_peer(&self, id: &DeviceId) -> Result<Option<Peer>>;
    fn upsert_peer(&self, peer: Peer) -> Result<()>;
    fn mark_peer_synced(&self, id: &DeviceId, at_ms: u64, addr: Option<String>) -> Result<()>;
}

/// Implémentation disque compatible avec l'arborescence PC et Android :
/// - `config_path` : `config.toml`
/// - `peers_path` : `sync_peers.json`
/// - `meetings_dir` : `<meetings_dir>/<id>/meeting.json`
pub struct DiskSyncStorage {
    config_path: PathBuf,
    peers_path: PathBuf,
    meetings_dir: PathBuf,
    default_device_name: String,
    lock: Mutex<()>,
}

impl DiskSyncStorage {
    pub fn new(
        config_path: PathBuf,
        peers_path: PathBuf,
        meetings_dir: PathBuf,
        default_device_name: impl Into<String>,
    ) -> Self {
        Self {
            config_path,
            peers_path,
            meetings_dir,
            default_device_name: sanitize_device_name(&default_device_name.into()),
            lock: Mutex::new(()),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn peers_path(&self) -> &Path {
        &self.peers_path
    }

    pub fn meetings_dir(&self) -> &Path {
        &self.meetings_dir
    }

    /// Liste tous les appareils actuellement appairés.
    pub fn peers(&self) -> Result<Vec<Peer>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        Ok(store.peers)
    }

    /// Oublie un appareil appairé (révoque immédiatement sa clé).
    pub fn remove_peer(&self, id: &DeviceId) -> Result<bool> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        let removed = store.remove(id);
        if removed {
            store.save(&self.peers_path)?;
        }
        Ok(removed)
    }

    /// Met à jour les préférences de synchronisation (réglages / réunions) d'un pair.
    pub fn set_peer_preferences(
        &self,
        id: &DeviceId,
        sync_settings: bool,
        sync_meetings: bool,
    ) -> Result<bool> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        let Some(peer) = store.peers.iter_mut().find(|p| &p.device_id == id) else {
            return Ok(false);
        };
        peer.sync_settings = sync_settings;
        peer.sync_meetings = sync_meetings;
        store.save(&self.peers_path)?;
        Ok(true)
    }

    /// Renomme l'appareil local dans le registre persistant.
    pub fn rename_local_device(&self, name: &str) -> Result<LocalIdentity> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        store.identity.device_name = sanitize_device_name(name);
        store.save(&self.peers_path)?;
        Ok(store.identity)
    }
}

impl SyncStorage for DiskSyncStorage {
    fn identity(&self) -> Result<LocalIdentity> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        Ok(store.identity)
    }

    fn load_config(&self) -> Result<(Config, u64)> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        match fs::read_to_string(&self.config_path) {
            Ok(source) => {
                let cfg = Config::from_toml(&source)
                    .map_err(|e| SyncError::InvalidConfig(e.to_string()))?
                    .checked()
                    .map_err(SyncError::InvalidConfig)?;
                let mtime_ms = file_mtime_ms(&self.config_path).unwrap_or(0);
                Ok((cfg, mtime_ms))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Config::default(), 0)),
            Err(e) => Err(SyncError::Io(e)),
        }
    }

    fn save_config(&self, config: &Config) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let checked = config.clone().checked().map_err(SyncError::InvalidConfig)?;
        let toml = checked
            .to_toml()
            .map_err(|e| SyncError::InvalidConfig(e.to_string()))?;
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.config_path.with_extension("toml.tmp");
        fs::write(&tmp, toml)?;
        fs::rename(&tmp, &self.config_path)?;
        Ok(())
    }

    fn list_meeting_digests(&self) -> Result<Vec<MeetingDigest>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let entries = match fs::read_dir(&self.meetings_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(SyncError::Io(e)),
        };
        let mut digests = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !valid_meeting_id(name) || !path.join(RECORD_FILE).is_file() {
                continue;
            }
            match MeetingRecord::load(&path) {
                Ok(record) => {
                    // On ne synchronise jamais un enregistrement ou une transcription en cours.
                    if matches!(
                        record.status,
                        MeetingStatus::Enregistrement | MeetingStatus::Transcription
                    ) || record.id != name
                    {
                        continue;
                    }
                    digests.push(meeting_digest(&record));
                }
                Err(e) => log::warn!("sync: réunion ignorée ({name}) : {e}"),
            }
        }
        digests.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(digests)
    }

    fn load_meeting(&self, id: &str) -> Result<Option<MeetingRecord>> {
        if !valid_meeting_id(id) {
            return Err(SyncError::Protocol(format!(
                "identifiant de réunion invalide : {id}"
            )));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.meetings_dir.join(id);
        if !dir.join(RECORD_FILE).is_file() {
            return Ok(None);
        }
        let record = MeetingRecord::load(&dir)
            .map_err(|e| SyncError::Protocol(format!("lecture réunion {id} : {e}")))?;
        Ok(Some(record))
    }

    fn save_meeting(&self, record: &MeetingRecord) -> Result<()> {
        if !valid_meeting_id(&record.id) {
            return Err(SyncError::Protocol(format!(
                "identifiant de réunion invalide : {}",
                record.id
            )));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.meetings_dir.join(&record.id);
        let mut clean = sanitize_meeting_for_sync(record);
        // Si une copie locale existait déjà (ex. fichier audio importé sur ce PC),
        // on conserve son chemin local pour ne pas casser la réécoute.
        if let Ok(existing) = MeetingRecord::load(&dir) {
            clean.source_path = existing.source_path;
        }
        fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(&clean)?;
        let tmp = dir.join("meeting.json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, dir.join(RECORD_FILE))?;
        Ok(())
    }

    fn get_peer(&self, id: &DeviceId) -> Result<Option<Peer>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        Ok(store.get(id).cloned())
    }

    fn upsert_peer(&self, peer: Peer) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        store.upsert(peer);
        store.save(&self.peers_path)?;
        Ok(())
    }

    fn mark_peer_synced(&self, id: &DeviceId, at_ms: u64, addr: Option<String>) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = PeerStore::load_or_init(&self.peers_path, &self.default_device_name)?;
        if store.mark_synced(id, at_ms, addr) {
            store.save(&self.peers_path)?;
        }
        Ok(())
    }
}

fn file_mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(dur.as_millis()).ok()
}

/// Exécute un appairage côté initiateur (l'appareil qui a scanné le QR code).
pub fn pair_as_initiator<S: Read + Write>(
    stream: S,
    storage: &dyn SyncStorage,
    offer: &PairingOffer,
    peer_addr: Option<String>,
    now_ms: u64,
) -> Result<Peer> {
    offer.check_valid(now_ms)?;
    let local = storage.identity()?;
    let mut channel =
        EncryptedChannel::connect(stream, &local, &offer.key, Some(&offer.device_id), true)?;

    let key_b64 = match recv_checked(&mut channel)? {
        SyncMessage::PairGrant { key_b64 } => key_b64,
        other => {
            return Err(SyncError::Protocol(format!(
                "PairGrant attendu, reçu {other:?}"
            )));
        }
    };
    let permanent_key = SyncKey::from_base64(&key_b64)?;
    channel.send_message(&SyncMessage::PairAck)?;

    let remote = channel.remote().clone();
    let peer = Peer {
        device_id: remote.device_id,
        device_name: remote.device_name,
        key: permanent_key,
        paired_at_ms: now_ms,
        last_sync_ms: None,
        last_addr: peer_addr.or_else(|| offer.addrs.first().cloned()),
        sync_settings: true,
        sync_meetings: true,
    };
    storage.upsert_peer(peer.clone())?;
    Ok(peer)
}

/// Exécute un appairage côté serveur (l'appareil qui affiche le QR code) sur
/// un canal déjà authentifié avec la clé éphémère de l'offre active.
pub fn pair_as_responder<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
    storage: &dyn SyncStorage,
    peer_addr: Option<String>,
    now_ms: u64,
) -> Result<Peer> {
    if !channel.remote().pairing {
        return Err(SyncError::Protocol(
            "canal non ouvert en mode appairage".into(),
        ));
    }
    let permanent_key = SyncKey::generate()?;
    channel.send_message(&SyncMessage::PairGrant {
        key_b64: permanent_key.to_base64(),
    })?;

    match recv_checked(channel)? {
        SyncMessage::PairAck => {}
        other => {
            return Err(SyncError::Protocol(format!(
                "PairAck attendu, reçu {other:?}"
            )));
        }
    }

    let remote = channel.remote().clone();
    let peer = Peer {
        device_id: remote.device_id,
        device_name: remote.device_name,
        key: permanent_key,
        paired_at_ms: now_ms,
        last_sync_ms: None,
        last_addr: peer_addr,
        sync_settings: true,
        sync_meetings: true,
    };
    storage.upsert_peer(peer.clone())?;
    Ok(peer)
}

/// Exécute une synchronisation bidirectionnelle côté initiateur (client).
///
/// Protocole strictement alterné (sans risque d'interblocage TCP) :
/// 1. Client → Serveur : `ConfigSnapshot` (si `sync_settings`), puis `MeetingIndex`.
/// 2. Serveur → Client : `ConfigSnapshot` (si les deux veulent `sync_settings`),
///    puis `MeetingIndex`, puis `MeetingRequest` (ce que le serveur veut).
/// 3. Client → Serveur : `MeetingRequest` (ce que le client veut), puis les
///    `MeetingTransfer` demandés par le serveur, puis `Done`.
/// 4. Serveur → Client : les `MeetingTransfer` demandés par le client, puis `Done`.
pub fn sync_as_initiator<S: Read + Write>(
    stream: S,
    storage: &dyn SyncStorage,
    peer: &Peer,
    options: SyncOptions,
    peer_addr: Option<String>,
    now_ms: u64,
) -> Result<SyncSummary> {
    let opts = options.intersect_with_peer(peer);
    let local = storage.identity()?;
    let mut channel =
        EncryptedChannel::connect(stream, &local, &peer.key, Some(&peer.device_id), false)?;

    let (local_cfg, local_cfg_ts) = if opts.sync_settings {
        storage.load_config()?
    } else {
        (Config::default(), 0)
    };
    let local_digests = if opts.sync_meetings {
        storage.list_meeting_digests()?
    } else {
        Vec::new()
    };

    // 1. Envoi de la config locale (sans clés API) + index des réunions.
    if opts.sync_settings {
        let (payload, _) = SyncableConfig::from_config(&local_cfg);
        channel.send_message(&SyncMessage::ConfigSnapshot {
            updated_at_ms: local_cfg_ts,
            device_id: local.device_id.clone(),
            payload,
        })?;
    }
    channel.send_message(&SyncMessage::MeetingIndex {
        digests: local_digests.clone(),
    })?;

    // 2. Réception de la config distante (éventuelle) + index distant + demande serveur.
    let mut config_changed = false;
    let mut merge_report = None;
    let remote_digests = match recv_checked(&mut channel)? {
        SyncMessage::ConfigSnapshot {
            updated_at_ms,
            device_id,
            payload,
        } => {
            if opts.sync_settings {
                let (merged, report) = merge_config(
                    &local_cfg,
                    local_cfg_ts,
                    &local.device_id,
                    &payload,
                    updated_at_ms,
                    &device_id,
                )?;
                let changed = report.total_changes() > 0;
                if changed {
                    storage.save_config(&merged)?;
                }
                config_changed = changed;
                merge_report = Some(report);
            }
            expect_meeting_index(&mut channel)?
        }
        SyncMessage::MeetingIndex { digests } => digests,
        other => {
            return Err(SyncError::Protocol(format!(
                "ConfigSnapshot ou MeetingIndex attendu, reçu {other:?}"
            )));
        }
    };
    let server_wants = expect_meeting_request(&mut channel)?;

    // 3. Envoi de notre demande + des réunions demandées par le serveur.
    let client_wants = if opts.sync_meetings {
        diff_meetings(&local_digests, &remote_digests).to_request
    } else {
        Vec::new()
    };
    channel.send_message(&SyncMessage::MeetingRequest {
        ids: client_wants.clone(),
    })?;

    let meetings_sent = if opts.sync_meetings {
        send_requested_meetings(&mut channel, storage, &server_wants)?
    } else {
        0
    };
    channel.send_message(&SyncMessage::Done {
        merged_items: meetings_sent,
    })?;

    // 4. Réception des réunions demandées au serveur.
    let meetings_received =
        recv_meetings_until_done(&mut channel, storage, &client_wants, opts.sync_meetings)?;

    let remote = channel.remote().clone();
    storage.mark_peer_synced(&remote.device_id, now_ms, peer_addr)?;

    Ok(SyncSummary {
        peer_id: remote.device_id,
        peer_name: remote.device_name,
        config_changed,
        merge_report,
        meetings_sent,
        meetings_received,
        at_ms: now_ms,
    })
}

/// Exécute une synchronisation bidirectionnelle côté serveur sur un canal déjà
/// authentifié avec la clé permanente du pair.
pub fn sync_as_responder<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
    storage: &dyn SyncStorage,
    peer: &Peer,
    options: SyncOptions,
    peer_addr: Option<String>,
    now_ms: u64,
) -> Result<SyncSummary> {
    if channel.remote().pairing {
        return Err(SyncError::Protocol(
            "canal ouvert en mode appairage au lieu de synchronisation".into(),
        ));
    }
    let opts = options.intersect_with_peer(peer);
    let local = storage.identity()?;

    // 1. Réception de la config distante (éventuelle) + index des réunions du client.
    let (client_snap, client_digests) = match recv_checked(channel)? {
        SyncMessage::ConfigSnapshot {
            updated_at_ms,
            device_id,
            payload,
        } => {
            let digests = expect_meeting_index(channel)?;
            (Some((updated_at_ms, device_id, payload)), digests)
        }
        SyncMessage::MeetingIndex { digests } => (None, digests),
        other => {
            return Err(SyncError::Protocol(format!(
                "ConfigSnapshot ou MeetingIndex attendu, reçu {other:?}"
            )));
        }
    };

    // 2. Si les deux côtés synchronisent la config, on envoie d'abord l'instantané
    //    local AVANT fusion afin que les deux appareils exécutent exactement le
    //    même `merge_config` et convergent au bit près.
    let mut config_changed = false;
    let mut merge_report = None;
    if opts.sync_settings
        && let Some((remote_ts, remote_id, remote_payload)) = client_snap
    {
        let (local_cfg, local_cfg_ts) = storage.load_config()?;
        let (local_payload, _) = SyncableConfig::from_config(&local_cfg);
        channel.send_message(&SyncMessage::ConfigSnapshot {
            updated_at_ms: local_cfg_ts,
            device_id: local.device_id.clone(),
            payload: local_payload,
        })?;
        let (merged, report) = merge_config(
            &local_cfg,
            local_cfg_ts,
            &local.device_id,
            &remote_payload,
            remote_ts,
            &remote_id,
        )?;
        let changed = report.total_changes() > 0;
        if changed {
            storage.save_config(&merged)?;
        }
        config_changed = changed;
        merge_report = Some(report);
    }

    let local_digests = if opts.sync_meetings {
        storage.list_meeting_digests()?
    } else {
        Vec::new()
    };
    channel.send_message(&SyncMessage::MeetingIndex {
        digests: local_digests.clone(),
    })?;

    let server_wants = if opts.sync_meetings {
        diff_meetings(&local_digests, &client_digests).to_request
    } else {
        Vec::new()
    };
    channel.send_message(&SyncMessage::MeetingRequest {
        ids: server_wants.clone(),
    })?;

    // 3. Réception de la demande client + des réunions envoyées par le client.
    let client_wants = expect_meeting_request(channel)?;
    let meetings_received =
        recv_meetings_until_done(channel, storage, &server_wants, opts.sync_meetings)?;

    // 4. Envoi des réunions demandées par le client + clôture.
    let meetings_sent = if opts.sync_meetings {
        send_requested_meetings(channel, storage, &client_wants)?
    } else {
        0
    };
    channel.send_message(&SyncMessage::Done {
        merged_items: meetings_sent,
    })?;

    let remote = channel.remote().clone();
    storage.mark_peer_synced(&remote.device_id, now_ms, peer_addr)?;

    Ok(SyncSummary {
        peer_id: remote.device_id,
        peer_name: remote.device_name,
        config_changed,
        merge_report,
        meetings_sent,
        meetings_received,
        at_ms: now_ms,
    })
}

fn recv_checked<S: Read + Write>(channel: &mut EncryptedChannel<S>) -> Result<SyncMessage> {
    match channel.recv_message()? {
        SyncMessage::Error { message } => Err(SyncError::Remote(message)),
        other => Ok(other),
    }
}

fn expect_meeting_index<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
) -> Result<Vec<MeetingDigest>> {
    match recv_checked(channel)? {
        SyncMessage::MeetingIndex { digests } => Ok(digests),
        other => Err(SyncError::Protocol(format!(
            "MeetingIndex attendu, reçu {other:?}"
        ))),
    }
}

fn expect_meeting_request<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
) -> Result<Vec<String>> {
    match recv_checked(channel)? {
        SyncMessage::MeetingRequest { ids } => {
            if ids.len() > MAX_MEETINGS_PER_SESSION {
                return Err(SyncError::Protocol(format!(
                    "trop de réunions demandées ({})",
                    ids.len()
                )));
            }
            for id in &ids {
                if !valid_meeting_id(id) {
                    return Err(SyncError::Protocol(format!(
                        "identifiant de réunion invalide demandé : {id}"
                    )));
                }
            }
            Ok(ids)
        }
        other => Err(SyncError::Protocol(format!(
            "MeetingRequest attendu, reçu {other:?}"
        ))),
    }
}

fn send_requested_meetings<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
    storage: &dyn SyncStorage,
    requested_ids: &[String],
) -> Result<usize> {
    let mut sent = 0;
    let mut seen = HashSet::new();
    for id in requested_ids {
        if !seen.insert(id.as_str()) {
            continue;
        }
        if let Some(record) = storage.load_meeting(id)? {
            let clean = sanitize_meeting_for_sync(&record);
            channel.send_message(&SyncMessage::MeetingTransfer {
                record: Box::new(clean),
            })?;
            sent += 1;
        }
    }
    Ok(sent)
}

fn recv_meetings_until_done<S: Read + Write>(
    channel: &mut EncryptedChannel<S>,
    storage: &dyn SyncStorage,
    expected_ids: &[String],
    allow_save: bool,
) -> Result<usize> {
    let expected: HashSet<&str> = expected_ids.iter().map(String::as_str).collect();
    let mut received = 0;
    loop {
        match recv_checked(channel)? {
            SyncMessage::MeetingTransfer { record } => {
                if received >= MAX_MEETINGS_PER_SESSION {
                    return Err(SyncError::Protocol(
                        "limite de réunions par session dépassée".into(),
                    ));
                }
                if !valid_meeting_id(&record.id) {
                    return Err(SyncError::Protocol(format!(
                        "identifiant de réunion reçu invalide : {}",
                        record.id
                    )));
                }
                // On n'accepte que les réunions explicitement demandées (défense en profondeur).
                if allow_save && expected.contains(record.id.as_str()) {
                    storage.save_meeting(&record)?;
                    received += 1;
                }
            }
            SyncMessage::Done { .. } => return Ok(received),
            other => {
                return Err(SyncError::Protocol(format!(
                    "MeetingTransfer ou Done attendu, reçu {other:?}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::*;
    use crate::config::MeetingAudio;
    use crate::meeting::{MeetingKind, MeetingSummary, Output};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_storage(label: &str, device_name: &str) -> (PathBuf, DiskSyncStorage) {
        let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vtt_sync_session_{label}_{}_{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let storage = DiskSyncStorage::new(
            root.join("config.toml"),
            root.join("sync_peers.json"),
            root.join("meetings"),
            device_name,
        );
        (root, storage)
    }

    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let h = thread::spawn(move || TcpStream::connect(addr).unwrap());
        let (server, _) = listener.accept().unwrap();
        let client = h.join().unwrap();
        (client, server)
    }

    fn make_meeting(id: &str, started_at_ms: u64, title: &str) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: title.into(),
            started_at_ms,
            duration_ms: 90_000,
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
            source_path: Some("C:\\Users\\Alice\\demo.mp3".into()),
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
    fn valid_meeting_id_blocks_path_traversal() {
        assert!(valid_meeting_id("2026-11-19_10-30-00_abcd"));
        assert!(!valid_meeting_id(""));
        assert!(!valid_meeting_id(".."));
        assert!(!valid_meeting_id("../secret"));
        assert!(!valid_meeting_id("a/b"));
        assert!(!valid_meeting_id("a\\b"));
        assert!(!valid_meeting_id("meeting.json"));
    }

    #[test]
    fn disk_storage_skips_in_progress_meetings_and_preserves_local_source_path() {
        let (root, storage) = temp_storage("disk", "PC");

        let mut active = make_meeting("reunion-en-cours", 1_000, "En cours");
        active.status = MeetingStatus::Enregistrement;
        let active_dir = storage.meetings_dir().join(&active.id);
        fs::create_dir_all(&active_dir).unwrap();
        active.save(&active_dir);

        let mut done = make_meeting("reunion-finie", 2_000, "Ancien titre");
        done.source_path = Some("C:\\Users\\PC\\original.wav".into());
        let done_dir = storage.meetings_dir().join(&done.id);
        fs::create_dir_all(&done_dir).unwrap();
        done.save(&done_dir);

        let digests = storage.list_meeting_digests().unwrap();
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].id, "reunion-finie");

        // Mise à jour reçue depuis Android (sans source_path) : le source_path local du PC est préservé.
        let mut updated_from_phone = make_meeting("reunion-finie", 2_000, "Titre mis à jour");
        updated_from_phone.source_path = Some("/storage/emulated/0/phone.m4a".into());
        storage.save_meeting(&updated_from_phone).unwrap();

        let reloaded = storage.load_meeting("reunion-finie").unwrap().unwrap();
        assert_eq!(reloaded.title, "Titre mis à jour");
        assert_eq!(
            reloaded.source_path.as_deref(),
            Some("C:\\Users\\PC\\original.wav")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bidirectional_sync_exchanges_config_and_meetings_and_converges() {
        let (root_pc, pc) = temp_storage("pc", "PC Bureau");
        let (root_phone, phone) = temp_storage("phone", "Android");

        let pc_id = pc.identity().unwrap();
        let phone_id = phone.identity().unwrap();
        let shared = SyncKey::generate().unwrap();

        pc.upsert_peer(Peer {
            device_id: phone_id.device_id.clone(),
            device_name: phone_id.device_name.clone(),
            key: shared.clone(),
            paired_at_ms: 1_000,
            last_sync_ms: None,
            last_addr: None,
            sync_settings: true,
            sync_meetings: true,
        })
        .unwrap();
        phone
            .upsert_peer(Peer {
                device_id: pc_id.device_id.clone(),
                device_name: pc_id.device_name.clone(),
                key: shared.clone(),
                paired_at_ms: 1_000,
                last_sync_ms: None,
                last_addr: None,
                sync_settings: true,
                sync_meetings: true,
            })
            .unwrap();

        // Configs distinctes : chaque appareil a son mot + sa touche/micro locaux.
        let mut pc_cfg = Config {
            dictionary: vec!["Aquadrox".into()],
            ..Default::default()
        };
        pc_cfg.hotkey.key = "F13".into();
        pc.save_config(&pc_cfg).unwrap();

        let mut phone_cfg = Config {
            dictionary: vec!["Parakeet".into()],
            ..Default::default()
        };
        phone_cfg.audio.device = "Micro Android".into();
        phone.save_config(&phone_cfg).unwrap();

        // Réunions : m_pc uniquement sur PC, m_phone uniquement sur Android,
        // m_shared plus récente sur PC (résumé généré).
        pc.save_meeting(&make_meeting("m_pc", 10_000, "Réunion PC"))
            .unwrap();
        phone
            .save_meeting(&make_meeting("m_phone", 20_000, "Réunion Android"))
            .unwrap();

        let m_shared_old = make_meeting("m_shared", 30_000, "Hebdo");
        let mut m_shared_new = m_shared_old.clone();
        m_shared_new.summary = Some(MeetingSummary {
            text: "Résumé final".into(),
            at_ms: 999_000,
            model: "gpt-5-mini".into(),
            cost: None,
            currency: "EUR".into(),
        });
        pc.save_meeting(&m_shared_new).unwrap();
        phone.save_meeting(&m_shared_old).unwrap();

        let (client_io, server_io) = tcp_pair();
        let pc_peer_for_srv = pc.get_peer(&phone_id.device_id).unwrap().unwrap();
        let pc_for_srv = DiskSyncStorage::new(
            pc.config_path().to_path_buf(),
            pc.peers_path().to_path_buf(),
            pc.meetings_dir().to_path_buf(),
            "PC Bureau",
        );

        let pc_id_srv = pc_id.clone();
        let srv = thread::spawn(move || {
            let mut ch = EncryptedChannel::accept(server_io, &pc_id_srv, |_| Ok(shared)).unwrap();
            sync_as_responder(
                &mut ch,
                &pc_for_srv,
                &pc_peer_for_srv,
                SyncOptions::default(),
                Some("127.0.0.1:50000".into()),
                50_000,
            )
            .unwrap()
        });

        let phone_peer = phone.get_peer(&pc_id.device_id).unwrap().unwrap();
        let client_summary = sync_as_initiator(
            client_io,
            &phone,
            &phone_peer,
            SyncOptions::default(),
            Some("127.0.0.1:41739".into()),
            50_000,
        )
        .unwrap();
        let server_summary = srv.join().unwrap();

        assert!(client_summary.config_changed);
        assert!(server_summary.config_changed);
        assert_eq!(client_summary.meetings_sent, 1); // m_phone
        assert_eq!(client_summary.meetings_received, 2); // m_pc + m_shared
        assert_eq!(server_summary.meetings_sent, 2);
        assert_eq!(server_summary.meetings_received, 1);

        // Les deux configs ont convergé tout en gardant les réglages matériels locaux.
        let (pc_final, _) = pc.load_config().unwrap();
        let (phone_final, _) = phone.load_config().unwrap();
        assert_eq!(pc_final.dictionary, phone_final.dictionary);
        let mut sorted = pc_final.dictionary;
        sorted.sort();
        assert_eq!(sorted, vec!["Aquadrox", "Parakeet"]);
        assert_eq!(pc_final.hotkey.key, "F13");
        assert_eq!(phone_final.audio.device, "Micro Android");

        // Les trois réunions sont identiques sur les deux appareils et sans source_path distant.
        let phone_shared = phone.load_meeting("m_shared").unwrap().unwrap();
        assert_eq!(phone_shared.summary.unwrap().text, "Résumé final");
        let pc_from_phone = pc.load_meeting("m_phone").unwrap().unwrap();
        assert_eq!(pc_from_phone.title, "Réunion Android");
        assert!(pc_from_phone.source_path.is_none());

        let _ = fs::remove_dir_all(&root_pc);
        let _ = fs::remove_dir_all(&root_phone);
    }

    #[test]
    fn selective_sync_respects_meetings_only_without_modifying_config() {
        let (root_a, a) = temp_storage("sel_a", "PC");
        let (root_b, b) = temp_storage("sel_b", "Phone");

        let id_a = a.identity().unwrap();
        let id_b = b.identity().unwrap();
        let shared = SyncKey::generate().unwrap();

        let peer_b_on_a = Peer {
            device_id: id_b.device_id.clone(),
            device_name: id_b.device_name.clone(),
            key: shared.clone(),
            paired_at_ms: 1_000,
            last_sync_ms: None,
            last_addr: None,
            sync_settings: true,
            sync_meetings: true,
        };
        let peer_a_on_b = Peer {
            device_id: id_a.device_id.clone(),
            device_name: id_a.device_name.clone(),
            key: shared.clone(),
            paired_at_ms: 1_000,
            last_sync_ms: None,
            last_addr: None,
            sync_settings: false, // B refuse la synchro de config
            sync_meetings: true,
        };
        a.upsert_peer(peer_b_on_a.clone()).unwrap();
        b.upsert_peer(peer_a_on_b.clone()).unwrap();

        a.save_config(&Config {
            dictionary: vec!["OnlyA".into()],
            ..Default::default()
        })
        .unwrap();
        b.save_config(&Config {
            dictionary: vec!["OnlyB".into()],
            ..Default::default()
        })
        .unwrap();
        a.save_meeting(&make_meeting("only_a", 1_000, "A")).unwrap();

        let (client_io, server_io) = tcp_pair();
        let a_srv = DiskSyncStorage::new(
            a.config_path().to_path_buf(),
            a.peers_path().to_path_buf(),
            a.meetings_dir().to_path_buf(),
            "PC",
        );
        let srv = thread::spawn(move || {
            let mut ch = EncryptedChannel::accept(server_io, &id_a, |_| Ok(shared)).unwrap();
            sync_as_responder(
                &mut ch,
                &a_srv,
                &peer_b_on_a,
                SyncOptions::default(),
                None,
                5_000,
            )
            .unwrap()
        });

        let summary = sync_as_initiator(
            client_io,
            &b,
            &peer_a_on_b,
            SyncOptions::default(),
            None,
            5_000,
        )
        .unwrap();
        let srv_summary = srv.join().unwrap();

        assert!(!summary.config_changed);
        assert!(!srv_summary.config_changed);
        assert_eq!(summary.meetings_received, 1);
        assert_eq!(a.load_config().unwrap().0.dictionary, vec!["OnlyA"]);
        assert_eq!(b.load_config().unwrap().0.dictionary, vec!["OnlyB"]);
        assert!(b.load_meeting("only_a").unwrap().is_some());

        let _ = fs::remove_dir_all(&root_a);
        let _ = fs::remove_dir_all(&root_b);
    }
}
