//! Secrets dans le trousseau de la session Linux (KWallet sur le Steam Deck
//! et KDE, GNOME Keyring ailleurs), via l'API Secret Service de D-Bus :
//! l'équivalent du Gestionnaire d'identifiants Windows.
//!
//! [`protect`] / [`unprotect`] remplacent DPAPI : une clé aléatoire de
//! 256 bits, créée au premier usage et gardée dans le trousseau, chiffre les
//! données (ChaCha20-Poly1305, authentifié : un fichier modifié est refusé).

use std::collections::HashMap;
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use secret_service::EncryptionType;
use secret_service::blocking::{Item, SecretService};

/// Attribut commun à tous nos secrets (comme le préfixe `VoiceToText/` du
/// Gestionnaire d'identifiants Windows).
const APPLICATION: &str = "voicetotext";
/// Préfixe du libellé visible dans KWallet ou « Mots de passe et clés ».
const LABEL_PREFIX: &str = "VoiceToText/";
/// Secret qui contient la clé de chiffrement des données ([`protect`]).
const DATA_KEY_SECRET: &str = "data-key";
/// Version du format chiffré : `[1][nonce 12 octets][données + tag 16 octets]`.
const FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;

fn service() -> Result<SecretService<'static>, String> {
    // Dh : le secret est chiffré pendant son passage sur le bus D-Bus.
    SecretService::connect(EncryptionType::Dh).map_err(|e| {
        format!(
            "trousseau de la session introuvable (KWallet ou GNOME Keyring doit être \
             actif) : {e}"
        )
    })
}

fn attributes(name: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", APPLICATION), ("name", name)])
}

/// Déverrouille l'élément si besoin (le trousseau peut demander son mot de
/// passe à l'utilisateur).
fn unlocked(item: &Item) -> Result<(), String> {
    item.ensure_unlocked()
        .map_err(|e| format!("trousseau verrouillé : {e}"))
}

pub fn store_secret(name: &str, value: &str) -> Result<(), String> {
    let service = service()?;
    let collection = service
        .get_any_collection()
        .map_err(|e| format!("trousseau sans portefeuille : {e}"))?;
    collection
        .ensure_unlocked()
        .map_err(|e| format!("trousseau verrouillé : {e}"))?;
    collection
        .create_item(
            &format!("{LABEL_PREFIX}{name}"),
            attributes(name),
            value.as_bytes(),
            // Remplace l'élément aux mêmes attributs.
            true,
            "text/plain",
        )
        .map(|_| ())
        .map_err(|e| format!("enregistrement dans le trousseau : {e}"))
}

pub fn load_secret(name: &str) -> Option<String> {
    let service = service().map_err(|e| log::warn!("{e}")).ok()?;
    let found = service.search_items(attributes(name)).ok()?;
    let item = found.unlocked.first().or(found.locked.first())?;
    unlocked(item).map_err(|e| log::warn!("{e}")).ok()?;
    String::from_utf8(item.get_secret().ok()?).ok()
}

pub fn delete_secret(name: &str) -> Result<(), String> {
    let service = service()?;
    let found = service
        .search_items(attributes(name))
        .map_err(|e| format!("recherche dans le trousseau : {e}"))?;
    // Déjà absent : rien à faire.
    for item in found.unlocked.iter().chain(&found.locked) {
        unlocked(item)?;
        item.delete()
            .map_err(|e| format!("suppression dans le trousseau : {e}"))?;
    }
    Ok(())
}

/// Noms des secrets enregistrés commençant par `prefix`. Seuls les
/// attributs sont lus (sans déverrouiller) : le contenu n'est jamais copié.
pub fn list_secrets(prefix: &str) -> Vec<String> {
    let Ok(service) = service() else {
        return Vec::new();
    };
    let Ok(found) = service.search_items(HashMap::from([("application", APPLICATION)])) else {
        return Vec::new();
    };
    let mut names: Vec<String> = found
        .unlocked
        .iter()
        .chain(&found.locked)
        .filter_map(|item| item.get_attributes().ok()?.remove("name"))
        .filter(|name| name.starts_with(prefix) && name != DATA_KEY_SECRET)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Clé gardée en mémoire après la première lecture : le trousseau n'est
/// interrogé qu'une fois par lancement (chaque bloc audio d'une réunion est
/// chiffré à part).
static DATA_KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);

fn data_key(create: bool) -> Result<[u8; 32], String> {
    let mut cached = DATA_KEY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = *cached {
        return Ok(key);
    }
    let key = match load_secret(DATA_KEY_SECRET) {
        Some(hex) => decode_hex(&hex).ok_or("clé de chiffrement illisible dans le trousseau")?,
        None if create => {
            let key: [u8; 32] = ChaCha20Poly1305::generate_key(&mut OsRng).into();
            store_secret(DATA_KEY_SECRET, &encode_hex(&key))?;
            key
        }
        None => return Err("clé de chiffrement absente du trousseau".into()),
    };
    *cached = Some(key);
    Ok(key)
}

/// Chiffre `data` avec la clé de l'utilisateur gardée dans son trousseau :
/// seul ce compte, sur cet ordinateur, peut le relire.
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    seal(&data_key(true)?, data)
}

/// Déchiffre un bloc produit par [`protect`].
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    open(&data_key(false)?, data)
}

fn seal(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(&nonce, data)
        .map_err(|_| "chiffrement impossible".to_owned())?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn open(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    let (&version, rest) = data.split_first().ok_or("bloc chiffré vide")?;
    if version != FORMAT_VERSION || rest.len() < NONCE_LEN {
        return Err("bloc chiffré inconnu".into());
    }
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), sealed)
        .map_err(|_| "déchiffrement : clé différente ou données abîmées".to_owned())
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim().as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_then_open() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"bonjour").unwrap();
        assert_eq!(sealed[0], FORMAT_VERSION);
        assert_eq!(sealed.len(), 1 + NONCE_LEN + 7 + 16);
        assert_eq!(open(&key, &sealed).unwrap(), b"bonjour");
        // Nonce aléatoire : deux chiffrements diffèrent.
        assert_ne!(seal(&key, b"bonjour").unwrap(), sealed);
    }

    #[test]
    fn open_rejects_wrong_key_or_tampering() {
        let sealed = seal(&[1u8; 32], b"secret").unwrap();
        assert!(open(&[2u8; 32], &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&[1u8; 32], &tampered).is_err());
        assert!(open(&[1u8; 32], &[]).is_err());
        assert!(open(&[1u8; 32], &[9, 0, 0]).is_err());
    }

    #[test]
    fn hex_round_trip() {
        let key: [u8; 32] = std::array::from_fn(|i| (i * 9) as u8);
        assert_eq!(decode_hex(&encode_hex(&key)), Some(key));
        assert_eq!(decode_hex("zz"), None);
        assert_eq!(decode_hex(&"g".repeat(64)), None);
    }
}
