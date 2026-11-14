//! Programmes qui utilisent un micro en ce moment (flux d'enregistrement de
//! PipeWire ou PulseAudio), pour la détection d'appel.
//!
//! Seuls les noms des programmes sont lus : jamais l'audio, jamais le titre
//! des fenêtres. Interrogé chaque seconde par `pactl` (client PulseAudio,
//! présent sur SteamOS et la plupart des bureaux, y compris avec PipeWire),
//! sinon par `pw-dump` (PipeWire).

use std::process::Command;
use std::sync::Once;

use serde_json::Value;

/// Noms (minuscules) des programmes qui enregistrent un micro en ce moment,
/// hors VoiceToText lui-même.
pub fn microphone_apps() -> Vec<String> {
    let own = std::process::id();
    let found = run("pactl", &["-f", "json", "list", "source-outputs"])
        .map(|json| from_pactl(&json, own))
        .or_else(|pactl| {
            run("pw-dump", &[])
                .map(|json| from_pw_dump(&json, own))
                .map_err(|pw| format!("{pactl} ; {pw}"))
        });
    match found {
        Ok(mut apps) => {
            apps.sort();
            apps.dedup();
            apps
        }
        Err(e) => {
            static WARNED: Once = Once::new();
            WARNED.call_once(|| log::warn!("détection des appels indisponible : {e}"));
            Vec::new()
        }
    }
}

/// Lance un outil du système et renvoie sa sortie JSON. Les variables d'une
/// AppImage qui pourraient lui faire charger nos bibliothèques sont retirées.
fn run(program: &str, args: &[&str]) -> Result<Value, String> {
    let output = Command::new(program)
        .args(args)
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
        .map_err(|e| format!("{program} : {e}"))?;
    if !output.status.success() {
        return Err(format!("{program} : code {}", output.status));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("{program} : {e}"))
}

/// Programme d'un flux d'après ses propriétés, s'il n'est pas le nôtre.
fn program(props: &Value, own: u32) -> Option<String> {
    let pid = props.get("application.process.id").and_then(|p| match p {
        Value::String(s) => s.parse::<u32>().ok(),
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    });
    if pid == Some(own) {
        return None;
    }
    let name = props
        .get("application.process.binary")
        .or_else(|| props.get("application.name"))?
        .as_str()?
        .trim();
    (!name.is_empty()).then(|| name.to_lowercase())
}

/// `pactl -f json list source-outputs` : un objet par flux d'enregistrement
/// (« corked » : mis en sourdine par l'application, ignoré).
fn from_pactl(json: &Value, own: u32) -> Vec<String> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter(|stream| stream.get("corked") != Some(&Value::Bool(true)))
        .filter_map(|stream| program(stream.get("properties")?, own))
        .collect()
}

/// `pw-dump` : nœuds « Stream/Input/Audio » en cours.
fn from_pw_dump(json: &Value, own: u32) -> Vec<String> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|object| object.get("info"))
        .filter(|info| info.get("state").and_then(Value::as_str) == Some("running"))
        .filter_map(|info| info.get("props"))
        .filter(|props| {
            props.get("media.class").and_then(Value::as_str) == Some("Stream/Input/Audio")
        })
        .filter_map(|props| program(props, own))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pactl_streams() {
        let json = serde_json::json!([
            {"corked": false, "properties": {"application.process.binary": "Discord", "application.process.id": "42"}},
            {"corked": true, "properties": {"application.process.binary": "zoom", "application.process.id": "43"}},
            {"corked": false, "properties": {"application.process.binary": "voicetotext", "application.process.id": "7"}},
            {"corked": false, "properties": {"application.name": "Firefox"}},
            {"corked": false, "properties": {}}
        ]);
        assert_eq!(from_pactl(&json, 7), ["discord", "firefox"]);
    }

    #[test]
    fn pw_dump_nodes() {
        let json = serde_json::json!([
            {"type": "PipeWire:Interface:Node", "info": {"state": "running", "props": {
                "media.class": "Stream/Input/Audio", "application.process.binary": "signal-desktop",
                "application.process.id": 99}}},
            {"type": "PipeWire:Interface:Node", "info": {"state": "idle", "props": {
                "media.class": "Stream/Input/Audio", "application.process.binary": "zoom"}}},
            {"type": "PipeWire:Interface:Node", "info": {"state": "running", "props": {
                "media.class": "Stream/Output/Audio", "application.process.binary": "spotify"}}},
            {"type": "PipeWire:Interface:Client", "info": {"props": {}}}
        ]);
        assert_eq!(from_pw_dump(&json, 1), ["signal-desktop"]);
    }
}
