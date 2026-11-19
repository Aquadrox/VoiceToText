//! Canal chiffré de session (`Read + Write`) : handshake d'authentification
//! mutuelle et échange de [`SyncMessage`] chiffrés par ChaCha20-Poly1305.
//!
//! Indépendant du transport : fonctionne sur un `TcpStream` (PC et Android)
//! comme sur un flux en mémoire en test.

use std::io::{Read, Write};

use super::crypto::{
    AuthConfirm, Hello, HelloAck, SessionCipher, SessionRole, SyncKey, auth_tag, decode_frame,
    derive_session_nonce, encode_frame, random_nonce, verify_tag,
};
use super::merge::SyncMessage;
use super::pairing::{DeviceId, LocalIdentity, sanitize_device_name};
use super::{PROTOCOL_VERSION, Result, SyncError};

/// Identité distante authentifiée à l'issue du handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePeerInfo {
    pub device_id: DeviceId,
    pub device_name: String,
    /// `true` si la connexion utilise une offre d'appairage QR éphémère.
    pub pairing: bool,
}

/// Canal bidirectionnel chiffré et authentifié.
pub struct EncryptedChannel<S> {
    stream: S,
    cipher: SessionCipher,
    remote: RemotePeerInfo,
}

impl<S> EncryptedChannel<S> {
    pub const fn remote(&self) -> &RemotePeerInfo {
        &self.remote
    }

    pub const fn cipher(&self) -> &SessionCipher {
        &self.cipher
    }

    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S: Read + Write> EncryptedChannel<S> {
    /// Handshake côté initiateur (l'appareil qui ouvre la connexion).
    ///
    /// Si `expected_remote` est renseigné (pair déjà connu ou QR scanné),
    /// refuse toute réponse émanant d'un autre `DeviceId`.
    pub fn connect(
        mut stream: S,
        local: &LocalIdentity,
        shared_key: &SyncKey,
        expected_remote: Option<&DeviceId>,
        pairing: bool,
    ) -> Result<Self> {
        let nonce_a = random_nonce()?;
        let hello = Hello {
            protocol: PROTOCOL_VERSION,
            device_id: local.device_id.clone(),
            device_name: sanitize_device_name(&local.device_name),
            pairing,
            nonce: nonce_a,
            direct_pairing: None,
        };
        send_plain(&mut stream, &hello)?;

        let ack: HelloAck = recv_plain(&mut stream)?;
        if ack.protocol != PROTOCOL_VERSION {
            return Err(SyncError::UnsupportedVersion(ack.protocol));
        }
        if let Some(expected) = expected_remote
            && &ack.device_id != expected
        {
            return Err(SyncError::AuthenticationFailed);
        }

        let session_nonce = derive_session_nonce(&nonce_a, &ack.nonce);
        let expected_responder_tag =
            auth_tag(shared_key, &session_nonce, "responder", &ack.device_id);
        verify_tag(&expected_responder_tag, &ack.proof)?;

        let initiator_tag = auth_tag(shared_key, &session_nonce, "initiator", &local.device_id);
        send_plain(
            &mut stream,
            &AuthConfirm {
                proof: initiator_tag,
            },
        )?;

        let cipher = SessionCipher::new(shared_key, &session_nonce, SessionRole::Initiator);
        Ok(Self {
            stream,
            cipher,
            remote: RemotePeerInfo {
                device_id: ack.device_id,
                device_name: sanitize_device_name(&ack.device_name),
                pairing,
            },
        })
    }

    /// Handshake côté serveur (l'appareil qui écoute).
    ///
    /// `resolve_key` reçoit le [`Hello`] du client et retourne la clé partagée
    /// attendue (clé permanente du pair déjà enregistré, ou clé éphémère de
    /// l'offre QR active si `hello.pairing == true`).
    pub fn accept<F>(mut stream: S, local: &LocalIdentity, resolve_key: F) -> Result<Self>
    where
        F: FnOnce(&Hello) -> Result<SyncKey>,
    {
        let hello: Hello = recv_plain(&mut stream)?;
        Self::accept_with_hello(stream, hello, local, resolve_key)
    }

    /// Variante de [`Self::accept`] lorsque le [`Hello`] initial a déjà été lu
    /// (permet à `SyncServer` d'aiguiller l'appairage direct avant le handshake).
    pub(crate) fn accept_with_hello<F>(
        mut stream: S,
        hello: Hello,
        local: &LocalIdentity,
        resolve_key: F,
    ) -> Result<Self>
    where
        F: FnOnce(&Hello) -> Result<SyncKey>,
    {
        if hello.protocol != PROTOCOL_VERSION {
            return Err(SyncError::UnsupportedVersion(hello.protocol));
        }
        let shared_key = resolve_key(&hello)?;
        let nonce_b = random_nonce()?;
        let session_nonce = derive_session_nonce(&hello.nonce, &nonce_b);
        let responder_tag = auth_tag(&shared_key, &session_nonce, "responder", &local.device_id);

        let ack = HelloAck {
            protocol: PROTOCOL_VERSION,
            device_id: local.device_id.clone(),
            device_name: sanitize_device_name(&local.device_name),
            nonce: nonce_b,
            proof: responder_tag,
        };
        send_plain(&mut stream, &ack)?;

        let confirm: AuthConfirm = recv_plain(&mut stream)?;
        let expected_initiator_tag =
            auth_tag(&shared_key, &session_nonce, "initiator", &hello.device_id);
        verify_tag(&expected_initiator_tag, &confirm.proof)?;

        let cipher = SessionCipher::new(&shared_key, &session_nonce, SessionRole::Responder);
        Ok(Self {
            stream,
            cipher,
            remote: RemotePeerInfo {
                device_id: hello.device_id,
                device_name: sanitize_device_name(&hello.device_name),
                pairing: hello.pairing,
            },
        })
    }

    /// Sérialise, chiffre et envoie un [`SyncMessage`].
    pub fn send_message(&mut self, msg: &SyncMessage) -> Result<()> {
        let plain = serde_json::to_vec(msg)?;
        let ciphertext = self.cipher.encrypt(&plain)?;
        encode_frame(&mut self.stream, &ciphertext)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Reçoit, déchiffre et désérialise le prochain [`SyncMessage`].
    pub fn recv_message(&mut self) -> Result<SyncMessage> {
        let ciphertext = decode_frame(&mut self.stream)?;
        let plain = self.cipher.decrypt(&ciphertext)?;
        Ok(serde_json::from_slice(&plain)?)
    }
}

pub(crate) fn send_plain<W: Write, T: serde::Serialize>(writer: &mut W, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    encode_frame(writer, &bytes)?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn recv_plain<R: Read, T: serde::de::DeserializeOwned>(reader: &mut R) -> Result<T> {
    let bytes = decode_frame(reader)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use super::*;
    use crate::config::Config;
    use crate::sync::merge::{SyncableConfig, merge_config};
    use crate::sync::pairing::PairingOffer;

    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || TcpStream::connect(addr).unwrap());
        let (server, _) = listener.accept().unwrap();
        let client = handle.join().unwrap();
        (client, server)
    }

    #[test]
    fn qr_pairing_grants_permanent_key_then_syncs_config_encrypted() {
        let pc_id = LocalIdentity::generate("PC Bureau").unwrap();
        let phone_id = LocalIdentity::generate("Xiaomi 17T Pro").unwrap();

        // 1. Le PC génère un QR code éphémère, le téléphone le lit.
        let offer =
            PairingOffer::create(&pc_id, vec!["127.0.0.1:41739".into()], 1000, Some(60_000))
                .unwrap();
        let scanned = PairingOffer::from_uri(&offer.to_uri()).unwrap();
        scanned.check_valid(2000).unwrap();

        let (client_io, server_io) = tcp_pair();
        let pc_id_srv = pc_id.clone();
        let offer_key = offer.key.clone();
        let permanent_key_srv = SyncKey::generate().unwrap();
        let expected_perm = permanent_key_srv.clone();

        let srv = thread::spawn(move || {
            let mut ch = EncryptedChannel::accept(server_io, &pc_id_srv, |hello| {
                assert!(hello.pairing);
                Ok(offer_key.clone())
            })
            .unwrap();
            ch.send_message(&SyncMessage::PairGrant {
                key_b64: permanent_key_srv.to_base64(),
            })
            .unwrap();
            assert!(matches!(ch.recv_message().unwrap(), SyncMessage::PairAck));
            ch.remote().clone()
        });

        let mut phone_ch = EncryptedChannel::connect(
            client_io,
            &phone_id,
            &scanned.key,
            Some(&scanned.device_id),
            true,
        )
        .unwrap();
        let permanent_key_phone = match phone_ch.recv_message().unwrap() {
            SyncMessage::PairGrant { key_b64 } => SyncKey::from_base64(&key_b64).unwrap(),
            other => panic!("attendu PairGrant, reçu {other:?}"),
        };
        phone_ch.send_message(&SyncMessage::PairAck).unwrap();
        let seen_by_pc = srv.join().unwrap();

        assert_eq!(permanent_key_phone, expected_perm);
        assert_eq!(seen_by_pc.device_id, phone_id.device_id);
        assert_eq!(phone_ch.remote().device_id, pc_id.device_id);

        // 2. Session suivante avec la clé permanente : synchronisation chiffrée de la config.
        let (client_io, server_io) = tcp_pair();
        let pc_cfg = Config {
            dictionary: vec!["Aquadrox".into()],
            ..Default::default()
        };
        let phone_cfg = Config {
            dictionary: vec!["Parakeet".into()],
            ..Default::default()
        };

        let pc_id_srv = pc_id.clone();
        let perm_srv = expected_perm.clone();
        let pc_cfg_srv = pc_cfg.clone();
        let srv = thread::spawn(move || {
            let mut ch = EncryptedChannel::accept(server_io, &pc_id_srv, |hello| {
                assert!(!hello.pairing);
                Ok(perm_srv.clone())
            })
            .unwrap();
            let (snap, _) = SyncableConfig::from_config(&pc_cfg_srv);
            ch.send_message(&SyncMessage::ConfigSnapshot {
                updated_at_ms: 5000,
                device_id: pc_id_srv.device_id.clone(),
                payload: snap,
            })
            .unwrap();
            let remote = ch.recv_message().unwrap();
            match remote {
                SyncMessage::ConfigSnapshot {
                    updated_at_ms,
                    device_id,
                    payload,
                } => {
                    merge_config(
                        &pc_cfg_srv,
                        5000,
                        &pc_id_srv.device_id,
                        &payload,
                        updated_at_ms,
                        &device_id,
                    )
                    .unwrap()
                    .0
                }
                other => panic!("attendu ConfigSnapshot, reçu {other:?}"),
            }
        });

        let mut ch = EncryptedChannel::connect(
            client_io,
            &phone_id,
            &permanent_key_phone,
            Some(&pc_id.device_id),
            false,
        )
        .unwrap();
        let (phone_snap, _) = SyncableConfig::from_config(&phone_cfg);
        ch.send_message(&SyncMessage::ConfigSnapshot {
            updated_at_ms: 6000,
            device_id: phone_id.device_id.clone(),
            payload: phone_snap,
        })
        .unwrap();
        let phone_merged = match ch.recv_message().unwrap() {
            SyncMessage::ConfigSnapshot {
                updated_at_ms,
                device_id,
                payload,
            } => {
                merge_config(
                    &phone_cfg,
                    6000,
                    &phone_id.device_id,
                    &payload,
                    updated_at_ms,
                    &device_id,
                )
                .unwrap()
                .0
            }
            other => panic!("attendu ConfigSnapshot, reçu {other:?}"),
        };
        let pc_merged = srv.join().unwrap();

        assert_eq!(pc_merged.dictionary, phone_merged.dictionary);
        assert_eq!(pc_merged.dictionary, vec!["Aquadrox", "Parakeet"]);
    }

    #[test]
    fn handshake_rejects_mismatched_key() {
        let pc_id = LocalIdentity::generate("PC").unwrap();
        let phone_id = LocalIdentity::generate("Phone").unwrap();
        let key_pc = SyncKey::generate().unwrap();
        let key_phone = SyncKey::generate().unwrap();

        let (client_io, server_io) = tcp_pair();
        let pc_id_srv = pc_id.clone();
        let srv =
            thread::spawn(move || EncryptedChannel::accept(server_io, &pc_id_srv, |_| Ok(key_pc)));

        let client_res = EncryptedChannel::connect(
            client_io,
            &phone_id,
            &key_phone,
            Some(&pc_id.device_id),
            false,
        );
        assert!(matches!(client_res, Err(SyncError::AuthenticationFailed)));
        // Le serveur échoue lui aussi (fermeture ou authentification).
        assert!(srv.join().unwrap().is_err());
    }
}
