//! Pause de la musique et des vidéos pendant la dictée, via MPRIS (D-Bus) :
//! l'interface que les lecteurs Linux exposent pour les touches multimédias
//! et l'applet du bureau (Spotify, navigateurs, VLC, Elisa…). Seuls les
//! lecteurs en cours de lecture sont mis en pause, et seuls eux sont
//! relancés ensuite.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use zbus::blocking::{Connection, Proxy, fdo::DBusProxy};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

enum Command {
    Pause,
    Resume,
}

/// Pilote un thread dédié : les appels D-Bus ne doivent pas bloquer le
/// clavier. Les commandes sont traitées dans l'ordre.
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
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(e) => {
            log::warn!("contrôles multimédias indisponibles (bus de session) : {e}");
            return;
        }
    };
    let mut paused: Vec<String> = Vec::new();
    for command in rx {
        match command {
            Command::Pause => match pause_playing(&connection) {
                Ok(players) => {
                    if !players.is_empty() {
                        log::info!("mis en pause : {}", players.join(", "));
                    }
                    paused.extend(players);
                }
                Err(e) => log::warn!("pause des lecteurs : {e}"),
            },
            Command::Resume => {
                for name in paused.drain(..) {
                    // Lecteur fermé entre-temps : rien à relancer.
                    if let Err(e) = call(&connection, &name, "Play") {
                        log::warn!("reprise de {name} : {e}");
                    }
                }
            }
        }
    }
}

fn player(connection: &Connection, name: &str) -> zbus::Result<Proxy<'static>> {
    // Sans cache des propriétés : une seule lecture, pas d'abonnement aux
    // changements de chaque lecteur.
    zbus::blocking::proxy::Builder::new(connection)
        .destination(name.to_owned())?
        .path(PATH)?
        .interface(PLAYER)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn call(connection: &Connection, name: &str, method: &str) -> zbus::Result<()> {
    player(connection, name)?.call_method(method, &())?;
    Ok(())
}

/// Met en pause les lecteurs qui jouent ; renvoie leurs noms.
fn pause_playing(connection: &Connection) -> zbus::Result<Vec<String>> {
    let names = DBusProxy::new(connection)?.list_names()?;
    let mut paused = Vec::new();
    for name in names
        .iter()
        .map(|n| n.as_str())
        .filter(|n| n.starts_with(PREFIX))
    {
        let playing = player(connection, name)
            .and_then(|p| p.get_property::<String>("PlaybackStatus"))
            .is_ok_and(|status| status == "Playing");
        if playing {
            match call(connection, name, "Pause") {
                Ok(()) => paused.push(name.to_owned()),
                Err(e) => log::warn!("pause de {name} : {e}"),
            }
        }
    }
    Ok(paused)
}
