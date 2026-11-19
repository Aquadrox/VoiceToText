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
//!   différentiel des réunions ([`MeetingDigest`], [`diff_meetings`]).

pub mod channel;
pub mod crypto;
pub mod merge;
pub mod pairing;

pub use channel::{EncryptedChannel, RemotePeerInfo};
pub use crypto::{
    AuthConfirm, Hello, HelloAck, SessionCipher, SessionRole, SyncKey, auth_tag, decode_frame,
    derive_session_nonce, encode_frame, verify_tag,
};
pub use merge::{
    MeetingDiff, MeetingDigest, MergeReport, SyncMessage, SyncableConfig, diff_meetings,
    meeting_digest, merge_config, sanitize_meeting_for_sync,
};
pub use pairing::{DeviceId, LocalIdentity, PairingOffer, Peer, PeerStore};

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
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
    #[error("erreur JSON : {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SyncError>;
