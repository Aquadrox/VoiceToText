//! Appairage direct Wi-Fi par clic sur un appareil détecté + code de
//! vérification court à 6 chiffres (SAS), sans scanner de QR code.
//!
//! Protocole cryptographique (Commit-Before-Reveal + X25519 + SAS) :
//! 1. L'initiateur (A) génère une paire éphémère X25519 `(sk_A, pk_A)` et un
//!    aléa `r_A` (32 octets), puis envoie uniquement un engagement
//!    `commit_A = SHA256("vtt-sync-direct-commit-v1\0" || id_A || pk_A || r_A)`
//!    dans le `Hello` initial (rétrocompatible v1).
//! 2. Le serveur (B) — uniquement si l'appairage direct est explicitement ouvert
//!    (page Synchronisation active) et qu'aucune demande n'est déjà en attente —
//!    génère `(sk_B, pk_B)` et `r_B` et répond avec `DirectPairReply::Challenge`.
//! 3. A révèle `(pk_A, r_A)` (`DirectPairReveal`). B vérifie `commit_A` en temps
//!    constant avant de continuer.
//! 4. Les deux côtés calculent `shared = X25519(sk, pk_distant)` (en rejetant les
//!    points non contributifs), le transcript `T`, un code SAS identique à
//!    6 chiffres, une clé de session ChaCha20-Poly1305 et la clé permanente
//!    candidate `SyncKey`.
//! 5. Les deux écrans affichent le nom de l'autre appareil et le code à 6 chiffres.
//!    Dès que les deux utilisateurs cliquent sur **Appairer (OK)** (dans n'importe
//!    quel ordre, avant l'expiration de 60 s), chaque appareil envoie son tag de
//!    confirmation chiffré et enregistre le pair avec la clé permanente identique.
//!    Tout refus ou expiration coupe aussitôt la session des deux côtés sans rien
//!    enregistrer.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

use super::channel::{recv_plain, send_plain};
use super::crypto::{
    Hello, SessionCipher, SessionRole, SyncKey, auth_tag, constant_time_eq, encode_frame,
    hmac_sha256, random_nonce, verify_tag,
};
use super::pairing::{DeviceId, LocalIdentity, Peer, sanitize_device_name};
use super::session::SyncStorage;
use super::transport::{DEFAULT_IO_TIMEOUT, configure_stream, connect_first, now_ms};
use super::{MAX_FRAME_BYTES, PROTOCOL_VERSION, Result, SyncError};

/// Durée maximale d'attente de confirmation d'un code à 6 chiffres (60 s).
pub const DEFAULT_DIRECT_PAIRING_TTL_MS: u64 = 60 * 1000;

/// Pas d'attente sur le socket pendant la confirmation utilisateur : assure une
/// réaction quasi immédiate au clic local comme au refus distant, sans async.
const DECISION_POLL_INTERVAL: Duration = Duration::from_millis(120);

/// Données d'initiation d'appairage direct embarquées dans [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectPairInit {
    /// Engagement `SHA256("vtt-sync-direct-commit-v1\0" || id_A || pk_A || r_A)`.
    pub commit_a: [u8; 32],
    /// Port TCP d'écoute local de l'initiateur (si son serveur écoute), pour
    /// permettre au répondeur de mémoriser une adresse de synchronisation utile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
}

/// Demande d'appairage direct en attente de validation visuelle du code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDirectPairing {
    /// Identifiant unique local de cette tentative d'appairage.
    pub session_id: String,
    pub remote_device_id: DeviceId,
    pub remote_device_name: String,
    /// Code de vérification à 6 chiffres (ex. `"482917"`).
    pub code: String,
    /// Horodatage d'expiration (ms Unix).
    pub expires_at_ms: u64,
    /// `true` si la demande a été reçue par le serveur, `false` si initiée ici.
    pub incoming: bool,
}

impl PendingDirectPairing {
    /// Code lisible séparé en deux blocs de 3 chiffres (ex. `"482 917"`).
    pub fn formatted_code(&self) -> String {
        format_pairing_code(&self.code)
    }
}

/// Formate un code à 6 chiffres en deux groupes lisibles (`"482917"` → `"482 917"`).
pub fn format_pairing_code(code: &str) -> String {
    let trimmed = code.trim();
    if trimmed.len() == 6 && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        format!("{} {}", &trimmed[..3], &trimmed[3..])
    } else {
        trimmed.to_owned()
    }
}

/// Poignée thread-safe permettant à l'interface d'approuver ou de refuser une
/// demande d'appairage direct en cours (entrante ou sortante).
#[derive(Debug, Clone)]
pub struct DirectDecisionHandle {
    slot: Arc<Mutex<Option<bool>>>,
}

impl DirectDecisionHandle {
    pub(crate) fn new() -> Self {
        Self {
            slot: Arc::new(Mutex::new(None)),
        }
    }

    /// Enregistre la décision de l'utilisateur (`true` = appairer, `false` = refuser).
    /// Seule la première décision est retenue.
    pub fn decide(&self, approve: bool) {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(approve);
        }
    }

    pub fn approve(&self) {
        self.decide(true);
    }

    pub fn reject(&self) {
        self.decide(false);
    }

    fn poll(&self) -> Option<bool> {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Session d'appairage direct initiée localement : le code à 6 chiffres est
/// déjà calculé et prêt à afficher ; appeler [`Self::wait`] sur un thread dédié
/// pendant que l'interface appelle [`DirectDecisionHandle::approve`] ou `reject`.
pub struct DirectPairingHandle {
    stream: TcpStream,
    storage: Arc<dyn SyncStorage>,
    local_id: DeviceId,
    remote_addr: Option<String>,
    pending: PendingDirectPairing,
    decision: DirectDecisionHandle,
    cipher: SessionCipher,
    transcript: [u8; 32],
    permanent_key: SyncKey,
    deadline: Instant,
}

impl DirectPairingHandle {
    pub fn pending(&self) -> &PendingDirectPairing {
        &self.pending
    }

    pub fn decision_handle(&self) -> DirectDecisionHandle {
        self.decision.clone()
    }

    /// Attend la confirmation mutuelle (locale et distante) jusqu'à expiration,
    /// puis enregistre le pair avec la clé permanente partagée.
    pub fn wait(mut self) -> Result<Peer> {
        await_mutual_confirmation(
            &mut self.stream,
            &mut self.cipher,
            &self.decision,
            &self.permanent_key,
            &self.transcript,
            &self.local_id,
            &self.pending.remote_device_id,
            SessionRole::Initiator,
            self.deadline,
        )?;
        let peer = Peer {
            device_id: self.pending.remote_device_id.clone(),
            device_name: self.pending.remote_device_name.clone(),
            key: self.permanent_key.clone(),
            paired_at_ms: now_ms(),
            last_sync_ms: None,
            last_addr: self.remote_addr.clone(),
            sync_settings: true,
            sync_meetings: true,
        };
        self.storage.upsert_peer(peer.clone())?;
        Ok(peer)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DirectPairReply {
    Challenge {
        protocol: u8,
        device_id: DeviceId,
        device_name: String,
        public_key_b: [u8; 32],
        nonce_b: [u8; 32],
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectPairReveal {
    public_key_a: [u8; 32],
    nonce_a: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DirectDecision {
    Approve { proof: [u8; 32] },
    Reject,
}

struct EphemeralKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl EphemeralKeyPair {
    fn generate() -> Result<Self> {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| SyncError::Random(e.to_string()))?;
        let secret = StaticSecret::from(raw);
        raw.zeroize();
        let public = PublicKey::from(&secret);
        Ok(Self { secret, public })
    }
}

fn random_bytes_32() -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| SyncError::Random(e.to_string()))?;
    Ok(bytes)
}

fn random_session_id() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(|e| SyncError::Random(e.to_string()))?;
    let mut hex = String::with_capacity(24);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

/// Calcule l'engagement anti-MITM envoyé avant la révélation de `pk_A` et `r_A`.
pub(crate) fn compute_commitment(
    initiator_id: &DeviceId,
    public_key_a: &[u8; 32],
    nonce_a: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"vtt-sync-direct-commit-v1\0");
    hasher.update(initiator_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(public_key_a);
    hasher.update(nonce_a);
    hasher.finalize().into()
}

pub(crate) fn verify_commitment(
    expected: &[u8; 32],
    initiator_id: &DeviceId,
    public_key_a: &[u8; 32],
    nonce_a: &[u8; 32],
) -> Result<()> {
    let actual = compute_commitment(initiator_id, public_key_a, nonce_a);
    if constant_time_eq(expected, &actual) {
        Ok(())
    } else {
        Err(SyncError::AuthenticationFailed)
    }
}

pub(crate) struct DerivedDirectMaterial {
    pub sas_code: String,
    pub transcript: [u8; 32],
    pub session_key: SyncKey,
    pub permanent_key: SyncKey,
}

/// Dérive de façon symétrique le code SAS à 6 chiffres, le transcript, la clé
/// de session et la clé permanente candidate à partir du secret X25519.
#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_direct_material(
    local_secret: &StaticSecret,
    remote_public_bytes: &[u8; 32],
    initiator_id: &DeviceId,
    responder_id: &DeviceId,
    public_key_a: &[u8; 32],
    public_key_b: &[u8; 32],
    nonce_a: &[u8; 32],
    nonce_b: &[u8; 32],
) -> Result<DerivedDirectMaterial> {
    let remote_public = PublicKey::from(*remote_public_bytes);
    let shared = local_secret.diffie_hellman(&remote_public);
    if !shared.was_contributory() {
        return Err(SyncError::AuthenticationFailed);
    }

    let mut th = Sha256::new();
    th.update(b"vtt-sync-direct-transcript-v1\0");
    th.update(initiator_id.as_str().as_bytes());
    th.update(b"\0");
    th.update(responder_id.as_str().as_bytes());
    th.update(b"\0");
    th.update(public_key_a);
    th.update(public_key_b);
    th.update(nonce_a);
    th.update(nonce_b);
    let transcript: [u8; 32] = th.finalize().into();

    let shared_bytes = shared.as_bytes();

    let mut sas_msg = Vec::with_capacity(64);
    sas_msg.extend_from_slice(b"vtt-sync-direct-sas-v1\0");
    sas_msg.extend_from_slice(&transcript);
    let sas_digest = hmac_sha256(shared_bytes, &sas_msg);
    let sas_num = u32::from_be_bytes([sas_digest[0], sas_digest[1], sas_digest[2], sas_digest[3]])
        % 1_000_000;
    let sas_code = format!("{sas_num:06}");

    let mut sess_msg = Vec::with_capacity(64);
    sess_msg.extend_from_slice(b"vtt-sync-direct-session-key-v1\0");
    sess_msg.extend_from_slice(&transcript);
    let session_key = SyncKey::from_bytes(hmac_sha256(shared_bytes, &sess_msg));

    let mut perm_msg = Vec::with_capacity(64);
    perm_msg.extend_from_slice(b"vtt-sync-direct-permanent-key-v1\0");
    perm_msg.extend_from_slice(&transcript);
    let permanent_key = SyncKey::from_bytes(hmac_sha256(shared_bytes, &perm_msg));

    Ok(DerivedDirectMaterial {
        sas_code,
        transcript,
        session_key,
        permanent_key,
    })
}

/// Lance un appairage direct vers un appareil détecté (sans QR code) :
/// effectue l'échange X25519 + engagement et retourne aussitôt le code à
/// 6 chiffres à afficher sur l'écran local.
pub fn start_direct_pairing_over_tcp(
    storage: Arc<dyn SyncStorage>,
    addrs: &[String],
    expected_remote: Option<&DeviceId>,
    local_listen_port: Option<u16>,
    connect_timeout: Duration,
    ttl_ms: Option<u64>,
) -> Result<DirectPairingHandle> {
    let local = storage.identity()?;
    let (mut stream, connected_addr) = connect_first(addrs, connect_timeout)?;
    configure_stream(&stream, DEFAULT_IO_TIMEOUT);

    let keypair_a = EphemeralKeyPair::generate()?;
    let pk_a = keypair_a.public.to_bytes();
    let nonce_a = random_bytes_32()?;
    let commit_a = compute_commitment(&local.device_id, &pk_a, &nonce_a);

    let hello = Hello {
        protocol: PROTOCOL_VERSION,
        device_id: local.device_id.clone(),
        device_name: sanitize_device_name(&local.device_name),
        pairing: true,
        nonce: random_nonce()?,
        direct_pairing: Some(DirectPairInit {
            commit_a,
            listen_port: local_listen_port,
        }),
    };
    send_plain(&mut stream, &hello)?;

    let reply: DirectPairReply = recv_plain(&mut stream)?;
    let (remote_id, remote_name, pk_b, nonce_b) = match reply {
        DirectPairReply::Challenge {
            protocol,
            device_id,
            device_name,
            public_key_b,
            nonce_b,
        } => {
            if protocol != PROTOCOL_VERSION {
                return Err(SyncError::UnsupportedVersion(protocol));
            }
            if let Some(expected) = expected_remote
                && &device_id != expected
            {
                return Err(SyncError::AuthenticationFailed);
            }
            (
                device_id,
                sanitize_device_name(&device_name),
                public_key_b,
                nonce_b,
            )
        }
        DirectPairReply::Rejected { reason } => {
            return Err(map_rejection_reason(&reason));
        }
    };

    send_plain(
        &mut stream,
        &DirectPairReveal {
            public_key_a: pk_a,
            nonce_a,
        },
    )?;

    let material = derive_direct_material(
        &keypair_a.secret,
        &pk_b,
        &local.device_id,
        &remote_id,
        &pk_a,
        &pk_b,
        &nonce_a,
        &nonce_b,
    )?;

    let ttl = ttl_ms.unwrap_or(DEFAULT_DIRECT_PAIRING_TTL_MS).max(200);
    let pending = PendingDirectPairing {
        session_id: random_session_id()?,
        remote_device_id: remote_id,
        remote_device_name: remote_name,
        code: material.sas_code,
        expires_at_ms: now_ms().saturating_add(ttl),
        incoming: false,
    };

    let cipher = SessionCipher::new(
        &material.session_key,
        &material.transcript,
        SessionRole::Initiator,
    );

    Ok(DirectPairingHandle {
        stream,
        storage,
        local_id: local.device_id,
        remote_addr: Some(connected_addr),
        pending,
        decision: DirectDecisionHandle::new(),
        cipher,
        transcript: material.transcript,
        permanent_key: material.permanent_key,
        deadline: Instant::now() + Duration::from_millis(ttl),
    })
}

/// Réponse immédiate d'un serveur qui refuse une demande d'appairage direct
/// avant même l'échange X25519 (page fermée ou demande déjà en cours).
pub(crate) fn send_direct_rejection(stream: &mut TcpStream, err: &SyncError) {
    let reason = match err {
        SyncError::PairingClosed => "closed".to_owned(),
        SyncError::DirectPairingBusy => "busy".to_owned(),
        other => other.to_string(),
    };
    let _ = send_plain(stream, &DirectPairReply::Rejected { reason });
}

fn map_rejection_reason(reason: &str) -> SyncError {
    match reason {
        "closed" => SyncError::PairingClosed,
        "busy" => SyncError::DirectPairingBusy,
        other => SyncError::Remote(other.to_owned()),
    }
}

/// Traite une connexion entrante d'appairage direct après lecture du [`Hello`].
///
/// `on_pending` enregistre la demande en attente sur le serveur (et notifie
/// l'interface) puis retourne le [`DirectDecisionHandle`] contrôlé par l'UI.
#[allow(clippy::too_many_arguments)]
pub(crate) fn accept_direct_pairing<F>(
    mut stream: TcpStream,
    remote_addr: SocketAddr,
    hello: Hello,
    init: DirectPairInit,
    local: &LocalIdentity,
    storage: &dyn SyncStorage,
    ttl_ms: Option<u64>,
    on_pending: F,
) -> Result<(String, Peer)>
where
    F: FnOnce(PendingDirectPairing) -> Result<DirectDecisionHandle>,
{
    if hello.protocol != PROTOCOL_VERSION {
        return Err(SyncError::UnsupportedVersion(hello.protocol));
    }

    let keypair_b = EphemeralKeyPair::generate()?;
    let pk_b = keypair_b.public.to_bytes();
    let nonce_b = random_bytes_32()?;

    send_plain(
        &mut stream,
        &DirectPairReply::Challenge {
            protocol: PROTOCOL_VERSION,
            device_id: local.device_id.clone(),
            device_name: sanitize_device_name(&local.device_name),
            public_key_b: pk_b,
            nonce_b,
        },
    )?;

    let reveal: DirectPairReveal = recv_plain(&mut stream)?;
    verify_commitment(
        &init.commit_a,
        &hello.device_id,
        &reveal.public_key_a,
        &reveal.nonce_a,
    )?;

    let material = derive_direct_material(
        &keypair_b.secret,
        &reveal.public_key_a,
        &hello.device_id,
        &local.device_id,
        &reveal.public_key_a,
        &pk_b,
        &reveal.nonce_a,
        &nonce_b,
    )?;

    let ttl = ttl_ms.unwrap_or(DEFAULT_DIRECT_PAIRING_TTL_MS).max(200);
    let session_id = random_session_id()?;
    let remote_name = sanitize_device_name(&hello.device_name);
    let pending = PendingDirectPairing {
        session_id: session_id.clone(),
        remote_device_id: hello.device_id.clone(),
        remote_device_name: remote_name.clone(),
        code: material.sas_code,
        expires_at_ms: now_ms().saturating_add(ttl),
        incoming: true,
    };

    let decision = on_pending(pending)?;

    let mut cipher = SessionCipher::new(
        &material.session_key,
        &material.transcript,
        SessionRole::Responder,
    );

    await_mutual_confirmation(
        &mut stream,
        &mut cipher,
        &decision,
        &material.permanent_key,
        &material.transcript,
        &local.device_id,
        &hello.device_id,
        SessionRole::Responder,
        Instant::now() + Duration::from_millis(ttl),
    )?;

    let remembered_addr = init
        .listen_port
        .filter(|&p| p > 0)
        .map(|port| SocketAddr::new(remote_addr.ip(), port).to_string());
    let peer = Peer {
        device_id: hello.device_id,
        device_name: remote_name,
        key: material.permanent_key,
        paired_at_ms: now_ms(),
        last_sync_ms: None,
        last_addr: remembered_addr,
        sync_settings: true,
        sync_meetings: true,
    };
    storage.upsert_peer(peer.clone())?;
    Ok((session_id, peer))
}

#[allow(clippy::too_many_arguments)]
fn await_mutual_confirmation(
    stream: &mut TcpStream,
    cipher: &mut SessionCipher,
    decision: &DirectDecisionHandle,
    permanent_key: &SyncKey,
    transcript: &[u8; 32],
    local_id: &DeviceId,
    remote_id: &DeviceId,
    role: SessionRole,
    deadline: Instant,
) -> Result<()> {
    let (local_label, remote_label) = match role {
        SessionRole::Initiator => ("direct-initiator", "direct-responder"),
        SessionRole::Responder => ("direct-responder", "direct-initiator"),
    };
    let mut local_sent = false;
    let mut remote_approved = false;

    loop {
        if !local_sent && let Some(approve) = decision.poll() {
            if approve {
                let proof = auth_tag(permanent_key, transcript, local_label, local_id);
                send_encrypted_decision(stream, cipher, &DirectDecision::Approve { proof })?;
                local_sent = true;
            } else {
                let _ = send_encrypted_decision(stream, cipher, &DirectDecision::Reject);
                return Err(SyncError::PairingRejected);
            }
        }

        if local_sent && remote_approved {
            return Ok(());
        }

        let now = Instant::now();
        if now >= deadline {
            let _ = send_encrypted_decision(stream, cipher, &DirectDecision::Reject);
            return Err(SyncError::OfferExpired);
        }

        if !remote_approved {
            let step = (deadline - now).min(DECISION_POLL_INTERVAL);
            match recv_encrypted_decision_step(stream, cipher, step)? {
                Some(DirectDecision::Approve { proof }) => {
                    let expected = auth_tag(permanent_key, transcript, remote_label, remote_id);
                    verify_tag(&expected, &proof)?;
                    remote_approved = true;
                }
                Some(DirectDecision::Reject) => {
                    return Err(SyncError::PairingRejected);
                }
                None => {}
            }
        } else {
            // Le distant a déjà approuvé : on attend seulement le clic local.
            let step = (deadline - Instant::now()).min(Duration::from_millis(50));
            if !step.is_zero() {
                std::thread::sleep(step);
            }
        }
    }
}

fn send_encrypted_decision(
    stream: &mut TcpStream,
    cipher: &mut SessionCipher,
    decision: &DirectDecision,
) -> Result<()> {
    use std::io::Write as _;
    let _ = stream.set_write_timeout(Some(DEFAULT_IO_TIMEOUT));
    let plain = serde_json::to_vec(decision)?;
    let ct = cipher.encrypt(&plain)?;
    encode_frame(stream, &ct)?;
    stream.flush()?;
    Ok(())
}

/// Tente de lire la décision chiffrée distante en bornant l'attente du premier
/// octet à `step`. Dès qu'un octet arrive, lit le reste de la trame avec
/// [`DEFAULT_IO_TIMEOUT`] afin de ne jamais corrompre une trame fragmentée.
fn recv_encrypted_decision_step(
    stream: &mut TcpStream,
    cipher: &mut SessionCipher,
    step: Duration,
) -> Result<Option<DirectDecision>> {
    use std::io::Read as _;
    let wait = step.max(Duration::from_millis(10));
    stream.set_read_timeout(Some(wait))?;
    let mut first = [0u8; 1];
    match stream.read(&mut first) {
        Ok(0) => return Err(SyncError::PairingRejected),
        Ok(_) => {}
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            return Ok(None);
        }
        Err(e) => return Err(SyncError::Io(e)),
    }

    stream.set_read_timeout(Some(DEFAULT_IO_TIMEOUT))?;
    let mut rest_header = [0u8; 3];
    stream.read_exact(&mut rest_header)?;
    let size =
        u32::from_be_bytes([first[0], rest_header[0], rest_header[1], rest_header[2]]) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(SyncError::FrameTooLarge { size });
    }
    let mut ct = vec![0u8; size];
    stream.read_exact(&mut ct)?;
    let plain = cipher.decrypt(&ct)?;
    let decision: DirectDecision = serde_json::from_slice(&plain)?;
    Ok(Some(decision))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_and_sas_derivation_are_symmetric_and_tamper_evident() {
        let id_a = DeviceId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let id_b = DeviceId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let kp_a = EphemeralKeyPair::generate().unwrap();
        let kp_b = EphemeralKeyPair::generate().unwrap();
        let pk_a = kp_a.public.to_bytes();
        let pk_b = kp_b.public.to_bytes();
        let nonce_a = random_bytes_32().unwrap();
        let nonce_b = random_bytes_32().unwrap();

        let commit = compute_commitment(&id_a, &pk_a, &nonce_a);
        assert!(verify_commitment(&commit, &id_a, &pk_a, &nonce_a).is_ok());

        let mut bad_nonce = nonce_a;
        bad_nonce[0] ^= 0x01;
        assert!(matches!(
            verify_commitment(&commit, &id_a, &pk_a, &bad_nonce),
            Err(SyncError::AuthenticationFailed)
        ));

        let mat_a = derive_direct_material(
            &kp_a.secret,
            &pk_b,
            &id_a,
            &id_b,
            &pk_a,
            &pk_b,
            &nonce_a,
            &nonce_b,
        )
        .unwrap();
        let mat_b = derive_direct_material(
            &kp_b.secret,
            &pk_a,
            &id_a,
            &id_b,
            &pk_a,
            &pk_b,
            &nonce_a,
            &nonce_b,
        )
        .unwrap();

        assert_eq!(mat_a.sas_code.len(), 6);
        assert!(mat_a.sas_code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(mat_a.sas_code, mat_b.sas_code);
        assert_eq!(mat_a.transcript, mat_b.transcript);
        assert_eq!(mat_a.session_key, mat_b.session_key);
        assert_eq!(mat_a.permanent_key, mat_b.permanent_key);
        assert_ne!(mat_a.session_key, mat_a.permanent_key);

        // Un MITM qui substitue sa propre clé obtient un code/clé différent.
        let kp_m = EphemeralKeyPair::generate().unwrap();
        let pk_m = kp_m.public.to_bytes();
        let mat_mitm = derive_direct_material(
            &kp_a.secret,
            &pk_m,
            &id_a,
            &id_b,
            &pk_a,
            &pk_m,
            &nonce_a,
            &nonce_b,
        )
        .unwrap();
        assert_ne!(mat_a.permanent_key, mat_mitm.permanent_key);
        assert_ne!(mat_a.transcript, mat_mitm.transcript);
    }

    #[test]
    fn rejects_non_contributory_zero_public_key() {
        let id_a = DeviceId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let id_b = DeviceId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let kp_a = EphemeralKeyPair::generate().unwrap();
        let pk_a = kp_a.public.to_bytes();
        let zero_pk = [0u8; 32];
        let nonce_a = [1u8; 32];
        let nonce_b = [2u8; 32];

        let res = derive_direct_material(
            &kp_a.secret,
            &zero_pk,
            &id_a,
            &id_b,
            &pk_a,
            &zero_pk,
            &nonce_a,
            &nonce_b,
        );
        assert!(matches!(res, Err(SyncError::AuthenticationFailed)));
    }

    #[test]
    fn formats_six_digit_code_in_two_groups() {
        assert_eq!(format_pairing_code("482917"), "482 917");
        assert_eq!(format_pairing_code(" 000123 "), "000 123");
        assert_eq!(format_pairing_code("abc"), "abc");
    }
}
