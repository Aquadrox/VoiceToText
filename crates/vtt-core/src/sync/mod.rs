//! Synchronisation locale Wi-Fi (pair-à-pair, chiffrée de bout en bout).
//!
//! Brique portable partagée par le PC, le Steam Deck et Android :
//! - [`crypto`] : clés secrètes effacées en mémoire ([`SyncKey`]), empreinte
//!   vérifiable, authentification mutuelle ([`Handshake`]) et chiffrement
//!   authentifié ChaCha20-Poly1305 par session ([`SessionCipher`]) ;
//! - [`pairing`] : identité locale ([`DeviceId`]), offre QR code compacte
//!   ([`PairingOffer`], URI `vttsync://v1?...`) et registre des appareils
//!   appairés ([`PeerStore`]) ;
//! - [`merge`] : fusion déterministe de la configuration portable (dictionnaire,
//!   snippets, styles, prompts, profils LLM — **jamais les clés API**) et
//!   différentiel des réunions ([`MeetingDigest`], [`diff_meetings`]) ;
//! - [`session`] : stockage local ([`SyncStorage`], [`DiskSyncStorage`]) et
//!   orchestration des sessions d'appairage et de synchronisation (config +
//!   réunions `meeting.json`) ;
//! - [`transport`] : serveur et client TCP local ([`SyncServer`],
//!   [`pair_over_tcp`], [`sync_with_peer`]) ;
//! - [`discovery`] : découverte mDNS (`_voicetotext._tcp.local.`) sans démon
//!   système, avec secours direct IP:port.

pub mod channel;
pub mod crypto;
pub mod direct_pairing;
pub mod discovery;
pub mod merge;
pub mod pairing;
pub mod session;
pub mod transport;

pub use channel::{EncryptedChannel, RemotePeerInfo};
pub use crypto::{
    AuthConfirm, Hello, HelloAck, SessionCipher, SessionRole, SyncKey, auth_tag, decode_frame,
    derive_session_nonce, encode_frame, verify_tag,
};
pub use direct_pairing::{
    DEFAULT_DIRECT_PAIRING_TTL_MS, DirectDecisionHandle, DirectPairInit, DirectPairingHandle,
    PendingDirectPairing, format_pairing_code, start_direct_pairing_over_tcp,
};
pub use discovery::{DiscoveredPeer, MdnsPacket, MdnsService};
pub use merge::{
    MeetingDiff, MeetingDigest, MergeReport, SyncMessage, SyncableConfig, diff_meetings,
    meeting_digest, merge_config, sanitize_meeting_for_sync,
};
pub use pairing::{DeviceId, LocalIdentity, PairingOffer, Peer, PeerStore};
pub use session::{
    DiskSyncStorage, SyncOptions, SyncStorage, SyncSummary, pair_as_initiator, pair_as_responder,
    sync_as_initiator, sync_as_responder, valid_meeting_id,
};
pub use transport::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_IO_TIMEOUT, SyncEvent, SyncServer, detect_local_addrs, now_ms,
    pair_over_tcp, sync_with_peer,
};

use thiserror::Error;

/// Version actuelle du protocole de synchronisation pair-à-pair.
pub const PROTOCOL_VERSION: u8 = 1;

/// Type de service mDNS / NSD annoncé sur le réseau local.
pub const SERVICE_TYPE: &str = "_voicetotext._tcp.local.";

/// Port TCP par défaut pour l'écoute sur le réseau local.
pub const DEFAULT_PORT: u16 = 41739;

/// Durée de validité par défaut d'un QR code d'appairage (5 minutes).
pub const DEFAULT_PAIRING_TTL_MS: u64 = 5 * 60 * 1000;

/// Taille maximale d'une trame réseau (16 Mio) : protège contre les allocations
/// excessives sur une trame corrompue ou malveillante.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("aléa système indisponible : {0}")]
    Random(String),
    #[error("clé de synchronisation invalide : {0}")]
    InvalidKey(String),
    #[error("identifiant d'appareil invalide : {0}")]
    InvalidDeviceId(String),
    #[error("offre d'appairage invalide : {0}")]
    InvalidOffer(String),
    #[error("offre d'appairage expirée")]
    OfferExpired,
    #[error("aucun appairage n'est ouvert sur cet appareil")]
    PairingClosed,
    #[error("appairage direct refusé ou annulé")]
    PairingRejected,
    #[error("un appairage direct est déjà en cours sur cet appareil")]
    DirectPairingBusy,
    #[error("appareil non appairé : {0}")]
    UnknownPeer(DeviceId),
    #[error("version de protocole non prise en charge : {0}")]
    UnsupportedVersion(u8),
    #[error("authentification refusée : l'appareil ne partage pas la même clé")]
    AuthenticationFailed,
    #[error("échec de chiffrement : {0}")]
    Encrypt(String),
    #[error("échec de déchiffrement ou message altéré")]
    Decrypt,
    #[error("compteur de messages épuisé : ouvrez une nouvelle session")]
    NonceExhausted,
    #[error("trame trop grande ({size} octets, maximum {MAX_FRAME_BYTES})")]
    FrameTooLarge { size: usize },
    #[error("configuration synchronisée invalide : {0}")]
    InvalidConfig(String),
    #[error("message de synchronisation inattendu : {0}")]
    Protocol(String),
    #[error("erreur renvoyée par l'appareil distant : {0}")]
    Remote(String),
    #[error("impossible de joindre l'appareil distant : {0}")]
    Connect(String),
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
    #[error("erreur JSON : {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SyncError>;
