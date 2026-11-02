//! Secrets dans le Gestionnaire d'identifiants Windows (identifiants
//! génériques), chiffrés par Windows pour la session de l'utilisateur.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
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
