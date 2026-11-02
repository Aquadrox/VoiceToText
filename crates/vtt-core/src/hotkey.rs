//! Logique du push-to-talk, indépendante de l'OS.
//!
//! Reçoit les appuis/relâchements bruts du clavier et décide quand démarrer,
//! valider ou annuler un enregistrement. Le raccourci n'est jamais bloqué :
//! sur AZERTY, AltGr sert aussi à taper `@ # €…`, donc toute autre touche
//! pressée pendant l'appui annule la dictée et laisse passer le caractère.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Commencer à enregistrer.
    Start,
    /// Relâché après un appui suffisant : traiter l'enregistrement.
    Stop,
    /// Appui trop court ou combinaison de touches : jeter l'enregistrement.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Recording {
        since_ms: u64,
    },
    /// Dictée annulée, on attend le relâchement du raccourci.
    Blocked,
}

#[derive(Debug)]
pub struct PushToTalk {
    trigger: u32,
    min_hold_ms: u64,
    /// Touches émises par le système avec le raccourci (faux Ctrl gauche d'AltGr).
    ignored: Vec<u32>,
    state: State,
}

impl PushToTalk {
    /// `trigger` : code de touche propre à la plateforme (virtual-key sous Windows).
    pub fn new(trigger: u32, min_hold_ms: u64) -> Self {
        Self {
            trigger,
            min_hold_ms,
            ignored: Vec::new(),
            state: State::Idle,
        }
    }

    pub fn with_ignored(mut self, keys: Vec<u32>) -> Self {
        self.ignored = keys;
        self
    }

    pub fn trigger(&self) -> u32 {
        self.trigger
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, State::Recording { .. })
    }

    /// Durée d'enregistrement en cours, si un enregistrement est actif.
    pub fn recording_ms(&self, now_ms: u64) -> Option<u64> {
        match self.state {
            State::Recording { since_ms } => Some(now_ms.saturating_sub(since_ms)),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: u32, down: bool, at_ms: u64) -> Option<Action> {
        if self.ignored.contains(&key) {
            return None;
        }
        match (self.state, key == self.trigger, down) {
            (State::Idle, true, true) => {
                self.state = State::Recording { since_ms: at_ms };
                Some(Action::Start)
            }
            (State::Recording { .. }, true, false) => self.release(at_ms),
            (State::Recording { .. }, false, true) => {
                self.state = State::Blocked;
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
            State::Blocked => {
                self.state = State::Idle;
                None
            }
            State::Idle => None,
        }
    }

    fn release(&mut self, at_ms: u64) -> Option<Action> {
        let State::Recording { since_ms } = self.state else {
            return None;
        };
        self.state = State::Idle;
        if at_ms.saturating_sub(since_ms) < self.min_hold_ms {
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

    #[test]
    fn force_release_unblocks() {
        let mut p = ptt();
        p.on_key(ALTGR, true, 1000);
        p.on_key(KEY_A, true, 1300);
        assert_eq!(p.force_release(1400), None);
        assert_eq!(p.on_key(ALTGR, true, 1500), Some(Action::Start));
    }
}
