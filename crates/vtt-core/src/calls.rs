//! Détection d'appel : une application d'appel (Teams, Zoom, Discord…) ou un
//! navigateur utilise le micro depuis un moment → proposer d'enregistrer la
//! réunion, selon le choix de l'utilisateur pour ce service.
//!
//! La couche plateforme fournit la liste des programmes qui utilisent le
//! micro ; tout le reste (seuils, rappels, choix) est ici.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Service d'appel reconnu d'après le nom de son programme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provider {
    /// Clé stable (« teams », « navigateur:chrome »…), gardée dans les réglages.
    pub key: &'static str,
    pub label: &'static str,
    /// Un navigateur tient aussi le micro pour un message vocal ou une
    /// dictée : il faut plus longtemps avant de proposer.
    pub browser: bool,
}

const fn app(key: &'static str, label: &'static str) -> Provider {
    Provider {
        key,
        label,
        browser: false,
    }
}

const fn browser(key: &'static str, label: &'static str) -> Provider {
    Provider {
        key,
        label,
        browser: true,
    }
}

/// Programmes (en minuscules) → service. Exécutables Windows (`.exe`) et
/// Linux (nom du binaire, y compris en Flatpak).
const PROVIDERS: &[(&[&str], Provider)] = &[
    (
        &[
            "ms-teams.exe",
            "teams.exe",
            "msteams.exe",
            "teams-for-linux",
        ],
        app("teams", "Microsoft Teams"),
    ),
    (
        &[
            "zoom.exe",
            "cpthost.exe",
            "zoomwebviewhost.exe",
            "zoom",
            "zoomwebviewhost",
        ],
        app("zoom", "Zoom"),
    ),
    (
        &[
            "discord.exe",
            "discordptb.exe",
            "discordcanary.exe",
            "discord",
            "discordptb",
            "discordcanary",
        ],
        app("discord", "Discord"),
    ),
    (&["slack.exe", "slack"], app("slack", "Slack")),
    (
        &[
            "whatsapp.exe",
            "whatsapp.root.exe",
            "whatsapp-for-linux",
            "zapzap",
        ],
        app("whatsapp", "WhatsApp"),
    ),
    (&["signal.exe", "signal-desktop"], app("signal", "Signal")),
    (
        &["telegram.exe", "telegram-desktop", "telegram"],
        app("telegram", "Telegram"),
    ),
    (&["skype.exe", "skypeforlinux"], app("skype", "Skype")),
    (
        &["ciscocollabhost.exe", "webexhost.exe", "atmgr.exe", "webex"],
        app("webex", "Webex"),
    ),
    (
        &[
            "chrome.exe",
            "chrome",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ],
        browser("navigateur:chrome", "Chrome"),
    ),
    (
        &["msedge.exe", "msedge", "microsoft-edge"],
        browser("navigateur:edge", "Edge"),
    ),
    (
        &["firefox.exe", "firefox", "firefox-bin", "firefox-esr"],
        browser("navigateur:firefox", "Firefox"),
    ),
    (
        &["brave.exe", "brave", "brave-browser"],
        browser("navigateur:brave", "Brave"),
    ),
    (
        &["opera.exe", "opera"],
        browser("navigateur:opera", "Opera"),
    ),
    (
        &["vivaldi.exe", "vivaldi-bin", "vivaldi"],
        browser("navigateur:vivaldi", "Vivaldi"),
    ),
];

/// Service d'un programme qui utilise le micro (`None` : pas un appel).
pub fn provider(exe: &str) -> Option<&'static Provider> {
    let exe = exe.to_lowercase();
    PROVIDERS
        .iter()
        .find(|(exes, _)| exes.contains(&exe.as_str()))
        .map(|(_, provider)| provider)
}

/// Tous les services reconnus, pour les réglages.
pub fn providers() -> impl Iterator<Item = &'static Provider> {
    PROVIDERS.iter().map(|(_, provider)| provider)
}

/// Que faire quand un appel de ce service est détecté.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Proposer d'enregistrer (par défaut).
    #[default]
    Demander,
    /// Enregistrer sans demander.
    Toujours,
    /// Ne rien proposer.
    Jamais,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Offer(&'static Provider),
    Record(&'static Provider),
}

/// Une application d'appel doit tenir le micro 2 s ; un navigateur, 20 s.
const APP_THRESHOLD_MS: u64 = 2_000;
const BROWSER_THRESHOLD_MS: u64 = 20_000;
/// Un appel proposé (ou enregistré) n'est plus proposé tant que le service
/// n'a pas lâché le micro pendant une minute.
const SNOOZE_MS: u64 = 60_000;
/// Deux relevés plus espacés (mise en veille…) : on recommence à compter.
const MAX_GAP_MS: u64 = 10_000;

#[derive(Debug, Default)]
pub struct Detector {
    /// Service sur le micro → début de la période continue.
    runs: HashMap<&'static str, u64>,
    /// Service déjà proposé → dernière fois vu sur le micro.
    snoozed: HashMap<&'static str, u64>,
    last_ms: Option<u64>,
}

impl Detector {
    /// Relevé : programmes qui utilisent le micro à l'instant `now_ms`.
    /// `recording` : une réunion s'enregistre déjà (rien à proposer ; les
    /// services présents sont mis en sourdine pour ne pas reproposer le même
    /// appel après l'arrêt).
    pub fn observe(
        &mut self,
        now_ms: u64,
        exes: &[String],
        recording: bool,
        policy: impl Fn(&str) -> Policy,
    ) -> Option<Decision> {
        if self
            .last_ms
            .is_some_and(|last| now_ms.saturating_sub(last) > MAX_GAP_MS)
        {
            self.runs.clear();
        }
        self.last_ms = Some(now_ms);
        let mut present: Vec<&'static Provider> = Vec::new();
        for found in exes.iter().filter_map(|e| provider(e)) {
            if !present.iter().any(|p| p.key == found.key) {
                present.push(found);
            }
        }
        let on_mic = |key: &str| present.iter().any(|p| p.key == key);

        for provider in &present {
            if let Some(seen) = self.snoozed.get_mut(provider.key) {
                *seen = now_ms;
            }
        }
        self.snoozed
            .retain(|key, seen| on_mic(key) || now_ms.saturating_sub(*seen) < SNOOZE_MS);
        self.runs.retain(|key, _| on_mic(key));

        let mut decision = None;
        for provider in present {
            if recording {
                self.snoozed.insert(provider.key, now_ms);
                continue;
            }
            if self.snoozed.contains_key(provider.key) {
                continue;
            }
            let start = *self.runs.entry(provider.key).or_insert(now_ms);
            let threshold = if provider.browser {
                BROWSER_THRESHOLD_MS
            } else {
                APP_THRESHOLD_MS
            };
            if now_ms.saturating_sub(start) < threshold || decision.is_some() {
                continue;
            }
            self.snoozed.insert(provider.key, now_ms);
            decision = match policy(provider.key) {
                Policy::Jamais => None,
                Policy::Toujours => Some(Decision::Record(provider)),
                Policy::Demander => Some(Decision::Offer(provider)),
            };
        }
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exes(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn ask(_: &str) -> Policy {
        Policy::Demander
    }

    #[test]
    fn providers_are_recognised_case_insensitively() {
        assert_eq!(provider("ms-teams.exe").unwrap().key, "teams");
        assert_eq!(provider("Zoom.exe").unwrap().label, "Zoom");
        assert!(provider("chrome.exe").unwrap().browser);
        assert_eq!(provider("notepad.exe"), None);
        // Linux : nom du binaire.
        assert_eq!(provider("Discord").unwrap().key, "discord");
        assert_eq!(provider("signal-desktop").unwrap().key, "signal");
        assert!(provider("firefox").unwrap().browser);
        assert_eq!(provider("voicetotext"), None);
        let mut keys: Vec<&str> = providers().map(|p| p.key).collect();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn a_call_app_is_offered_after_two_seconds_then_snoozed() {
        let mut detector = Detector::default();
        let teams = exes(&["ms-teams.exe"]);
        assert_eq!(detector.observe(0, &teams, false, ask), None);
        assert_eq!(detector.observe(1_000, &teams, false, ask), None);
        assert_eq!(
            detector.observe(2_000, &teams, false, ask),
            Some(Decision::Offer(provider("teams.exe").unwrap()))
        );
        // Même appel : plus rien, même longtemps après.
        assert_eq!(detector.observe(30_000, &teams, false, ask), None);
        // Micro lâché une minute, puis nouvel appel : de nouveau proposé.
        assert_eq!(detector.observe(31_000, &[], false, ask), None);
        assert_eq!(detector.observe(91_000, &[], false, ask), None);
        assert_eq!(detector.observe(92_000, &teams, false, ask), None);
        assert!(detector.observe(94_000, &teams, false, ask).is_some());
    }

    #[test]
    fn browsers_wait_twenty_seconds_and_short_uses_are_ignored() {
        let mut detector = Detector::default();
        let chrome = exes(&["chrome.exe"]);
        for t in (0..=15_000).step_by(1_000) {
            assert_eq!(detector.observe(t, &chrome, false, ask), None);
        }
        // Message vocal terminé : la période recommence.
        assert_eq!(detector.observe(16_000, &[], false, ask), None);
        for t in (17_000..37_000).step_by(1_000) {
            assert_eq!(detector.observe(t, &chrome, false, ask), None);
        }
        assert!(detector.observe(37_000, &chrome, false, ask).is_some());
    }

    #[test]
    fn policies_record_or_stay_silent() {
        let zoom = exes(&["zoom.exe"]);
        let mut detector = Detector::default();
        let always = |key: &str| {
            if key == "zoom" {
                Policy::Toujours
            } else {
                Policy::Demander
            }
        };
        detector.observe(0, &zoom, false, always);
        assert_eq!(
            detector.observe(2_000, &zoom, false, always),
            Some(Decision::Record(provider("zoom.exe").unwrap()))
        );
        let mut detector = Detector::default();
        detector.observe(0, &zoom, false, |_| Policy::Jamais);
        assert_eq!(
            detector.observe(5_000, &zoom, false, |_| Policy::Jamais),
            None
        );
    }

    #[test]
    fn nothing_is_offered_while_recording_nor_right_after() {
        let mut detector = Detector::default();
        let discord = exes(&["discord.exe"]);
        for t in (0..10_000).step_by(1_000) {
            assert_eq!(detector.observe(t, &discord, true, ask), None);
        }
        // Enregistrement arrêté, l'appel continue : pas de nouvelle proposition.
        assert_eq!(detector.observe(11_000, &discord, false, ask), None);
        assert_eq!(detector.observe(20_000, &discord, false, ask), None);
    }

    #[test]
    fn a_long_gap_restarts_the_count() {
        let mut detector = Detector::default();
        let slack = exes(&["slack.exe"]);
        detector.observe(0, &slack, false, ask);
        // PC en veille : le relevé suivant arrive bien plus tard.
        assert_eq!(detector.observe(60_000, &slack, false, ask), None);
        assert!(detector.observe(62_000, &slack, false, ask).is_some());
    }
}
