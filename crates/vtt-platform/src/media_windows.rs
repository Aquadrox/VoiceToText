//! Pause de la musique et des vidéos pendant la dictée, via les contrôles
//! multimédias de Windows (ceux des touches lecture/pause du clavier) :
//! Spotify, navigateurs, lecteurs… Seuls les lecteurs en cours de lecture
//! sont mis en pause, et seuls eux sont relancés ensuite.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

enum Command {
    Pause,
    Resume,
}

/// Pilote un thread dédié : les appels WinRT peuvent prendre quelques
/// dizaines de millisecondes et ne doivent pas bloquer le clavier. Les
/// commandes sont traitées dans l'ordre.
#[derive(Clone)]
pub struct MediaPauser {
    tx: Sender<Command>,
}

impl MediaPauser {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("media".into())
            .spawn(move || run(rx))
            .expect("création du thread multimédia");
        Self { tx }
    }

    /// Met en pause ce qui est en cours de lecture.
    pub fn pause(&self) {
        let _ = self.tx.send(Command::Pause);
    }

    /// Relance ce qui a été mis en pause par [`MediaPauser::pause`].
    pub fn resume(&self) {
        let _ = self.tx.send(Command::Resume);
    }
}

fn run(rx: Receiver<Command>) {
    // SAFETY: initialisation WinRT du thread, une seule fois, avant tout appel.
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        log::warn!("contrôles multimédias indisponibles : {e}");
        return;
    }
    let mut paused: Vec<Session> = Vec::new();
    for command in rx {
        match command {
            Command::Pause => match pause_playing() {
                Ok(sessions) => {
                    if !sessions.is_empty() {
                        log::info!("{} lecteur(s) mis en pause", sessions.len());
                    }
                    paused.extend(sessions);
                }
                Err(e) => log::warn!("pause multimédia : {e}"),
            },
            Command::Resume => {
                for session in paused.drain(..) {
                    if let Err(e) = session.TryPlayAsync().and_then(|op| op.join()) {
                        log::warn!("reprise multimédia : {e}");
                    }
                }
            }
        }
    }
}

fn pause_playing() -> windows::core::Result<Vec<Session>> {
    let manager = Manager::RequestAsync()?.join()?;
    let sessions = manager.GetSessions()?;
    let mut paused = Vec::new();
    for i in 0..sessions.Size()? {
        let session = sessions.GetAt(i)?;
        let playing = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .is_ok_and(|status| status == PlaybackStatus::Playing);
        if playing && session.TryPauseAsync()?.join()? {
            paused.push(session);
        }
    }
    Ok(paused)
}
