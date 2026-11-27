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

    /// Retire un service de la mise en sourdine (après un arrêt automatique de
    /// fin d'appel : un nouvel appel lancé peu après sera aussitôt détecté).
    pub fn unsnooze(&mut self, key: &str) {
        self.snoozed.remove(key);
        self.runs.remove(key);
    }
}

/// Temps de retenue si du son actif est encore détecté sur la piste « Eux »
/// (évite de couper pendant que quelqu'un parle encore si l'application a
/// relâché le micro brièvement).
const SYSTEM_AUDIO_HOLD_MS: u64 = 5_000;

/// Durée minimale d'enregistrement avant qu'un arrêt sur silence prolongé ne
/// puisse se déclencher (laisse le temps aux participants de se dire bonjour).
const MIN_SILENCE_ARM_MS: u64 = 30_000;

/// Raison d'un arrêt automatique d'enregistrement de réunion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Le service d'appel suivi a libéré le micro pendant tout le délai de grâce.
    CallEnded(&'static Provider),
    /// Aucun son (ni micro ni son du PC) pendant ce nombre de minutes.
    Silence(u32),
}

/// État renvoyé par [`EndDetector::observe`] pendant un enregistrement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndStatus {
    /// Enregistrement en cours normalement.
    Active,
    /// Fin d'appel probable : arrêt imminent si l'appel ne reprend pas.
    StoppingIn {
        provider: &'static Provider,
        remaining_ms: u64,
    },
    /// Conditions d'arrêt réunies.
    Stop(StopReason),
}

/// Détecteur de fin d'enregistrement pour une réunion (`MeetingKind::Reunion`).
///
/// Suit l'application d'appel attachée à la réunion (dès le démarrage si elle a
/// déclenché l'enregistrement, ou automatiquement si une application d'appel est
/// détectée sur le micro pendant une réunion lancée à la main). Peut aussi
/// arrêter sur silence complet prolongé si l'utilisateur l'active.
#[derive(Debug)]
pub struct EndDetector {
    tracked: Option<&'static Provider>,
    /// Service aperçu sur le micro en cours de réunion manuelle : (service, début).
    candidate: Option<(&'static Provider, u64)>,
    /// Moment où `tracked` a disparu des applications utilisant le micro.
    absent_since_ms: Option<u64>,
    started_at_ms: u64,
    last_observe_ms: Option<u64>,
}

impl EndDetector {
    pub fn new(started_at_ms: u64, initial_provider: Option<&'static Provider>) -> Self {
        Self {
            tracked: initial_provider,
            candidate: None,
            absent_since_ms: None,
            started_at_ms,
            last_observe_ms: None,
        }
    }

    pub fn tracked(&self) -> Option<&'static Provider> {
        self.tracked
    }

    /// Relevé périodique pendant l'enregistrement.
    ///
    /// - `exes` : programmes utilisant actuellement le micro (`microphone_apps`).
    /// - `last_audio_ms` : dernier instant où une voix (micro ou son du PC) a été captée.
    /// - `last_system_audio_ms` : dernier instant où le son du PC (« Eux ») a été actif.
    /// - `auto_stop_call` : arrêter quand l'application d'appel lâche le micro.
    /// - `call_grace_ms` : délai de grâce continu avant arrêt (ex. 15 000 ms).
    /// - `silence_limit_ms` : durée de silence complet avant arrêt (`None` : désactivé).
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        now_ms: u64,
        exes: &[String],
        last_audio_ms: u64,
        last_system_audio_ms: u64,
        auto_stop_call: bool,
        call_grace_ms: u64,
        silence_limit_ms: Option<u64>,
    ) -> EndStatus {
        if self
            .last_observe_ms
            .is_some_and(|last| now_ms.saturating_sub(last) > MAX_GAP_MS)
        {
            // Mise en veille / reprise : on repart proprement.
            self.absent_since_ms = None;
            self.candidate = None;
        }
        self.last_observe_ms = Some(now_ms);

        let mut present: Vec<&'static Provider> = Vec::new();
        for found in exes.iter().filter_map(|e| provider(e)) {
            if !present.iter().any(|p| p.key == found.key) {
                present.push(found);
            }
        }

        if auto_stop_call {
            if self.tracked.is_none() {
                // Réunion lancée à la main : s'attache automatiquement à l'appel en cours.
                // Priorité aux applications dédiées (Teams, Zoom…) sur les navigateurs.
                let chosen = present
                    .iter()
                    .copied()
                    .find(|p| !p.browser)
                    .or_else(|| present.first().copied());
                match chosen {
                    Some(p) => {
                        let start = match self.candidate {
                            Some((prev, s)) if prev.key == p.key => s,
                            _ => {
                                self.candidate = Some((p, now_ms));
                                now_ms
                            }
                        };
                        let threshold = if p.browser {
                            BROWSER_THRESHOLD_MS
                        } else {
                            APP_THRESHOLD_MS
                        };
                        if now_ms.saturating_sub(start) >= threshold {
                            self.tracked = Some(p);
                            self.candidate = None;
                        }
                    }
                    None => self.candidate = None,
                }
            }

            if let Some(provider) = self.tracked {
                if present.iter().any(|p| p.key == provider.key) {
                    self.absent_since_ms = None;
                } else {
                    let since = *self.absent_since_ms.get_or_insert(now_ms);
                    let absent = now_ms.saturating_sub(since);
                    // Si la piste « Eux » parle encore, on laisse une marge supplémentaire
                    // (plafonnée à 2 × délai pour ne pas bloquer si une vidéo est lancée ensuite).
                    let system_active = last_system_audio_ms >= self.started_at_ms
                        && now_ms.saturating_sub(last_system_audio_ms) < SYSTEM_AUDIO_HOLD_MS;
                    let target = if system_active {
                        call_grace_ms.saturating_mul(2)
                    } else {
                        call_grace_ms
                    };
                    if absent >= target {
                        return EndStatus::Stop(StopReason::CallEnded(provider));
                    }
                    return EndStatus::StoppingIn {
                        provider,
                        remaining_ms: target - absent,
                    };
                }
            }
        } else {
            self.absent_since_ms = None;
        }

        if let Some(limit_ms) = silence_limit_ms
            && limit_ms > 0
            && now_ms.saturating_sub(self.started_at_ms) >= MIN_SILENCE_ARM_MS
        {
            let reference = last_audio_ms.max(self.started_at_ms);
            if now_ms.saturating_sub(reference) >= limit_ms {
                let mins = (limit_ms / 60_000).max(1) as u32;
                return EndStatus::Stop(StopReason::Silence(mins));
            }
        }

        EndStatus::Active
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
        // Après unsnooze (arrêt auto fin d'appel), un nouvel appel est redétecté aussitôt.
        detector.unsnooze("discord");
        assert_eq!(detector.observe(21_000, &discord, false, ask), None);
        assert!(detector.observe(23_000, &discord, false, ask).is_some());
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

    #[test]
    fn end_detector_stops_after_grace_and_cancels_on_reconnect() {
        let teams = provider("ms-teams.exe").unwrap();
        let teams_exes = exes(&["ms-teams.exe"]);
        let mut end = EndDetector::new(0, Some(teams));

        // Appel actif : rien.
        assert_eq!(
            end.observe(5_000, &teams_exes, 5_000, 5_000, true, 15_000, None),
            EndStatus::Active
        );
        // Coupure brève de 5 s puis reconnexion : annulée sans arrêt.
        assert_eq!(
            end.observe(6_000, &[], 5_000, 0, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: teams,
                remaining_ms: 15_000
            }
        );
        assert_eq!(
            end.observe(11_000, &[], 5_000, 0, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: teams,
                remaining_ms: 10_000
            }
        );
        assert_eq!(
            end.observe(12_000, &teams_exes, 12_000, 12_000, true, 15_000, None),
            EndStatus::Active
        );

        // Vraie fin d'appel : 15 s continues sans micro → Stop(CallEnded).
        assert_eq!(
            end.observe(20_000, &[], 12_000, 12_000, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: teams,
                remaining_ms: 15_000
            }
        );
        assert_eq!(
            end.observe(28_000, &[], 12_000, 12_000, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: teams,
                remaining_ms: 7_000
            }
        );
        assert_eq!(
            end.observe(35_000, &[], 12_000, 12_000, true, 15_000, None),
            EndStatus::Stop(StopReason::CallEnded(teams))
        );
    }

    #[test]
    fn end_detector_delays_stop_while_system_audio_is_active_then_caps() {
        let zoom = provider("zoom.exe").unwrap();
        let mut end = EndDetector::new(0, Some(zoom));
        // Zoom relâche le micro à t=10 s mais la piste « Eux » parle encore.
        assert_eq!(
            end.observe(10_000, &[], 10_000, 10_000, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: zoom,
                remaining_ms: 30_000
            }
        );
        // Relevés réguliers (toutes les 5 s < MAX_GAP_MS) : à t=25 s (15 s
        // d'absence), la piste « Eux » parle toujours donc on ne coupe pas encore.
        for t in [15_000, 20_000] {
            assert!(matches!(
                end.observe(t, &[], t - 1_000, t - 1_000, true, 15_000, None),
                EndStatus::StoppingIn { .. }
            ));
        }
        assert_eq!(
            end.observe(25_000, &[], 24_000, 24_000, true, 15_000, None),
            EndStatus::StoppingIn {
                provider: zoom,
                remaining_ms: 15_000
            }
        );
        for t in [30_000, 35_000] {
            assert!(matches!(
                end.observe(t, &[], t - 1_000, t - 1_000, true, 15_000, None),
                EndStatus::StoppingIn { .. }
            ));
        }
        // À t=40 s (2 × 15 s), même si une vidéo tourne sur le PC, on finit par arrêter.
        assert_eq!(
            end.observe(40_000, &[], 39_000, 39_000, true, 15_000, None),
            EndStatus::Stop(StopReason::CallEnded(zoom))
        );
    }

    #[test]
    fn end_detector_attaches_to_call_during_manual_meeting_and_supports_silence_stop() {
        let mut end = EndDetector::new(0, None);
        let slack = exes(&["slack.exe"]);
        // Réunion manuelle : Slack arrive sur le micro pendant 2 s → attaché.
        assert_eq!(
            end.observe(1_000, &slack, 1_000, 1_000, true, 10_000, None),
            EndStatus::Active
        );
        assert_eq!(end.tracked(), None);
        assert_eq!(
            end.observe(3_000, &slack, 3_000, 3_000, true, 10_000, None),
            EndStatus::Active
        );
        assert_eq!(end.tracked().unwrap().key, "slack");
        // Slack raccroche 10 s → arrêt automatique.
        end.observe(4_000, &[], 3_000, 0, true, 10_000, None);
        assert_eq!(
            end.observe(14_000, &[], 3_000, 0, true, 10_000, None),
            EndStatus::Stop(StopReason::CallEnded(provider("slack.exe").unwrap()))
        );

        // Réunion présentielle sans application d'appel : arrêt sur silence de 2 min.
        let mut in_person = EndDetector::new(0, None);
        assert_eq!(
            in_person.observe(20_000, &[], 0, 0, true, 15_000, Some(120_000)),
            EndStatus::Active
        );
        assert_eq!(
            in_person.observe(100_000, &[], 50_000, 0, true, 15_000, Some(120_000)),
            EndStatus::Active
        );
        assert_eq!(
            in_person.observe(170_000, &[], 50_000, 0, true, 15_000, Some(120_000)),
            EndStatus::Stop(StopReason::Silence(2))
        );
    }
}
