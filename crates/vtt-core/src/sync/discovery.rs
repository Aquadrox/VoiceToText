//! Découverte mDNS locale (`_voicetotext._tcp.local.`, RFC 6762 / RFC 6763).
//!
//! Implémentation 100 % Rust pur sans démon système (ni Avahi ni Bonjour) :
//! - fonctionne de manière identique sur Linux, Windows, macOS et Android ;
//! - interopérable avec `NsdManager` (Android), Avahi et Bonjour grâce au
//!   format filaire DNS-SD standard (enregistrements PTR, SRV, TXT et A) ;
//! - dégradation gracieuse : si le multicast est bloqué sur le réseau,
//!   [`MdnsService::start`] renvoie une erreur propre et l'utilisateur peut
//!   toujours utiliser le QR code ou la saisie directe `IP:port`.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};

use super::pairing::{DeviceId, LocalIdentity, sanitize_device_name};
use super::transport::{detect_primary_ipv4, now_ms};
use super::{PROTOCOL_VERSION, Result, SERVICE_TYPE};

/// Adresse multicast mDNS standard (RFC 6762).
pub const MDNS_MULTICAST_V4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// Port UDP mDNS standard.
pub const MDNS_PORT: u16 = 5353;

/// Durée de vie (TTL) annoncée pour nos enregistrements mDNS (2 minutes).
pub const DEFAULT_MDNS_TTL_SECS: u32 = 120;

/// Intervalle de ré-annonce périodique sur le réseau local.
const REANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;
const TYPE_ANY: u16 = 255;

const CLASS_IN: u16 = 1;
const CLASS_IN_FLUSH: u16 = 0x8001;

/// Appareil VoiceToText détecté sur le réseau local via mDNS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: DeviceId,
    pub device_name: String,
    pub addr: SocketAddr,
    pub protocol: u8,
    pub last_seen_ms: u64,
    pub ttl_secs: u32,
}

/// Résultat du décodage d'un paquet UDP mDNS entrant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdnsPacket {
    /// Un pair sur le réseau demande la liste des services `_voicetotext._tcp.local.`.
    Query,
    /// Réponse contenant des appareils annoncés et/ou partis (`TTL = 0`).
    Response {
        discovered: Vec<DiscoveredPeer>,
        departed: Vec<DeviceId>,
    },
}

/// Construit une requête mDNS standard pour découvrir `_voicetotext._tcp.local.`.
pub fn build_ptr_query() -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    // Header DNS : ID=0, Flags=0 (Query), QDCOUNT=1, AN/NS/AR=0
    buf.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    write_dns_name(&mut buf, SERVICE_TYPE);
    buf.extend_from_slice(&TYPE_PTR.to_be_bytes());
    buf.extend_from_slice(&CLASS_IN.to_be_bytes());
    buf
}

/// Construit un paquet d'annonce mDNS (ou d'au-revoir si `ttl_secs == 0`)
/// exposant PTR, SRV, TXT (`v`, `id`, `name`) et A pour cet appareil.
pub fn build_service_announcement(
    identity: &LocalIdentity,
    ip: Ipv4Addr,
    port: u16,
    ttl_secs: u32,
) -> Vec<u8> {
    let short = &identity.device_id.as_str()[..12];
    let instance_name = format!("vtt-{short}.{SERVICE_TYPE}");
    let host_name = format!("vtt-{short}.local.");

    let mut buf = Vec::with_capacity(256);
    // Header DNS : ID=0, Flags=0x8400 (QR=1, AA=1), QDCOUNT=0, ANCOUNT=4
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&0x8400u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&4u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());

    // 1. PTR _voicetotext._tcp.local. -> <instance>
    let mut ptr_rdata = Vec::new();
    write_dns_name(&mut ptr_rdata, &instance_name);
    write_rr(
        &mut buf,
        SERVICE_TYPE,
        TYPE_PTR,
        CLASS_IN,
        ttl_secs,
        &ptr_rdata,
    );

    // 2. SRV <instance> -> priority=0, weight=0, port, target=<host>
    let mut srv_rdata = Vec::new();
    srv_rdata.extend_from_slice(&0u16.to_be_bytes());
    srv_rdata.extend_from_slice(&0u16.to_be_bytes());
    srv_rdata.extend_from_slice(&port.to_be_bytes());
    write_dns_name(&mut srv_rdata, &host_name);
    write_rr(
        &mut buf,
        &instance_name,
        TYPE_SRV,
        CLASS_IN_FLUSH,
        ttl_secs,
        &srv_rdata,
    );

    // 3. TXT <instance> -> v=1, id=<device_id>, name=<device_name>
    let mut txt_rdata = Vec::new();
    let v_entry = format!("v={PROTOCOL_VERSION}");
    let id_entry = format!("id={}", identity.device_id.as_str());
    let name_entry = format!("name={}", sanitize_device_name(&identity.device_name));
    write_txt_entry(&mut txt_rdata, &v_entry);
    write_txt_entry(&mut txt_rdata, &id_entry);
    write_txt_entry(&mut txt_rdata, &name_entry);
    write_rr(
        &mut buf,
        &instance_name,
        TYPE_TXT,
        CLASS_IN_FLUSH,
        ttl_secs,
        &txt_rdata,
    );

    // 4. A <host> -> IPv4
    write_rr(
        &mut buf,
        &host_name,
        TYPE_A,
        CLASS_IN_FLUSH,
        ttl_secs,
        &ip.octets(),
    );

    buf
}

/// Décode un paquet UDP mDNS entrant. Retourne `None` s'il ne concerne pas
/// `_voicetotext._tcp.local.` ou s'il est mal formé.
pub fn parse_mdns_packet(packet: &[u8], now_ms: u64) -> Option<MdnsPacket> {
    if packet.len() < 12 {
        return None;
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    let is_response = (flags & 0x8000) != 0;
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let ancount = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let nscount = u16::from_be_bytes([packet[8], packet[9]]) as usize;
    let arcount = u16::from_be_bytes([packet[10], packet[11]]) as usize;

    let mut offset = 12;
    let service_lc = normalize_dns_name(SERVICE_TYPE);

    if !is_response {
        for _ in 0..qdcount {
            let qname = read_dns_name(packet, &mut offset)?;
            if offset + 4 > packet.len() {
                return None;
            }
            let qtype = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
            offset += 4;
            if normalize_dns_name(&qname) == service_lc && (qtype == TYPE_PTR || qtype == TYPE_ANY)
            {
                return Some(MdnsPacket::Query);
            }
        }
        return None;
    }

    // Saute les questions éventuelles d'une réponse.
    for _ in 0..qdcount {
        let _ = read_dns_name(packet, &mut offset)?;
        if offset + 4 > packet.len() {
            return None;
        }
        offset += 4;
    }

    let total_rrs = ancount.saturating_add(nscount).saturating_add(arcount);
    let mut ptr_instances: Vec<(String, u32)> = Vec::new();
    let mut srvs: HashMap<String, (u16, String, u32)> = HashMap::new();
    let mut txts: HashMap<String, (HashMap<String, String>, u32)> = HashMap::new();
    let mut ipv4s: HashMap<String, Ipv4Addr> = HashMap::new();

    for _ in 0..total_rrs {
        if offset >= packet.len() {
            break;
        }
        let name = read_dns_name(packet, &mut offset)?;
        if offset + 10 > packet.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let rclass = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) & 0x7FFF;
        let ttl = u32::from_be_bytes([
            packet[offset + 4],
            packet[offset + 5],
            packet[offset + 6],
            packet[offset + 7],
        ]);
        let rdlength = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + rdlength > packet.len() {
            return None;
        }
        let rdata_start = offset;
        offset += rdlength;

        if rclass != CLASS_IN {
            continue;
        }
        let name_lc = normalize_dns_name(&name);
        match rtype {
            TYPE_PTR if name_lc == service_lc => {
                let mut r_off = rdata_start;
                if let Some(target) = read_dns_name(packet, &mut r_off) {
                    ptr_instances.push((normalize_dns_name(&target), ttl));
                }
            }
            TYPE_SRV if rdlength >= 6 => {
                let port = u16::from_be_bytes([packet[rdata_start + 4], packet[rdata_start + 5]]);
                let mut r_off = rdata_start + 6;
                if let Some(target) = read_dns_name(packet, &mut r_off) {
                    srvs.insert(name_lc, (port, normalize_dns_name(&target), ttl));
                }
            }
            TYPE_TXT => {
                let map = parse_txt_rdata(&packet[rdata_start..rdata_start + rdlength]);
                txts.insert(name_lc, (map, ttl));
            }
            TYPE_A if rdlength == 4 => {
                let ip = Ipv4Addr::new(
                    packet[rdata_start],
                    packet[rdata_start + 1],
                    packet[rdata_start + 2],
                    packet[rdata_start + 3],
                );
                ipv4s.insert(name_lc, ip);
            }
            _ => {}
        }
    }

    // Si un pair n'a envoyé que SRV+TXT+A sans PTR (ex. mise à jour), on inspecte
    // aussi les instances connues dans `srvs` qui se terminent par `_voicetotext._tcp.local`.
    let suffix = format!(".{service_lc}");
    let mut candidate_instances: Vec<(String, u32)> = ptr_instances;
    for (inst, (_, _, ttl)) in &srvs {
        if inst.ends_with(&suffix) && !candidate_instances.iter().any(|(i, _)| i == inst) {
            candidate_instances.push((inst.clone(), *ttl));
        }
    }

    let mut discovered = Vec::new();
    let mut departed = Vec::new();

    for (instance, ptr_ttl) in candidate_instances {
        let Some((txt_map, txt_ttl)) = txts.get(&instance) else {
            continue;
        };
        let Some(raw_id) = txt_map.get("id") else {
            continue;
        };
        let Ok(device_id) = DeviceId::parse(raw_id) else {
            continue;
        };

        let srv_info = srvs.get(&instance);
        let effective_ttl = srv_info
            .map(|(_, _, t)| *t)
            .unwrap_or(ptr_ttl)
            .min(ptr_ttl)
            .min(*txt_ttl);

        if effective_ttl == 0 {
            if !departed.contains(&device_id) {
                departed.push(device_id);
            }
            continue;
        }

        let Some((port, target_host, _)) = srv_info else {
            continue;
        };
        if *port == 0 {
            continue;
        }
        let Some(&ip) = ipv4s.get(target_host) else {
            continue;
        };
        if ip.is_unspecified() {
            continue;
        }

        let protocol = txt_map
            .get("v")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(PROTOCOL_VERSION);
        let device_name = txt_map
            .get("name")
            .map(|n| sanitize_device_name(n))
            .unwrap_or_else(|| sanitize_device_name(&instance));

        discovered.push(DiscoveredPeer {
            device_id,
            device_name,
            addr: SocketAddr::new(IpAddr::V4(ip), *port),
            protocol,
            last_seen_ms: now_ms,
            ttl_secs: effective_ttl,
        });
    }

    if discovered.is_empty() && departed.is_empty() {
        None
    } else {
        Some(MdnsPacket::Response {
            discovered,
            departed,
        })
    }
}

/// Cache des pairs découverts avec éviction automatique après expiration du TTL.
#[derive(Debug, Default)]
struct PeerCache {
    self_id: Option<DeviceId>,
    entries: BTreeMap<DeviceId, DiscoveredPeer>,
}

impl PeerCache {
    fn new(self_id: DeviceId) -> Self {
        Self {
            self_id: Some(self_id),
            entries: BTreeMap::new(),
        }
    }

    fn apply(&mut self, discovered: Vec<DiscoveredPeer>, departed: Vec<DeviceId>, now_ms: u64) {
        for id in departed {
            self.entries.remove(&id);
        }
        for peer in discovered {
            if self.self_id.as_ref() == Some(&peer.device_id) {
                continue;
            }
            self.entries.insert(peer.device_id.clone(), peer);
        }
        self.prune_expired(now_ms);
    }

    fn prune_expired(&mut self, now_ms: u64) {
        self.entries.retain(|_, peer| {
            let ttl_ms = u64::from(peer.ttl_secs).saturating_mul(1_000);
            now_ms.saturating_sub(peer.last_seen_ms) <= ttl_ms
        });
    }

    fn snapshot(&mut self, now_ms: u64) -> Vec<DiscoveredPeer> {
        self.prune_expired(now_ms);
        self.entries.values().cloned().collect()
    }
}

/// Service mDNS local : annonce cet appareil sur `_voicetotext._tcp.local.` et
/// maintient la liste des autres appareils VoiceToText sur le Wi-Fi.
pub struct MdnsService {
    socket: UdpSocket,
    identity: LocalIdentity,
    advertise_ip: Ipv4Addr,
    port: u16,
    cache: Arc<Mutex<PeerCache>>,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MdnsService {
    /// Démarre l'annonce et l'écoute mDNS sur `224.0.0.251:5353`.
    pub fn start(
        identity: LocalIdentity,
        port: u16,
        advertise_ip: Option<Ipv4Addr>,
    ) -> Result<Self> {
        let ip = advertise_ip
            .or_else(detect_primary_ipv4)
            .unwrap_or(Ipv4Addr::LOCALHOST);

        let socket = bind_mdns_socket()?;
        let worker_sock = socket.try_clone()?;

        let cache = Arc::new(Mutex::new(PeerCache::new(identity.device_id.clone())));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let w_cache = Arc::clone(&cache);
        let w_stop = Arc::clone(&stop_flag);
        let w_id = identity.clone();

        // Annonce initiale + découverte immédiate des pairs présents.
        let dest = SocketAddrV4::new(MDNS_MULTICAST_V4, MDNS_PORT);
        let _ = socket.send_to(
            &build_service_announcement(&identity, ip, port, DEFAULT_MDNS_TTL_SECS),
            dest,
        );
        let _ = socket.send_to(&build_ptr_query(), dest);

        let handle = thread::Builder::new()
            .name("vtt-sync-mdns".into())
            .spawn(move || {
                mdns_loop(worker_sock, w_id, ip, port, w_cache, w_stop);
            })?;

        Ok(Self {
            socket,
            identity,
            advertise_ip: ip,
            port,
            cache,
            stop_flag,
            handle: Some(handle),
        })
    }

    /// Liste les appareils VoiceToText actuellement visibles sur le réseau local.
    pub fn peers(&self) -> Vec<DiscoveredPeer> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.snapshot(now_ms())
    }

    /// Envoie immédiatement une requête mDNS + une ré-annonce.
    pub fn refresh(&self) -> Result<()> {
        let dest = SocketAddrV4::new(MDNS_MULTICAST_V4, MDNS_PORT);
        self.socket.send_to(&build_ptr_query(), dest)?;
        self.socket.send_to(
            &build_service_announcement(
                &self.identity,
                self.advertise_ip,
                self.port,
                DEFAULT_MDNS_TTL_SECS,
            ),
            dest,
        )?;
        Ok(())
    }

    /// Arrête le service mDNS et diffuse un paquet « au revoir » (`TTL = 0`).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.stop_flag.swap(true, Ordering::SeqCst) {
            return;
        }
        let dest = SocketAddrV4::new(MDNS_MULTICAST_V4, MDNS_PORT);
        let goodbye = build_service_announcement(&self.identity, self.advertise_ip, self.port, 0);
        let _ = self.socket.send_to(&goodbye, dest);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MdnsService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn bind_mdns_socket() -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    {
        let _ = socket.set_reuse_port(true);
    }
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT);
    socket.bind(&bind_addr.into())?;

    let udp: UdpSocket = socket.into();
    udp.join_multicast_v4(&MDNS_MULTICAST_V4, &Ipv4Addr::UNSPECIFIED)?;
    let _ = udp.set_multicast_loop_v4(true);
    udp.set_read_timeout(Some(Duration::from_millis(120)))?;
    Ok(udp)
}

fn mdns_loop(
    socket: UdpSocket,
    identity: LocalIdentity,
    ip: Ipv4Addr,
    port: u16,
    cache: Arc<Mutex<PeerCache>>,
    stop_flag: Arc<AtomicBool>,
) {
    let dest = SocketAddrV4::new(MDNS_MULTICAST_V4, MDNS_PORT);
    let mut buf = [0u8; 4096];
    let mut last_announce = Instant::now();

    while !stop_flag.load(Ordering::Relaxed) {
        if last_announce.elapsed() >= REANNOUNCE_INTERVAL {
            let pkt = build_service_announcement(&identity, ip, port, DEFAULT_MDNS_TTL_SECS);
            let _ = socket.send_to(&pkt, dest);
            last_announce = Instant::now();
        }

        match socket.recv_from(&mut buf) {
            Ok((n, _src)) => {
                let ts = now_ms();
                match parse_mdns_packet(&buf[..n], ts) {
                    Some(MdnsPacket::Query) => {
                        let pkt =
                            build_service_announcement(&identity, ip, port, DEFAULT_MDNS_TTL_SECS);
                        let _ = socket.send_to(&pkt, dest);
                    }
                    Some(MdnsPacket::Response {
                        discovered,
                        departed,
                    }) => {
                        let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
                        guard.apply(discovered, departed, ts);
                    }
                    None => {}
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                if !stop_flag.load(Ordering::Relaxed) {
                    log::debug!("sync/mdns: recv_from : {e}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

fn write_rr(buf: &mut Vec<u8>, name: &str, rtype: u16, rclass: u16, ttl: u32, rdata: &[u8]) {
    write_dns_name(buf, name);
    buf.extend_from_slice(&rtype.to_be_bytes());
    buf.extend_from_slice(&rclass.to_be_bytes());
    buf.extend_from_slice(&ttl.to_be_bytes());
    let len = u16::try_from(rdata.len()).unwrap_or(u16::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&rdata[..usize::from(len)]);
}

fn write_dns_name(buf: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        let bytes = label.as_bytes();
        let len = bytes.len().min(63);
        buf.push(len as u8);
        buf.extend_from_slice(&bytes[..len]);
    }
    buf.push(0);
}

fn write_txt_entry(buf: &mut Vec<u8>, entry: &str) {
    let bytes = entry.as_bytes();
    let len = bytes.len().min(255);
    buf.push(len as u8);
    buf.extend_from_slice(&bytes[..len]);
}

fn read_dns_name(packet: &[u8], offset: &mut usize) -> Option<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = *offset;
    let mut jumped = false;
    let mut jumps = 0;

    loop {
        if pos >= packet.len() {
            return None;
        }
        let len = packet[pos];
        if len == 0 {
            if !jumped {
                *offset = pos + 1;
            }
            break;
        }
        if (len & 0xC0) == 0xC0 {
            if pos + 1 >= packet.len() {
                return None;
            }
            let ptr = (usize::from(len & 0x3F) << 8) | usize::from(packet[pos + 1]);
            if ptr >= packet.len() {
                return None;
            }
            if !jumped {
                *offset = pos + 2;
                jumped = true;
            }
            jumps += 1;
            if jumps > 16 {
                // Protection contre une boucle de pointeurs malveillante.
                return None;
            }
            pos = ptr;
            continue;
        }
        if (len & 0xC0) != 0 {
            return None;
        }
        let label_len = usize::from(len);
        pos += 1;
        if pos + label_len > packet.len() {
            return None;
        }
        let label = std::str::from_utf8(&packet[pos..pos + label_len]).ok()?;
        labels.push(label.to_owned());
        pos += label_len;
    }

    Some(labels.join("."))
}

fn normalize_dns_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn parse_txt_rdata(rdata: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < rdata.len() {
        let len = usize::from(rdata[i]);
        i += 1;
        if len == 0 || i + len > rdata.len() {
            break;
        }
        if let Ok(entry) = std::str::from_utf8(&rdata[i..i + len])
            && let Some((k, v)) = entry.split_once('=')
        {
            out.insert(k.to_ascii_lowercase(), v.to_owned());
        }
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptr_query_roundtrips_and_ignores_unrelated_services() {
        let query = build_ptr_query();
        assert_eq!(parse_mdns_packet(&query, 1_000), Some(MdnsPacket::Query));

        // Requête pour un autre service (_http._tcp.local.) : ignorée.
        let mut other = Vec::new();
        other.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        write_dns_name(&mut other, "_http._tcp.local.");
        other.extend_from_slice(&TYPE_PTR.to_be_bytes());
        other.extend_from_slice(&CLASS_IN.to_be_bytes());
        assert_eq!(parse_mdns_packet(&other, 1_000), None);
    }

    #[test]
    fn service_announcement_and_goodbye_roundtrip() {
        let id = LocalIdentity::generate("Steam Deck OLED").unwrap();
        let ip = Ipv4Addr::new(192, 168, 1, 42);
        let pkt = build_service_announcement(&id, ip, 41739, 120);

        let Some(MdnsPacket::Response {
            discovered,
            departed,
        }) = parse_mdns_packet(&pkt, 50_000)
        else {
            panic!("réponse mDNS attendue");
        };
        assert!(departed.is_empty());
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].device_id, id.device_id);
        assert_eq!(discovered[0].device_name, "Steam Deck OLED");
        assert_eq!(discovered[0].addr, SocketAddr::new(IpAddr::V4(ip), 41739));
        assert_eq!(discovered[0].protocol, PROTOCOL_VERSION);
        assert_eq!(discovered[0].ttl_secs, 120);

        // Paquet « au revoir » (TTL = 0) : signalé comme parti.
        let bye = build_service_announcement(&id, ip, 41739, 0);
        let Some(MdnsPacket::Response {
            discovered,
            departed,
        }) = parse_mdns_packet(&bye, 51_000)
        else {
            panic!("réponse goodbye attendue");
        };
        assert!(discovered.is_empty());
        assert_eq!(departed, vec![id.device_id]);
    }

    #[test]
    fn decoder_handles_dns_compression_pointers_and_rejects_cycles() {
        // Construit un paquet avec un premier nom complet (`_voicetotext._tcp.local.`)
        // suivi d'un nom compressé (`android` + pointeur 0xC0 vers le premier nom).
        let mut pkt = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let service_offset = pkt.len();
        write_dns_name(&mut pkt, SERVICE_TYPE);
        let compressed_offset = pkt.len();
        pkt.push(7);
        pkt.extend_from_slice(b"android");
        pkt.push(0xC0 | ((service_offset >> 8) as u8));
        pkt.push((service_offset & 0xFF) as u8);

        let mut off = compressed_offset;
        assert_eq!(
            read_dns_name(&pkt, &mut off).as_deref(),
            Some("android._voicetotext._tcp.local")
        );
        assert_eq!(off, pkt.len());

        // Paquet avec boucle de pointeurs (0xC0 0x0C pointant sur lui-même) : rejeté sans boucle infinie.
        let cyclic = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xC0, 0x0C, 0, 12, 0, 1];
        assert_eq!(parse_mdns_packet(&cyclic, 0), None);
    }

    #[test]
    fn peer_cache_ignores_self_handles_goodbye_and_expires_stale_entries() {
        let self_id = LocalIdentity::generate("Mon PC").unwrap();
        let phone_id = LocalIdentity::generate("Mon Android").unwrap();
        let mut cache = PeerCache::new(self_id.device_id.clone());

        let self_peer = DiscoveredPeer {
            device_id: self_id.device_id,
            device_name: self_id.device_name,
            addr: "192.168.1.10:41739".parse().unwrap(),
            protocol: PROTOCOL_VERSION,
            last_seen_ms: 1_000,
            ttl_secs: 60,
        };
        let phone_peer = DiscoveredPeer {
            device_id: phone_id.device_id.clone(),
            device_name: phone_id.device_name,
            addr: "192.168.1.25:41739".parse().unwrap(),
            protocol: PROTOCOL_VERSION,
            last_seen_ms: 1_000,
            ttl_secs: 60,
        };

        cache.apply(vec![self_peer, phone_peer.clone()], Vec::new(), 1_000);
        assert_eq!(cache.snapshot(30_000).len(), 1);
        assert_eq!(cache.snapshot(30_000)[0].device_id, phone_id.device_id);

        // Expiration après 60 s.
        assert!(cache.snapshot(70_000).is_empty());

        // Ré-apparition puis goodbye immédiat.
        let mut refreshed = phone_peer;
        refreshed.last_seen_ms = 80_000;
        cache.apply(vec![refreshed], Vec::new(), 80_000);
        assert_eq!(cache.snapshot(80_000).len(), 1);
        cache.apply(Vec::new(), vec![phone_id.device_id], 81_000);
        assert!(cache.snapshot(81_000).is_empty());
    }
}
