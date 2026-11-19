//! Chiffrement de bout en bout (ChaCha20-Poly1305) et authentification
//! mutuelle pour la synchronisation locale.
//!
//! Chaque session dérive deux sous-clés directionnelles distinctes à partir de
//! la clé partagée (`SyncKey`) et des nonces aléatoires échangés par les deux
//! appareils. Chaque trame utilise un compteur monotone sur 64 bits comme nonce
//! AEAD et lie son numéro, son sens et la version du protocole dans les données
//! associées (AAD) : toute modification, relecture ou réordonnancement est
//! rejeté.

use std::fmt;
use std::io::{Read, Write};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{MAX_FRAME_BYTES, PROTOCOL_VERSION, Result, SyncError};
use crate::sync::pairing::DeviceId;

/// Clé secrète partagée de 256 bits, effacée de la mémoire à sa libération.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SyncKey([u8; 32]);

impl SyncKey {
    /// Génère une nouvelle clé aléatoire cryptographiquement sûre.
    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| SyncError::Random(e.to_string()))?;
        Ok(Self(bytes))
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Représentation compacte (base64url sans padding) pour les QR codes et le
    /// stockage local des pairs.
    pub fn to_base64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn from_base64(encoded: &str) -> Result<Self> {
        let raw = URL_SAFE_NO_PAD
            .decode(encoded.trim())
            .map_err(|e| SyncError::InvalidKey(e.to_string()))?;
        let bytes: [u8; 32] = raw
            .try_into()
            .map_err(|_| SyncError::InvalidKey("longueur attendue : 32 octets".into()))?;
        Ok(Self(bytes))
    }

    /// Empreinte courte vérifiable visuellement sur les deux écrans lors de
    /// l'appairage (ex. `A1B2-C3D4-E5F6-0718`).
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"vtt-sync-fingerprint-v1\0");
        hasher.update(self.0);
        let digest = hasher.finalize();
        format!(
            "{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}",
            digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]
        )
    }
}

impl PartialEq for SyncKey {
    fn eq(&self, other: &Self) -> bool {
        constant_time_eq(&self.0, &other.0)
    }
}

impl Eq for SyncKey {}

impl fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SyncKey([REDACTED; fp={}])", self.fingerprint())
    }
}

/// Rôle d'un appareil au sein d'une connexion (détermine le sens des sous-clés).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// Appareil qui ouvre la connexion TCP (ex. le téléphone qui scanne le QR).
    Initiator,
    /// Appareil qui écoute (ex. le PC qui affiche le QR).
    Responder,
}

const DIR_I2R: u8 = 0x01;
const DIR_R2I: u8 = 0x02;

/// Canal chiffré bidirectionnel pour une session de synchronisation.
pub struct SessionCipher {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_dir: u8,
    recv_dir: u8,
    send_counter: u64,
    recv_counter: u64,
}

impl fmt::Debug for SessionCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionCipher")
            .field("send_counter", &self.send_counter)
            .field("recv_counter", &self.recv_counter)
            .finish()
    }
}

impl SessionCipher {
    /// Initialise une session chiffrée à partir de la clé partagée et du nonce
    /// de session issu du handshake.
    pub fn new(shared_key: &SyncKey, session_nonce: &[u8; 32], role: SessionRole) -> Self {
        let mut i2r = derive_subkey(shared_key, session_nonce, b"i2r");
        let mut r2i = derive_subkey(shared_key, session_nonce, b"r2i");
        let (send_bytes, recv_bytes, send_dir, recv_dir) = match role {
            SessionRole::Initiator => (i2r, r2i, DIR_I2R, DIR_R2I),
            SessionRole::Responder => (r2i, i2r, DIR_R2I, DIR_I2R),
        };
        let send_cipher = ChaCha20Poly1305::new(Key::from_slice(&send_bytes));
        let recv_cipher = ChaCha20Poly1305::new(Key::from_slice(&recv_bytes));
        i2r.zeroize();
        r2i.zeroize();
        Self {
            send_cipher,
            recv_cipher,
            send_dir,
            recv_dir,
            send_counter: 0,
            recv_counter: 0,
        }
    }

    /// Chiffre une charge utile pour l'envoi et incrémente le compteur d'envoi.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if self.send_counter == u64::MAX {
            return Err(SyncError::NonceExhausted);
        }
        let counter = self.send_counter;
        let nonce = counter_nonce(counter);
        let aad = frame_aad(self.send_dir, counter);
        let ciphertext = self
            .send_cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| SyncError::Encrypt(e.to_string()))?;
        self.send_counter += 1;
        Ok(ciphertext)
    }

    /// Déchiffre et authentifie la prochaine trame attendue ; refuse toute
    /// altération, relecture ou réordonnancement.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if self.recv_counter == u64::MAX {
            return Err(SyncError::NonceExhausted);
        }
        let counter = self.recv_counter;
        let nonce = counter_nonce(counter);
        let aad = frame_aad(self.recv_dir, counter);
        let plaintext = self
            .recv_cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| SyncError::Decrypt)?;
        self.recv_counter += 1;
        Ok(plaintext)
    }

    pub const fn sent_count(&self) -> u64 {
        self.send_counter
    }

    pub const fn received_count(&self) -> u64 {
        self.recv_counter
    }
}

/// Nonce aléatoire de 24 octets échangé pendant le handshake.
pub fn random_nonce() -> Result<[u8; 24]> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|e| SyncError::Random(e.to_string()))?;
    Ok(nonce)
}

/// Dérive le nonce de session (32 octets) à partir des deux nonces aléatoires.
pub fn derive_session_nonce(initiator_nonce: &[u8; 24], responder_nonce: &[u8; 24]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"vtt-sync-session-v1\0");
    hasher.update(initiator_nonce);
    hasher.update(responder_nonce);
    hasher.finalize().into()
}

/// Calcule un tag d'authentification mutuelle (HMAC-SHA256) prouvant la
/// connaissance de `shared_key` sans jamais la transmettre.
pub fn auth_tag(
    shared_key: &SyncKey,
    session_nonce: &[u8; 32],
    role_label: &str,
    device_id: &DeviceId,
) -> [u8; 32] {
    let mut message = Vec::with_capacity(64 + role_label.len() + device_id.as_str().len());
    message.extend_from_slice(b"vtt-sync-auth-v1\0");
    message.extend_from_slice(session_nonce);
    message.extend_from_slice(role_label.as_bytes());
    message.push(0);
    message.extend_from_slice(device_id.as_str().as_bytes());
    hmac_sha256(shared_key.as_bytes(), &message)
}

/// Vérifie un tag d'authentification en temps constant.
pub fn verify_tag(expected: &[u8; 32], actual: &[u8; 32]) -> Result<()> {
    if constant_time_eq(expected, actual) {
        Ok(())
    } else {
        Err(SyncError::AuthenticationFailed)
    }
}

/// Premier message en clair du handshake : annonce l'appareil et son nonce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u8,
    pub device_id: DeviceId,
    pub device_name: String,
    pub pairing: bool,
    pub nonce: [u8; 24],
    /// Présent uniquement pour un appairage direct par clic + code à 6 chiffres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_pairing: Option<super::direct_pairing::DirectPairInit>,
}

/// Réponse du serveur : son identité, son nonce et sa preuve de clé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol: u8,
    pub device_id: DeviceId,
    pub device_name: String,
    pub nonce: [u8; 24],
    pub proof: [u8; 32],
}

/// Confirmation finale de l'initiateur : sa preuve de clé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfirm {
    pub proof: [u8; 32],
}

/// Écrit une trame préfixée par sa longueur (4 octets big-endian).
pub fn encode_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(SyncError::FrameTooLarge {
            size: payload.len(),
        });
    }
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    Ok(())
}

/// Lit une trame préfixée par sa longueur, en refusant toute taille dépassant
/// [`MAX_FRAME_BYTES`].
pub fn decode_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(SyncError::FrameTooLarge { size });
    }
    let mut buf = vec![0u8; size];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

fn derive_subkey(shared_key: &SyncKey, session_nonce: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let mut info = Vec::with_capacity(64 + label.len());
    info.extend_from_slice(b"vtt-sync-subkey-v1\0");
    info.extend_from_slice(session_nonce);
    info.extend_from_slice(label);
    hmac_sha256(shared_key.as_bytes(), &info)
}

fn counter_nonce(counter: u64) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[4..12].copy_from_slice(&counter.to_be_bytes());
    *Nonce::from_slice(&bytes)
}

fn frame_aad(direction: u8, counter: u64) -> [u8; 10] {
    let mut aad = [0u8; 10];
    aad[0] = PROTOCOL_VERSION;
    aad[1] = direction;
    aad[2..10].copy_from_slice(&counter.to_be_bytes());
    aad
}

/// HMAC-SHA256 minimal (RFC 2104) pour éviter une dépendance supplémentaire.
pub(crate) fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..32 {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out: [u8; 32] = outer.finalize().into();

    ipad.zeroize();
    opad.zeroize();
    out
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn key_base64_roundtrip_fingerprint_and_redacted_debug() {
        let key = SyncKey::generate().unwrap();
        let encoded = key.to_base64();
        let decoded = SyncKey::from_base64(&encoded).unwrap();
        assert_eq!(key, decoded);

        let fp = key.fingerprint();
        assert_eq!(fp.len(), 19); // XXXX-XXXX-XXXX-XXXX
        assert_eq!(fp, decoded.fingerprint());

        let dbg = format!("{key:?}");
        assert!(dbg.contains("REDACTED"));
        assert!(dbg.contains(&fp));
        assert!(!dbg.contains(&encoded));

        assert!(SyncKey::from_base64("pas-valide!!!").is_err());
        assert!(SyncKey::from_base64(&URL_SAFE_NO_PAD.encode([1u8; 16])).is_err());
    }

    #[test]
    fn session_cipher_encrypts_bidirectionally_and_rejects_tamper_or_replay() {
        let key = SyncKey::generate().unwrap();
        let nonce_a = random_nonce().unwrap();
        let nonce_b = random_nonce().unwrap();
        let session = derive_session_nonce(&nonce_a, &nonce_b);

        let mut client = SessionCipher::new(&key, &session, SessionRole::Initiator);
        let mut server = SessionCipher::new(&key, &session, SessionRole::Responder);

        // Aller client -> serveur.
        let ct1 = client.encrypt(b"bonjour serveur").unwrap();
        assert_ne!(ct1, b"bonjour serveur");
        assert_eq!(server.decrypt(&ct1).unwrap(), b"bonjour serveur");

        // Retour serveur -> client (sous-clé directionnelle distincte).
        let ct_reply = server.encrypt(b"bonjour client").unwrap();
        assert_eq!(client.decrypt(&ct_reply).unwrap(), b"bonjour client");

        // Relecture de la trame 0 refusée (compteur attendu = 1).
        assert!(matches!(server.decrypt(&ct1), Err(SyncError::Decrypt)));

        // Altération d'un seul bit refusée.
        let mut ct2 = client.encrypt(b"message 2").unwrap();
        ct2[0] ^= 0x01;
        assert!(matches!(server.decrypt(&ct2), Err(SyncError::Decrypt)));

        // Un message envoyé dans un sens ne peut jamais être déchiffré par l'émetteur lui-même.
        let ct3 = client.encrypt(b"sens unique").unwrap();
        assert!(matches!(client.decrypt(&ct3), Err(SyncError::Decrypt)));
    }

    #[test]
    fn mutual_auth_tags_verify_same_key_and_reject_wrong_key_or_role() {
        let key = SyncKey::generate().unwrap();
        let wrong_key = SyncKey::generate().unwrap();
        let session = derive_session_nonce(&[1u8; 24], &[2u8; 24]);
        let dev = DeviceId::parse("0123456789abcdef0123456789abcdef").unwrap();

        let tag = auth_tag(&key, &session, "responder", &dev);
        assert!(verify_tag(&tag, &tag).is_ok());

        let wrong_tag = auth_tag(&wrong_key, &session, "responder", &dev);
        assert!(matches!(
            verify_tag(&tag, &wrong_tag),
            Err(SyncError::AuthenticationFailed)
        ));

        let wrong_role = auth_tag(&key, &session, "initiator", &dev);
        assert!(matches!(
            verify_tag(&tag, &wrong_role),
            Err(SyncError::AuthenticationFailed)
        ));
    }

    #[test]
    fn frame_codec_roundtrips_and_enforces_max_size() {
        let mut buf = Vec::new();
        encode_frame(&mut buf, b"trame 1").unwrap();
        encode_frame(&mut buf, b"").unwrap();

        let mut cursor = Cursor::new(buf);
        assert_eq!(decode_frame(&mut cursor).unwrap(), b"trame 1");
        assert_eq!(decode_frame(&mut cursor).unwrap(), b"");

        // En-tête annonçant plus que MAX_FRAME_BYTES : refusé sans allouer.
        let huge_header = ((MAX_FRAME_BYTES as u32) + 1).to_be_bytes();
        let mut bad = Cursor::new(huge_header);
        assert!(matches!(
            decode_frame(&mut bad),
            Err(SyncError::FrameTooLarge { .. })
        ));
    }
}
