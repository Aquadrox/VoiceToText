//! Transport réseau local TCP pour la synchronisation pair-à-pair chiffrée :
//! - [`SyncServer`] : écoute TCP locale, gestion des offres d'appairage QR à
//!   usage unique et acceptation des sessions chiffrées ;
//! - [`pair_over_tcp`] et [`sync_with_peer`] : client TCP avec délais bornés et
//!   secours direct par adresse `IP:port` ;
//! - [`detect_local_addrs`] : détection portable (Linux, Windows, Android) des
//!   adresses IPv4 locales joignables sur le Wi-Fi/LAN.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use socket2::{Domain, Protocol, Socket, Type};

use super::channel::EncryptedChannel;
use super::pairing::{DeviceId, PairingOffer, Peer};
use super::session::{
    SyncOptions, SyncStorage, SyncSummary, pair_as_initiator, pair_as_responder, sync_as_initiator,
    sync_as_responder,
};
use super::{Result, SyncError};

/// Délai maximal d'établissement d'une connexion TCP sur le réseau local.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Délai maximal de lecture/écriture sur une session active.
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(15);

/// Nombre maximal de sessions entrantes simultanées traitées par [`SyncServer`].
const MAX_CONCURRENT_SESSIONS: usize = 4;

/// Événement émis par [`SyncServer`] vers l'interface (PC ou Android).
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Un nouvel appareil vient d'être appairé via le QR code actif.
    Paired { peer: Peer },
    /// Une synchronisation avec un appareil appairé vient de réussir.
    Synced { summary: SyncSummary },
    /// Une connexion entrante a été refusée ou interrompue.
    ConnectionFailed { reason: String },
}

type EventCallback = Arc<dyn Fn(SyncEvent) + Send + Sync>;

/// Horodatage courant en millisecondes depuis l'époque Unix.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Détecte les adresses IPv4 locales joignables sur le réseau local (sans
/// envoyer aucun paquet sur le fil : `UdpSocket::connect` interroge seulement
/// la table de routage du système).
pub fn detect_local_addrs(port: u16) -> Vec<String> {
    if port == 0 {
        return Vec::new();
    }
    // Sondes multi-réseaux : route par défaut + sous-réseaux privés courants
    // (utile si le réseau Wi-Fi est isolé sans accès Internet).
    const PROBES: &[&str] = &[
        "192.0.2.1:80",
        "192.168.0.1:80",
        "10.0.0.1:80",
        "172.16.0.1:80",
    ];
    let mut addrs = Vec::new();
    for probe in PROBES {
        let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if sock.connect(probe).is_err() {
            continue;
        }
        let Ok(local) = sock.local_addr() else {
            continue;
        };
        if let IpAddr::V4(ip) = local.ip()
            && !ip.is_loopback()
            && !ip.is_unspecified()
            && !ip.is_link_local()
        {
            let formatted = SocketAddr::new(IpAddr::V4(ip), port).to_string();
            if !addrs.contains(&formatted) {
                addrs.push(formatted);
            }
        }
    }
    addrs
}

/// Retourne la première IPv4 locale détectée (utile pour l'annonce mDNS).
pub fn detect_primary_ipv4() -> Option<Ipv4Addr> {
    detect_local_addrs(1)
        .into_iter()
        .find_map(|s| s.parse::<SocketAddr>().ok())
        .and_then(|sa| match sa.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
}

/// Serveur TCP local écoutant les appairages QR et les synchronisations chiffrées.
pub struct SyncServer {
    local_addr: SocketAddr,
    storage: Arc<dyn SyncStorage>,
    active_offer: Arc<Mutex<Option<PairingOffer>>>,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SyncServer {
    /// Démarre l'écoute TCP sur `bind_addr` (ex. `0.0.0.0:41739`, ou
    /// `127.0.0.1:0` pour un port éphémère en test).
    pub fn start(
        bind_addr: SocketAddr,
        storage: Arc<dyn SyncStorage>,
        on_event: Option<EventCallback>,
    ) -> Result<Self> {
        // Vérifie dès le démarrage que l'identité locale est lisible/initialisable.
        let _ = storage.identity()?;

        let listener = bind_reusable_tcp(bind_addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;

        let active_offer = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let active_workers = Arc::new(AtomicUsize::new(0));

        let srv_storage = Arc::clone(&storage);
        let srv_offer = Arc::clone(&active_offer);
        let srv_stop = Arc::clone(&stop_flag);
        let srv_workers = Arc::clone(&active_workers);

        let handle = thread::Builder::new()
            .name("vtt-sync-server".into())
            .spawn(move || {
                accept_loop(
                    listener,
                    srv_storage,
                    srv_offer,
                    srv_stop,
                    srv_workers,
                    on_event,
                );
            })?;

        Ok(Self {
            local_addr,
            storage,
            active_offer,
            stop_flag,
            handle: Some(handle),
        })
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub const fn port(&self) -> u16 {
        self.local_addr.port()
    }

    /// Ouvre une offre d'appairage QR éphémère (à usage unique) avec la liste
    /// d'adresses locales fournie.
    pub fn open_pairing(&self, addrs: Vec<String>, ttl_ms: Option<u64>) -> Result<PairingOffer> {
        let identity = self.storage.identity()?;
        let offer = PairingOffer::create(&identity, addrs, now_ms(), ttl_ms)?;
        let mut guard = self.active_offer.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(offer.clone());
        Ok(offer)
    }

    /// Ouvre une offre d'appairage QR en détectant automatiquement les adresses
    /// locales du réseau (avec repli sur `local_addr` si aucune interface LAN
    /// n'est trouvée).
    pub fn open_pairing_auto(&self, ttl_ms: Option<u64>) -> Result<PairingOffer> {
        let mut addrs = detect_local_addrs(self.port());
        if addrs.is_empty() {
            let fallback_ip = if self.local_addr.ip().is_unspecified() {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            } else {
                self.local_addr.ip()
            };
            addrs.push(SocketAddr::new(fallback_ip, self.port()).to_string());
        }
        self.open_pairing(addrs, ttl_ms)
    }

    /// Retourne l'offre d'appairage active si elle existe et n'est pas expirée.
    pub fn active_pairing(&self) -> Option<PairingOffer> {
        let mut guard = self.active_offer.lock().unwrap_or_else(|e| e.into_inner());
        let valid = guard
            .as_ref()
            .is_some_and(|o| o.check_valid(now_ms()).is_ok());
        if !valid {
            *guard = None;
            return None;
        }
        guard.clone()
    }

    /// Ferme immédiatement l'offre d'appairage QR active.
    pub fn cancel_pairing(&self) {
        let mut guard = self.active_offer.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Arrête proprement le serveur et attend la fin de la boucle d'écoute.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SyncServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn bind_reusable_tcp(addr: SocketAddr) -> Result<TcpListener> {
    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

struct WorkerGuard(Arc<AtomicUsize>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(
    listener: TcpListener,
    storage: Arc<dyn SyncStorage>,
    active_offer: Arc<Mutex<Option<PairingOffer>>>,
    stop_flag: Arc<AtomicBool>,
    active_workers: Arc<AtomicUsize>,
    on_event: Option<EventCallback>,
) {
    while !stop_flag.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, remote_addr)) => {
                let prev = active_workers.fetch_add(1, Ordering::SeqCst);
                if prev >= MAX_CONCURRENT_SESSIONS {
                    active_workers.fetch_sub(1, Ordering::SeqCst);
                    log::warn!("sync: connexion {remote_addr} ignorée (trop de sessions actives)");
                    continue;
                }
                let guard = WorkerGuard(Arc::clone(&active_workers));
                let w_storage = Arc::clone(&storage);
                let w_offer = Arc::clone(&active_offer);
                let w_event = on_event.clone();
                let spawn_res =
                    thread::Builder::new()
                        .name("vtt-sync-worker".into())
                        .spawn(move || {
                            let _guard = guard;
                            handle_incoming(stream, remote_addr, w_storage, w_offer, w_event);
                        });
                if let Err(e) = spawn_res {
                    log::warn!("sync: impossible de créer le thread de session : {e}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(35));
            }
            Err(e) => {
                if !stop_flag.load(Ordering::Relaxed) {
                    log::warn!("sync: erreur accept TCP : {e}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

fn configure_stream(stream: &TcpStream, io_timeout: Duration) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(io_timeout));
    let _ = stream.set_write_timeout(Some(io_timeout));
}

fn emit(on_event: &Option<EventCallback>, event: SyncEvent) {
    if let Some(cb) = on_event {
        cb(event);
    }
}

fn handle_incoming(
    stream: TcpStream,
    remote_addr: SocketAddr,
    storage: Arc<dyn SyncStorage>,
    active_offer: Arc<Mutex<Option<PairingOffer>>>,
    on_event: Option<EventCallback>,
) {
    configure_stream(&stream, DEFAULT_IO_TIMEOUT);
    let peer_addr_str = Some(remote_addr.to_string());

    let outcome = (|| -> Result<SyncEvent> {
        let local = storage.identity()?;
        let mut used_offer_key_b64: Option<String> = None;
        let mut matched_peer: Option<Peer> = None;

        let mut channel = EncryptedChannel::accept(stream, &local, |hello| {
            if hello.pairing {
                let mut guard = active_offer.lock().unwrap_or_else(|e| e.into_inner());
                let Some(offer) = guard.as_ref() else {
                    return Err(SyncError::PairingClosed);
                };
                if let Err(e) = offer.check_valid(now_ms()) {
                    *guard = None;
                    return Err(e);
                }
                used_offer_key_b64 = Some(offer.key.to_base64());
                Ok(offer.key.clone())
            } else {
                let peer = storage
                    .get_peer(&hello.device_id)?
                    .ok_or_else(|| SyncError::UnknownPeer(hello.device_id.clone()))?;
                let key = peer.key.clone();
                matched_peer = Some(peer);
                Ok(key)
            }
        })?;

        if channel.remote().pairing {
            let peer = pair_as_responder(&mut channel, storage.as_ref(), peer_addr_str, now_ms())?;
            // Offre QR à usage unique : on l'invalide dès qu'un appairage aboutit.
            if let Some(used_b64) = used_offer_key_b64 {
                let mut guard = active_offer.lock().unwrap_or_else(|e| e.into_inner());
                if guard
                    .as_ref()
                    .is_some_and(|o| o.key.to_base64() == used_b64)
                {
                    *guard = None;
                }
            }
            Ok(SyncEvent::Paired { peer })
        } else {
            let peer = matched_peer
                .ok_or_else(|| SyncError::UnknownPeer(channel.remote().device_id.clone()))?;
            let summary = sync_as_responder(
                &mut channel,
                storage.as_ref(),
                &peer,
                SyncOptions::default(),
                peer_addr_str,
                now_ms(),
            )?;
            Ok(SyncEvent::Synced { summary })
        }
    })();

    match outcome {
        Ok(event) => emit(&on_event, event),
        Err(e) => {
            log::warn!("sync: session entrante {remote_addr} échouée : {e}");
            emit(
                &on_event,
                SyncEvent::ConnectionFailed {
                    reason: e.to_string(),
                },
            );
        }
    }
}

/// Appaire cet appareil avec un pair distant à partir de son offre QR code
/// (`vttsync://v1?...`) en essayant ses adresses locales jusqu'à ce qu'une
/// connexion aboutisse.
pub fn pair_over_tcp(
    storage: &dyn SyncStorage,
    offer: &PairingOffer,
    connect_timeout: Duration,
) -> Result<Peer> {
    let ts = now_ms();
    offer.check_valid(ts)?;
    let (stream, connected_addr) = connect_first(&offer.addrs, connect_timeout)?;
    configure_stream(&stream, DEFAULT_IO_TIMEOUT);
    pair_as_initiator(stream, storage, offer, Some(connected_addr), now_ms())
}

/// Synchronise cet appareil avec un pair déjà appairé.
///
/// Utilise `addr_override` en priorité (par exemple une adresse découverte via
/// mDNS ou saisie manuellement `IP:port`), puis se rabat sur la dernière adresse
/// connue (`peer.last_addr`).
pub fn sync_with_peer(
    storage: &dyn SyncStorage,
    peer_id: &DeviceId,
    addr_override: Option<&str>,
    options: SyncOptions,
    connect_timeout: Duration,
) -> Result<SyncSummary> {
    let peer = storage
        .get_peer(peer_id)?
        .ok_or_else(|| SyncError::UnknownPeer(peer_id.clone()))?;

    let mut candidates = Vec::new();
    if let Some(addr) = addr_override {
        let trimmed = addr.trim();
        if !trimmed.is_empty() {
            candidates.push(trimmed.to_owned());
        }
    }
    if let Some(last) = &peer.last_addr
        && !candidates.contains(last)
    {
        candidates.push(last.clone());
    }
    if candidates.is_empty() {
        return Err(SyncError::Connect(
            "aucune adresse connue pour cet appareil (saisissez IP:port ou relancez la découverte)"
                .into(),
        ));
    }

    let (stream, connected_addr) = connect_first(&candidates, connect_timeout)?;
    configure_stream(&stream, DEFAULT_IO_TIMEOUT);
    sync_as_initiator(
        stream,
        storage,
        &peer,
        options,
        Some(connected_addr),
        now_ms(),
    )
}

fn connect_first(addrs: &[String], timeout: Duration) -> Result<(TcpStream, String)> {
    let mut last_err = String::from("aucune adresse valide");
    for raw in addrs {
        let Ok(sock_addr) = raw.parse::<SocketAddr>() else {
            last_err = format!("adresse invalide : {raw}");
            continue;
        };
        match TcpStream::connect_timeout(&sock_addr, timeout) {
            Ok(stream) => return Ok((stream, sock_addr.to_string())),
            Err(e) => last_err = format!("{sock_addr} : {e}"),
        }
    }
    Err(SyncError::Connect(last_err))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;

    use super::*;
    use crate::config::{Config, MeetingAudio};
    use crate::meeting::{MeetingKind, MeetingRecord, MeetingStatus, Output};
    use crate::sync::session::DiskSyncStorage;

    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_disk_storage(label: &str, name: &str) -> (PathBuf, Arc<DiskSyncStorage>) {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vtt_sync_transport_{label}_{}_{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let storage = Arc::new(DiskSyncStorage::new(
            root.join("config.toml"),
            root.join("sync_peers.json"),
            root.join("meetings"),
            name,
        ));
        (root, storage)
    }

    fn sample_meeting(id: &str, title: &str) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: title.into(),
            started_at_ms: 100_000,
            duration_ms: 45_000,
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
    fn tcp_loopback_qr_pairing_then_full_sync_and_revocation() {
        let (root_pc, pc_storage) = temp_disk_storage("pc", "PC Linux");
        let (root_phone, phone_storage) = temp_disk_storage("phone", "Android");

        // Prépare des données distinctes sur PC et téléphone.
        pc_storage
            .save_config(&Config {
                dictionary: vec!["Tauri".into()],
                ..Default::default()
            })
            .unwrap();
        pc_storage
            .save_meeting(&sample_meeting("pc_meeting_1", "Point produit"))
            .unwrap();

        phone_storage
            .save_config(&Config {
                dictionary: vec!["Android".into()],
                ..Default::default()
            })
            .unwrap();
        phone_storage
            .save_meeting(&sample_meeting("phone_meeting_1", "Note vocale"))
            .unwrap();

        let (tx, rx) = mpsc::channel();
        let cb: EventCallback = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let server =
            SyncServer::start("127.0.0.1:0".parse().unwrap(), pc_storage.clone(), Some(cb))
                .unwrap();

        // 1. Le PC ouvre un QR code éphémère, le téléphone le scanne et s'appaire.
        let offer = server
            .open_pairing(vec![server.local_addr().to_string()], Some(60_000))
            .unwrap();
        let qr_uri = offer.to_uri();
        let parsed_offer = PairingOffer::from_uri(&qr_uri).unwrap();

        let paired_pc = pair_over_tcp(
            phone_storage.as_ref(),
            &parsed_offer,
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(paired_pc.device_name, "PC Linux");

        let ev1 = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(ev1, SyncEvent::Paired { .. }));

        // L'offre QR étant à usage unique, elle doit être refermée immédiatement.
        assert!(server.active_pairing().is_none());
        let reuse = pair_over_tcp(
            phone_storage.as_ref(),
            &parsed_offer,
            Duration::from_secs(2),
        );
        assert!(reuse.is_err());

        // 2. Le téléphone lance une synchronisation complète via TCP.
        let summary = sync_with_peer(
            phone_storage.as_ref(),
            &paired_pc.device_id,
            None,
            SyncOptions::default(),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(summary.config_changed);
        assert_eq!(summary.meetings_sent, 1);
        assert_eq!(summary.meetings_received, 1);

        let ev2 = loop {
            match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
                SyncEvent::Synced { summary } => break summary,
                SyncEvent::ConnectionFailed { .. } => continue, // le test reuse ci-dessus
                other => panic!("événement inattendu : {other:?}"),
            }
        };
        assert_eq!(ev2.meetings_sent, 1);
        assert_eq!(ev2.meetings_received, 1);

        // Vérifie la convergence exacte des deux côtés.
        let (pc_cfg, _) = pc_storage.load_config().unwrap();
        let (phone_cfg, _) = phone_storage.load_config().unwrap();
        assert_eq!(pc_cfg.dictionary, phone_cfg.dictionary);
        let mut sorted = pc_cfg.dictionary;
        sorted.sort();
        assert_eq!(sorted, vec!["Android", "Tauri"]);
        assert!(
            pc_storage
                .load_meeting("phone_meeting_1")
                .unwrap()
                .is_some()
        );
        assert!(
            phone_storage
                .load_meeting("pc_meeting_1")
                .unwrap()
                .is_some()
        );

        // 3. Une deuxième synchro immédiate n'a rien à transférer.
        let summary2 = sync_with_peer(
            phone_storage.as_ref(),
            &paired_pc.device_id,
            None,
            SyncOptions::default(),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(!summary2.config_changed);
        assert_eq!(summary2.meetings_sent, 0);
        assert_eq!(summary2.meetings_received, 0);

        // 4. Si le PC révoque le téléphone, la synchro suivante est refusée.
        let phone_id = phone_storage.identity().unwrap().device_id;
        assert!(pc_storage.remove_peer(&phone_id).unwrap());
        let revoked = sync_with_peer(
            phone_storage.as_ref(),
            &paired_pc.device_id,
            None,
            SyncOptions::default(),
            Duration::from_secs(3),
        );
        assert!(revoked.is_err());

        server.stop();
        let _ = fs::remove_dir_all(&root_pc);
        let _ = fs::remove_dir_all(&root_phone);
    }

    #[test]
    fn sync_with_peer_honors_manual_ip_port_override() {
        let (root_pc, pc_storage) = temp_disk_storage("override_pc", "PC");
        let (root_phone, phone_storage) = temp_disk_storage("override_phone", "Phone");

        let server = SyncServer::start("127.0.0.1:0".parse().unwrap(), pc_storage, None).unwrap();
        let offer = server
            .open_pairing(vec![server.local_addr().to_string()], Some(60_000))
            .unwrap();
        let mut peer =
            pair_over_tcp(phone_storage.as_ref(), &offer, Duration::from_secs(3)).unwrap();
        // Simule un changement d'IP DHCP : l'ancienne adresse ne répond plus.
        peer.last_addr = Some("127.0.0.1:1".into());
        phone_storage.upsert_peer(peer.clone()).unwrap();

        let manual_addr = server.local_addr().to_string();
        let summary = sync_with_peer(
            phone_storage.as_ref(),
            &peer.device_id,
            Some(&manual_addr),
            SyncOptions::default(),
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(summary.peer_id, peer.device_id);

        // La nouvelle adresse valide a été mémorisée.
        let refreshed = phone_storage.get_peer(&peer.device_id).unwrap().unwrap();
        assert_eq!(refreshed.last_addr.as_deref(), Some(manual_addr.as_str()));

        server.stop();
        let _ = fs::remove_dir_all(&root_pc);
        let _ = fs::remove_dir_all(&root_phone);
    }
}
