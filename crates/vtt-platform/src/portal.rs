//! Portails du bureau (xdg-desktop-portal, D-Bus) : les services que KDE,
//! GNOME… offrent aux applications sous Wayland à la place des accès directs
//! d'X11 (capture d'écran, raccourcis globaux). Chaque demande passe par un
//! objet « Request » dont la réponse arrive par un signal ; le bureau peut
//! d'abord demander l'autorisation à l'utilisateur.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

pub const DESTINATION: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";

/// Connexion au bus de la session, partagée.
pub fn session() -> Result<&'static Connection, String> {
    static SESSION: OnceLock<Result<Connection, String>> = OnceLock::new();
    SESSION
        .get_or_init(|| Connection::session().map_err(|e| format!("bus de session : {e}")))
        .as_ref()
        .map_err(Clone::clone)
}

/// Proxy sans cache de propriétés (une lecture, pas d'abonnement).
pub fn proxy(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
) -> zbus::Result<Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(destination.to_owned())?
        .path(path.to_owned())?
        .interface(interface.to_owned())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// Jeton unique pour une demande (le portail en tire le chemin de l'objet
/// Request : il faut s'abonner à sa réponse avant d'envoyer la demande).
pub fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    format!("voicetotext{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Chemin de l'objet Request d'une demande faite avec `token`.
pub fn request_path(connection: &Connection, token: &str) -> Result<String, String> {
    let unique = connection
        .unique_name()
        .ok_or("bus de session sans nom unique")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    Ok(format!("{PATH}/request/{unique}/{token}"))
}

/// Envoie une demande au portail et attend sa réponse : `call` reçoit le
/// jeton à mettre dans les options (`handle_token`). Renvoie les résultats si
/// l'utilisateur a accepté.
pub fn request(
    connection: &'static Connection,
    timeout: Duration,
    call: impl FnOnce(&str) -> zbus::Result<()>,
) -> Result<HashMap<String, OwnedValue>, String> {
    let token = token();
    let path = request_path(connection, &token)?;
    let request =
        proxy(connection, DESTINATION, &path, REQUEST).map_err(|e| format!("portail : {e}"))?;
    let mut responses = request
        .receive_signal("Response")
        .map_err(|e| format!("portail : {e}"))?;
    call(&token).map_err(|e| format!("portail : {e}"))?;

    // L'itérateur de signaux n'a pas de délai : un fil le lit pour nous.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Some(message) = responses.next() {
            let _ = tx.send(
                message
                    .body()
                    .deserialize::<(u32, HashMap<String, OwnedValue>)>()
                    .map_err(|e| format!("réponse du portail : {e}")),
            );
        }
    });
    let (code, results) = rx
        .recv_timeout(timeout)
        .map_err(|_| "le portail n'a pas répondu à temps".to_owned())??;
    match code {
        0 => Ok(results),
        1 => Err("refusé par l'utilisateur".into()),
        _ => Err("refusé par le portail".into()),
    }
}

/// Option de demande : une chaîne.
pub fn text(value: &str) -> Value<'static> {
    Value::from(value.to_owned())
}

/// Chemin local d'une adresse `file://` (caractères %XX décodés).
pub fn file_uri_path(uri: &str) -> Option<std::path::PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(
        decoded,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris() {
        assert_eq!(
            file_uri_path("file:///home/deck/Images/Capture%20d%27%C3%A9cran.png").unwrap(),
            std::path::PathBuf::from("/home/deck/Images/Capture d'écran.png")
        );
        assert_eq!(
            file_uri_path("file:///tmp/a.png").unwrap(),
            std::path::PathBuf::from("/tmp/a.png")
        );
        assert!(file_uri_path("https://exemple.fr/a.png").is_none());
        assert!(file_uri_path("file:///tmp/%zz").is_none());
    }

    #[test]
    fn tokens_are_unique() {
        assert_ne!(token(), token());
    }
}
