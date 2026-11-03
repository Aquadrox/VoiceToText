//! Logique du push-to-talk, indépendante de l'OS.
//!
//! Reçoit les appuis/relâchements bruts du clavier et décide quand démarrer,
//! valider ou annuler un enregistrement. Le raccourci n'est jamais bloqué :
//! sur AZERTY, AltGr sert aussi à taper `@ # €…`, donc toute autre touche
//! pressée pendant l'appui annule la dictée et laisse passer le caractère.
//!
//! Mode mains libres : deux appuis courts rapprochés lancent un
//! enregistrement qui continue après le relâchement ; un nouvel appui le
//! termine, la touche d'annulation (Échap) le jette.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Commencer à enregistrer.
    Start,
    /// Relâché après un appui suffisant : traiter l'enregistrement.
    Stop,
    /// Appui trop court ou combinaison de touches : jeter l'enregistrement.
    Cancel,
    /// Double appui : l'enregistrement en cours continue sans tenir la touche.
    HandsFree,
}

/// Délai maximal entre le relâchement du premier appui et le second.
pub const DOUBLE_TAP_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Recording {
        since_ms: u64,
        /// Second appui d'un double appui : s'il est court, passer en mains libres.
        double_tap: bool,
    },
    /// Enregistrement mains libres : la touche est relâchée.
    HandsFree,
    /// Dictée terminée ou annulée, on attend le relâchement du raccourci.
    Blocked,
}

#[derive(Debug)]
pub struct PushToTalk {
    trigger: u32,
    min_hold_ms: u64,
    /// Touches émises par le système avec le raccourci (faux Ctrl gauche d'AltGr).
    ignored: Vec<u32>,
    /// Touche qui annule un enregistrement mains libres.
    cancel_key: Option<u32>,
    hands_free_enabled: bool,
    /// Fin du dernier appui court (candidat au double appui).
    last_tap_ms: Option<u64>,
    /// Début de l'enregistrement mains libres en cours.
    hands_free_since: Option<u64>,
    state: State,
}

impl PushToTalk {
    /// `trigger` : code de touche propre à la plateforme (virtual-key sous Windows).
    pub fn new(trigger: u32, min_hold_ms: u64) -> Self {
        Self {
            trigger,
            min_hold_ms,
            ignored: Vec::new(),
            cancel_key: None,
            hands_free_enabled: true,
            last_tap_ms: None,
            hands_free_since: None,
            state: State::Idle,
        }
    }

    pub fn with_ignored(mut self, keys: Vec<u32>) -> Self {
        self.ignored = keys;
        self
    }

    pub fn with_hands_free(mut self, enabled: bool, cancel_key: Option<u32>) -> Self {
        self.hands_free_enabled = enabled;
        self.cancel_key = cancel_key;
        self
    }

    pub fn trigger(&self) -> u32 {
        self.trigger
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, State::Recording { .. } | State::HandsFree)
    }

    /// Enregistrement en cours sans que la touche soit tenue.
    pub fn is_hands_free(&self) -> bool {
        self.state == State::HandsFree
    }

    /// Durée d'enregistrement en cours, si un enregistrement est actif.
    pub fn recording_ms(&self, now_ms: u64) -> Option<u64> {
        match self.state {
            State::Recording { since_ms, .. } => Some(now_ms.saturating_sub(since_ms)),
            State::HandsFree => self
                .hands_free_since
                .map(|since| now_ms.saturating_sub(since)),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: u32, down: bool, at_ms: u64) -> Option<Action> {
        if self.ignored.contains(&key) {
            return None;
        }
        match (self.state, key == self.trigger, down) {
            (State::Idle, true, true) => {
                let double_tap = self.hands_free_enabled
                    && self
                        .last_tap_ms
                        .is_some_and(|tap| at_ms.saturating_sub(tap) <= DOUBLE_TAP_MS);
                self.last_tap_ms = None;
                self.state = State::Recording {
                    since_ms: at_ms,
                    double_tap,
                };
                Some(Action::Start)
            }
            (State::Recording { .. }, true, false) => self.release(at_ms),
            (State::Recording { .. }, false, true) => {
                self.state = State::Blocked;
                Some(Action::Cancel)
            }
            (State::HandsFree, true, true) => {
                self.state = State::Blocked;
                self.hands_free_since = None;
                Some(Action::Stop)
            }
            (State::HandsFree, false, true) if Some(key) == self.cancel_key => {
                self.state = State::Idle;
                self.hands_free_since = None;
                Some(Action::Cancel)
            }
            (State::Blocked, true, false) => {
                self.state = State::Idle;
                None
            }
            // Répétition automatique du raccourci, relâchement d'autres touches…
            _ => None,
        }
    }

    /// À appeler quand le relâchement n'a pas été vu (hook privé de
    /// l'événement, par ex. appli administrateur au premier plan) ou pour
    /// couper un enregistrement trop long.
    pub fn force_release(&mut self, at_ms: u64) -> Option<Action> {
        match self.state {
            State::Recording { .. } => self.release(at_ms),
            State::HandsFree => {
                self.state = State::Idle;
                self.hands_free_since = None;
                Some(Action::Stop)
            }
            State::Blocked => {
                self.state = State::Idle;
                None
            }
            State::Idle => None,
        }
    }

    fn release(&mut self, at_ms: u64) -> Option<Action> {
        let State::Recording {
            since_ms,
            double_tap,
        } = self.state
        else {
            return None;
        };
        let short = at_ms.saturating_sub(since_ms) < self.min_hold_ms;
        if short && double_tap {
            self.state = State::HandsFree;
            self.hands_free_since = Some(since_ms);
            return Some(Action::HandsFree);
        }
        self.state = State::Idle;
        if short {
            self.last_tap_ms = Some(at_ms);
            Some(Action::Cancel)
        } else {
            Some(Action::Stop)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALTGR: u32 = 0xA5;
    const KEY_A: u32 = 0x41;

    fn ptt() -> PushToTalk {
        PushToTalk::new(ALTGR, 250)
    }

    #[test]
    fn hold_then_release_records() {
        let mut p = ptt();
        assert_eq!(p.on_key(ALTGR, true, 1000), Some(Action::Start));
        assert!(p.is_recording());
        assert_eq!(p.on_key(ALTGR, true, 1100), None, "répétition ignorée");
        assert_eq!(p.on_key(ALTGR, false, 3000), Some(Action::Stop));
        assert!(!p.is_recording());
    }

    #[test]
    fn short_press_cancels() {
        let mut p = ptt();
        p.on_key(ALTGR, true, 1000);
        assert_eq!(p.on_key(ALTGR, false, 1100), Some(Action::Cancel));
    }

    #[test]
    fn altgr_combination_cancels_until_release() {
        let mut p = ptt();
        p.on_key(ALTGR, true, 1000);
        // AltGr + à → @ : c'est une frappe, pas une dictée.
        assert_eq!(p.on_key(0x30, true, 1300), Some(Action::Cancel));
        assert_eq!(p.on_key(0x30, false, 1350), None);
        assert_eq!(p.on_key(0x30, true, 1400), None);
        assert_eq!(p.on_key(ALTGR, false, 2000), None);
        assert_eq!(p.on_key(ALTGR, true, 3000), Some(Action::Start));
    }

    #[test]
    fn ignored_keys_do_not_cancel() {
        const LCTRL: u32 = 0xA2;
        let mut p = ptt().with_ignored(vec![LCTRL]);
        // Séquence réelle d'AltGr sous Windows : faux Ctrl gauche + Alt droit, répétés.
        assert_eq!(p.on_key(LCTRL, true, 1000), None);
        assert_eq!(p.on_key(ALTGR, true, 1000), Some(Action::Start));
        assert_eq!(p.on_key(LCTRL, true, 1500), None);
        assert_eq!(p.on_key(ALTGR, true, 1500), None);
        assert_eq!(p.on_key(LCTRL, false, 2000), None);
        assert_eq!(p.on_key(ALTGR, false, 2000), Some(Action::Stop));
    }

    #[test]
    fn other_keys_ignored_when_idle() {
        let mut p = ptt();
        assert_eq!(p.on_key(KEY_A, true, 1000), None);
        assert_eq!(p.on_key(KEY_A, false, 1050), None);
        assert!(!p.is_recording());
    }

    #[test]
    fn force_release_stops_recording() {
        let mut p = ptt();
        p.on_key(ALTGR, true, 1000);
        assert_eq!(p.recording_ms(1600), Some(600));
        assert_eq!(p.force_release(2000), Some(Action::Stop));
        assert_eq!(p.force_release(2100), None);
    }

    const ESC: u32 = 0x1B;

    fn hands_free() -> PushToTalk {
        ptt().with_hands_free(true, Some(ESC))
    }

    /// Double appui court : mains libres jusqu'au prochain appui.
    #[test]
    fn double_tap_enters_hands_free_until_next_press() {
        let mut p = hands_free();
        assert_eq!(p.on_key(ALTGR, true, 1000), Some(Action::Start));
        assert_eq!(p.on_key(ALTGR, false, 1080), Some(Action::Cancel));
        assert_eq!(p.on_key(ALTGR, true, 1250), Some(Action::Start));
        assert_eq!(p.on_key(ALTGR, false, 1320), Some(Action::HandsFree));
        assert!(p.is_recording() && p.is_hands_free());
        // Les autres touches ne coupent pas la dictée mains libres.
        assert_eq!(p.on_key(KEY_A, true, 2000), None);
        assert_eq!(p.recording_ms(31_250), Some(30_000));
        assert_eq!(p.on_key(ALTGR, true, 31_250), Some(Action::Stop));
        assert!(!p.is_recording());
        assert_eq!(p.on_key(ALTGR, false, 31_330), None);
        // Prochain appui : push-to-talk normal.
        assert_eq!(p.on_key(ALTGR, true, 40_000), Some(Action::Start));
        assert_eq!(p.on_key(ALTGR, false, 42_000), Some(Action::Stop));
    }

    #[test]
    fn cancel_key_drops_hands_free_recording() {
        let mut p = hands_free();
        p.on_key(ALTGR, true, 1000);
        p.on_key(ALTGR, false, 1080);
        p.on_key(ALTGR, true, 1250);
        p.on_key(ALTGR, false, 1320);
        assert_eq!(p.on_key(ESC, true, 5000), Some(Action::Cancel));
        assert!(!p.is_recording());
        assert_eq!(p.on_key(ESC, false, 5050), None);
        assert_eq!(p.on_key(ALTGR, true, 6000), Some(Action::Start));
    }

    #[test]
    fn slow_second_tap_is_not_a_double_tap() {
        let mut p = hands_free();
        p.on_key(ALTGR, true, 1000);
        p.on_key(ALTGR, false, 1080);
        p.on_key(ALTGR, true, 1080 + DOUBLE_TAP_MS + 1);
        assert_eq!(
            p.on_key(ALTGR, false, 1080 + DOUBLE_TAP_MS + 60),
            Some(Action::Cancel)
        );
        assert!(!p.is_hands_free());
    }

    /// Tap puis appui long : dictée normale, pas de mains libres.
    #[test]
    fn tap_then_hold_is_push_to_talk() {
        let mut p = hands_free();
        p.on_key(ALTGR, true, 1000);
        p.on_key(ALTGR, false, 1080);
        p.on_key(ALTGR, true, 1200);
        assert_eq!(p.on_key(ALTGR, false, 3000), Some(Action::Stop));
        assert!(!p.is_hands_free());
    }

    #[test]
    fn hands_free_can_be_disabled() {
        let mut p = ptt().with_hands_free(false, Some(ESC));
        p.on_key(ALTGR, true, 1000);
        p.on_key(ALTGR, false, 1080);
        p.on_key(ALTGR, true, 1200);
        assert_eq!(p.on_key(ALTGR, false, 1280), Some(Action::Cancel));
    }

    #[test]
    fn force_release_stops_hands_free() {
        let mut p = hands_free();
        p.on_key(ALTGR, true, 1000);
        p.on_key(ALTGR, false, 1080);
        p.on_key(ALTGR, true, 1250);
        p.on_key(ALTGR, false, 1320);
        assert_eq!(p.force_release(600_000), Some(Action::Stop));
        assert!(!p.is_recording());
        assert_eq!(p.on_key(ALTGR, true, 700_000), Some(Action::Start));
    }

    /// Un appui court suivi d'une combinaison AltGr+touche ne compte pas
    /// comme premier tap.
    #[test]
    fn combination_does_not_arm_double_tap() {
        let mut p = hands_free();
        p.on_key(ALTGR, true, 1000);
        p.on_key(0x30, true, 1050);
        p.on_key(ALTGR, false, 1100);
        p.on_key(ALTGR, true, 1200);
        assert_eq!(p.on_key(ALTGR, false, 1260), Some(Action::Cancel));
        assert!(!p.is_hands_free());
    }

    #[test]
    fn force_release_unblocks() {
        let mut p = ptt();
        p.on_key(ALTGR, true, 1000);
        p.on_key(KEY_A, true, 1300);
        assert_eq!(p.force_release(1400), None);
        assert_eq!(p.on_key(ALTGR, true, 1500), Some(Action::Start));
    }
}
