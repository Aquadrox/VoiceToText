//! Secrets dans le Gestionnaire d'identifiants Windows (identifiants
//! génériques), chiffrés par Windows pour la session de l'utilisateur.

use windows::Win32::Foundation::{FILETIME, HLOCAL, LocalFree};
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW,
    CredEnumerateW, CredFree, CredReadW, CredWriteW,
};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::{HSTRING, PWSTR};

/// Préfixe visible dans le Gestionnaire d'identifiants.
const TARGET_PREFIX: &str = "VoiceToText/";

fn target(name: &str) -> HSTRING {
    HSTRING::from(format!("{TARGET_PREFIX}{name}"))
}

/// Chaîne UTF-16 terminée par un zéro, modifiable (CREDENTIALW attend des PWSTR).
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn store_secret(name: &str, value: &str) -> Result<(), String> {
    let mut user = wide("VoiceToText");
    let mut target_buf = wide(&format!("{TARGET_PREFIX}{name}"));
    let mut blob = value.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_buf.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_mut_ptr()),
    };
    unsafe { CredWriteW(&credential, 0) }
        .map_err(|e| format!("enregistrement dans le Gestionnaire d'identifiants : {e}"))
}

pub fn load_secret(name: &str) -> Option<String> {
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe { CredReadW(&target(name), CRED_TYPE_GENERIC, None, &mut credential) }.ok()?;
    // SAFETY: CredReadW a réussi, `credential` pointe vers une structure valide
    // jusqu'à CredFree.
    unsafe {
        let c = &*credential;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let value = String::from_utf8(bytes.to_vec()).ok();
        CredFree(credential as *const _);
        value
    }
}

pub fn delete_secret(name: &str) -> Result<(), String> {
    match unsafe { CredDeleteW(&target(name), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        // Déjà absent : rien à faire.
        Err(_) if load_secret(name).is_none() => Ok(()),
        Err(e) => Err(format!(
            "suppression dans le Gestionnaire d'identifiants : {e}"
        )),
    }
}

/// Noms des secrets enregistrés commençant par `prefix` (sans le préfixe
/// `VoiceToText/`). Seuls les noms sont lus : le contenu des secrets n'est
/// jamais copié.
pub fn list_secrets(prefix: &str) -> Vec<String> {
    let filter = HSTRING::from(format!("{TARGET_PREFIX}{prefix}*"));
    let mut count = 0u32;
    let mut credentials: *mut *mut CREDENTIALW = std::ptr::null_mut();
    // Aucun secret : CredEnumerateW échoue (ERROR_NOT_FOUND).
    if unsafe { CredEnumerateW(&filter, None, &mut count, &mut credentials) }.is_err() {
        return Vec::new();
    }
    // SAFETY: CredEnumerateW a réussi : `credentials` pointe vers `count`
    // structures valides jusqu'à CredFree.
    let names = unsafe {
        std::slice::from_raw_parts(credentials, count as usize)
            .iter()
            .map(|&credential| &*credential)
            .filter(|c| c.Type == CRED_TYPE_GENERIC)
            .filter_map(|c| c.TargetName.to_string().ok())
            .filter_map(|target| target.strip_prefix(TARGET_PREFIX).map(str::to_owned))
            .collect()
    };
    unsafe { CredFree(credentials as *const _) };
    names
}

/// Copie le bloc renvoyé par DPAPI puis le libère.
///
/// SAFETY: `blob` vient de CryptProtectData / CryptUnprotectData réussis.
unsafe fn take_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let data = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(blob.pbData.cast())));
        data
    }
}

/// Chiffre `data` pour la session Windows de l'utilisateur (DPAPI) : seul
/// ce compte, sur ce PC, peut le relire.
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).map_err(|_| "bloc trop grand".to_owned())?,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: entrée valide pendant l'appel (DPAPI ne l'écrit pas), sortie
    // libérée par take_blob.
    unsafe {
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("chiffrement : {e}"))?;
        Ok(take_blob(output))
    }
}

/// Déchiffre un bloc produit par [`protect`] (même compte Windows).
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).map_err(|_| "bloc trop grand".to_owned())?,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: comme pour protect.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("déchiffrement : {e}"))?;
        Ok(take_blob(output))
    }
}
