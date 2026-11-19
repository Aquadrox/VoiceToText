//! Identité locale, offre QR code d'appairage et registre d'appareils appairés.

use std::fmt;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::crypto::SyncKey;
use super::{DEFAULT_PAIRING_TTL_MS, PROTOCOL_VERSION, Result, SyncError};

/// Schéma d'URI embarqué dans les QR codes d'appairage VoiceToText.
pub const PAIRING_SCHEME: &str = "vttsync://v1?";
const MAX_DEVICE_NAME_CHARS: usize = 64;
const MAX_OFFER_ADDRS: usize = 8;

/// Identifiant unique d'un appareil VoiceToText (32 caractères hexadécimaux
/// minuscules, 128 bits aléatoires).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(String);

impl DeviceId {
    /// Génère un nouvel identifiant aléatoire de 128 bits.
    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| SyncError::Random(e.to_string()))?;
        let mut hex = String::with_capacity(32);
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
        }
        Ok(Self(hex))
    }

    /// Analyse et valide un identifiant reçu (exactement 32 chiffres hexadécimaux).
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.len() != 32 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(SyncError::InvalidDeviceId(
                "attendu : 32 caractères hexadécimaux".into(),
            ));
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DeviceId").field(&self.0).finish()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Nettoie un nom d'appareil saisi ou reçu (sans caractères de contrôle, borné).
pub fn sanitize_device_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DEVICE_NAME_CHARS)
        .collect();
    if cleaned.is_empty() {
        "Appareil VoiceToText".into()
    } else {
        cleaned
    }
}

/// Offre éphémère encodée dans un QR code pour appairer deux appareils sur le
/// réseau local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingOffer {
    pub version: u8,
    pub device_id: DeviceId,
    pub device_name: String,
    /// Candidats `ip:port` accessibles sur le réseau local (IPv4 ou `[IPv6]:port`).
    pub addrs: Vec<String>,
    /// Clé éphémère de l'appairage (remplacée par une clé dédiée une fois appairés).
    pub key: SyncKey,
    /// Horodatage d'expiration (millisecondes depuis l'époque Unix).
    pub expires_at_ms: u64,
}

impl PairingOffer {
    /// Crée une nouvelle offre d'appairage avec clé éphémère aléatoire.
    pub fn create(
        identity: &LocalIdentity,
        addrs: Vec<String>,
        now_ms: u64,
        ttl_ms: Option<u64>,
    ) -> Result<Self> {
        let valid_addrs = validate_addrs(&addrs)?;
        let key = SyncKey::generate()?;
        let ttl = ttl_ms.unwrap_or(DEFAULT_PAIRING_TTL_MS);
        Ok(Self {
            version: PROTOCOL_VERSION,
            device_id: identity.device_id.clone(),
            device_name: sanitize_device_name(&identity.device_name),
            addrs: valid_addrs,
            key,
            expires_at_ms: now_ms.saturating_add(ttl),
        })
    }

    /// Empreinte visuelle partagée par les deux écrans pour confirmer l'appairage.
    pub fn fingerprint(&self) -> String {
        self.key.fingerprint()
    }

    pub const fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }

    pub fn check_valid(&self, now_ms: u64) -> Result<()> {
        if self.version != PROTOCOL_VERSION {
            return Err(SyncError::UnsupportedVersion(self.version));
        }
        if self.is_expired(now_ms) {
            return Err(SyncError::OfferExpired);
        }
        Ok(())
    }

    /// Encode l'offre en URI compacte (`vttsync://v1?...`) adaptée à un QR code.
    pub fn to_uri(&self) -> String {
        let name = pct_encode(&self.device_name);
        let addrs = pct_encode(&self.addrs.join(","));
        let key = self.key.to_base64();
        format!(
            "{PAIRING_SCHEME}id={}&n={name}&a={addrs}&k={key}&exp={}",
            self.device_id, self.expires_at_ms
        )
    }

    /// Décode et vérifie une URI lue depuis un QR code ou saisie manuellement.
    pub fn from_uri(uri: &str) -> Result<Self> {
        let uri = uri.trim();
        let query = uri
            .strip_prefix(PAIRING_SCHEME)
            .ok_or_else(|| SyncError::InvalidOffer("préfixe attendu : vttsync://v1?".into()))?;
        let mut id = None;
        let mut name = None;
        let mut addrs_raw = None;
        let mut key_raw = None;
        let mut exp_raw = None;

        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| SyncError::InvalidOffer("paramètre mal formé".into()))?;
            match k {
                "id" => id = Some(v),
                "n" => name = Some(pct_decode(v)?),
                "a" => addrs_raw = Some(pct_decode(v)?),
                "k" => key_raw = Some(v),
                "exp" => exp_raw = Some(v),
                _ => {}
            }
        }

        let device_id =
            DeviceId::parse(id.ok_or_else(|| SyncError::InvalidOffer("champ id absent".into()))?)?;
        let device_name = sanitize_device_name(
            name.as_deref()
                .ok_or_else(|| SyncError::InvalidOffer("champ n absent".into()))?,
        );
        let addrs_str =
            addrs_raw.ok_or_else(|| SyncError::InvalidOffer("champ a absent".into()))?;
        let raw_list: Vec<String> = addrs_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let addrs = validate_addrs(&raw_list)?;
        let key = SyncKey::from_base64(
            key_raw.ok_or_else(|| SyncError::InvalidOffer("champ k absent".into()))?,
        )?;
        let expires_at_ms: u64 = exp_raw
            .ok_or_else(|| SyncError::InvalidOffer("champ exp absent".into()))?
            .parse()
            .map_err(|_| SyncError::InvalidOffer("horodatage exp invalide".into()))?;

        Ok(Self {
            version: PROTOCOL_VERSION,
            device_id,
            device_name,
            addrs,
            key,
            expires_at_ms,
        })
    }
}

fn validate_addrs(addrs: &[String]) -> Result<Vec<String>> {
    if addrs.is_empty() {
        return Err(SyncError::InvalidOffer(
            "au moins une adresse locale est requise".into(),
        ));
    }
    let mut cleaned = Vec::new();
    for addr in addrs.iter().take(MAX_OFFER_ADDRS) {
        let addr = addr.trim();
        let parsed: SocketAddr = addr
            .parse()
            .map_err(|_| SyncError::InvalidOffer(format!("adresse invalide : {addr}")))?;
        if parsed.port() == 0 || parsed.ip().is_unspecified() {
            return Err(SyncError::InvalidOffer(format!(
                "adresse non joignable : {addr}"
            )));
        }
        let normalized = parsed.to_string();
        if !cleaned.contains(&normalized) {
            cleaned.push(normalized);
        }
    }
    if cleaned.is_empty() {
        return Err(SyncError::InvalidOffer(
            "au moins une adresse locale valide est requise".into(),
        ));
    }
    Ok(cleaned)
}

/// Identité persistante de l'appareil courant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIdentity {
    pub device_id: DeviceId,
    pub device_name: String,
}

impl LocalIdentity {
    pub fn generate(device_name: &str) -> Result<Self> {
        Ok(Self {
            device_id: DeviceId::generate()?,
            device_name: sanitize_device_name(device_name),
        })
    }
}

/// Appareil distant déjà appairé et autorisé à synchroniser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: DeviceId,
    pub device_name: String,
    #[serde(with = "sync_key_serde")]
    pub key: SyncKey,
    pub paired_at_ms: u64,
    #[serde(default)]
    pub last_sync_ms: Option<u64>,
    #[serde(default)]
    pub last_addr: Option<String>,
    #[serde(default = "default_true")]
    pub sync_settings: bool,
    #[serde(default = "default_true")]
    pub sync_meetings: bool,
}

const fn default_true() -> bool {
    true
}

impl Peer {
    pub fn fingerprint(&self) -> String {
        self.key.fingerprint()
    }
}

/// Registre persistant de l'identité locale et des appareils appairés.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStore {
    pub version: u8,
    pub identity: LocalIdentity,
    #[serde(default)]
    pub peers: Vec<Peer>,
}

impl PeerStore {
    pub fn new(identity: LocalIdentity) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            identity,
            peers: Vec::new(),
        }
    }

    /// Charge le registre depuis `path`, ou initialise une nouvelle identité
    /// locale si le fichier n'existe pas encore.
    pub fn load_or_init(path: &Path, default_device_name: &str) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => {
                let mut store: Self = serde_json::from_str(&content)?;
                store.identity.device_name = sanitize_device_name(&store.identity.device_name);
                Ok(store)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let store = Self::new(LocalIdentity::generate(default_device_name)?);
                store.save(path)?;
                Ok(store)
            }
            Err(e) => Err(SyncError::Io(e)),
        }
    }

    /// Enregistre le registre de manière atomique (fichier temporaire puis renommage).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn get(&self, id: &DeviceId) -> Option<&Peer> {
        self.peers.iter().find(|p| &p.device_id == id)
    }

    pub fn upsert(&mut self, mut peer: Peer) {
        peer.device_name = sanitize_device_name(&peer.device_name);
        if let Some(existing) = self
            .peers
            .iter_mut()
            .find(|p| p.device_id == peer.device_id)
        {
            *existing = peer;
        } else {
            self.peers.push(peer);
        }
    }

    pub fn remove(&mut self, id: &DeviceId) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| &p.device_id != id);
        self.peers.len() != before
    }

    pub fn mark_synced(&mut self, id: &DeviceId, at_ms: u64, addr: Option<String>) -> bool {
        let Some(peer) = self.peers.iter_mut().find(|p| &p.device_id == id) else {
            return false;
        };
        peer.last_sync_ms = Some(at_ms);
        if let Some(addr) = addr {
            peer.last_addr = Some(addr);
        }
        true
    }
}

mod sync_key_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::SyncKey;

    pub fn serialize<S: Serializer>(key: &SyncKey, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&key.to_base64())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SyncKey, D::Error> {
        let raw = String::deserialize(d)?;
        SyncKey::from_base64(&raw).map_err(serde::de::Error::custom)
    }
}

fn pct_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        let safe = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':');
        if safe {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn pct_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(SyncError::InvalidOffer(
                    "séquence pourcent tronquée dans l'URI".into(),
                ));
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .map_err(|_| SyncError::InvalidOffer("encodage URI invalide".into()))?;
            let val = u8::from_str_radix(hex, 16).map_err(|_| {
                SyncError::InvalidOffer("caractère hexadécimal URI invalide".into())
            })?;
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| SyncError::InvalidOffer("UTF-8 invalide dans l'URI".into()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vtt-sync-pairing-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        dir.join("sync-peers.json")
    }

    #[test]
    fn device_id_validates_hex_and_normalizes_case() {
        let generated = DeviceId::generate().unwrap();
        assert_eq!(generated.as_str().len(), 32);

        let parsed = DeviceId::parse("ABCDEF0123456789ABCDEF0123456789").unwrap();
        assert_eq!(parsed.as_str(), "abcdef0123456789abcdef0123456789");

        assert!(DeviceId::parse("").is_err());
        assert!(DeviceId::parse("../etc/passwd").is_err());
        assert!(DeviceId::parse("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }

    #[test]
    fn pairing_offer_uri_roundtrip_and_expiration() {
        let id = LocalIdentity::generate("PC de Benoît (Bureau)").unwrap();
        let offer = PairingOffer::create(
            &id,
            vec![
                "192.168.1.42:41739".into(),
                "[fe80::1]:41739".into(),
                "192.168.1.42:41739".into(), // doublon ignoré
            ],
            1_000_000,
            Some(60_000),
        )
        .unwrap();

        let uri = offer.to_uri();
        assert!(uri.starts_with(PAIRING_SCHEME));
        assert!(uri.len() < 220, "URI trop longue pour un QR code : {uri}");

        let decoded = PairingOffer::from_uri(&uri).unwrap();
        assert_eq!(decoded, offer);
        assert_eq!(decoded.addrs.len(), 2);
        assert_eq!(decoded.fingerprint(), offer.fingerprint());

        assert!(decoded.check_valid(1_059_999).is_ok());
        assert!(matches!(
            decoded.check_valid(1_060_000),
            Err(SyncError::OfferExpired)
        ));
    }

    #[test]
    fn pairing_offer_rejects_bad_scheme_and_unreachable_addrs() {
        assert!(PairingOffer::from_uri("https://example.com").is_err());
        assert!(PairingOffer::from_uri("vttsync://v1?id=bad").is_err());

        let id = LocalIdentity::generate("PC").unwrap();
        assert!(PairingOffer::create(&id, vec![], 0, None).is_err());
        assert!(PairingOffer::create(&id, vec!["0.0.0.0:41739".into()], 0, None).is_err());
        assert!(PairingOffer::create(&id, vec!["192.168.1.5:0".into()], 0, None).is_err());
        assert!(PairingOffer::create(&id, vec!["pas-une-ip".into()], 0, None).is_err());
    }

    #[test]
    fn peer_store_persists_and_updates_peers() {
        let path = temp_file("store");
        let mut store = PeerStore::load_or_init(&path, "PC principal").unwrap();
        assert!(path.is_file());
        let local_id = store.identity.device_id.clone();

        let peer_id = DeviceId::generate().unwrap();
        let peer_key = SyncKey::generate().unwrap();
        store.upsert(Peer {
            device_id: peer_id.clone(),
            device_name: "Xiaomi 17T Pro".into(),
            key: peer_key.clone(),
            paired_at_ms: 1000,
            last_sync_ms: None,
            last_addr: Some("192.168.1.88:41739".into()),
            sync_settings: true,
            sync_meetings: true,
        });
        assert!(store.mark_synced(&peer_id, 2000, Some("192.168.1.89:41739".into())));
        store.save(&path).unwrap();

        let reloaded = PeerStore::load_or_init(&path, "Autre nom ignoré").unwrap();
        assert_eq!(reloaded.identity.device_id, local_id);
        let peer = reloaded.get(&peer_id).unwrap();
        assert_eq!(peer.key, peer_key);
        assert_eq!(peer.last_sync_ms, Some(2000));
        assert_eq!(peer.last_addr.as_deref(), Some("192.168.1.89:41739"));

        let mut mutated = reloaded;
        assert!(mutated.remove(&peer_id));
        assert!(!mutated.remove(&peer_id));
        assert!(mutated.get(&peer_id).is_none());

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
