//! Sons de début et de fin de dictée.

use std::sync::OnceLock;

use vtt_core::audio::tone_wav;

/// Assez discret pour ne pas gêner, et pour ne pas perturber Whisper quand
/// le micro capte le son de début.
const VOLUME: f32 = 0.15;

#[derive(Debug, Clone, Copy)]
pub enum Sound {
    /// Deux notes montantes.
    Start,
    /// Deux notes descendantes.
    Stop,
    /// Une note grave.
    Cancel,
}

pub fn play(sound: Sound) {
    static SOUNDS: OnceLock<[Vec<u8>; 3]> = OnceLock::new();
    let [start, stop, cancel] = SOUNDS.get_or_init(|| {
        [
            tone_wav(&[(660.0, 60), (880.0, 80)], VOLUME),
            tone_wav(&[(880.0, 60), (660.0, 80)], VOLUME),
            tone_wav(&[(330.0, 140)], VOLUME),
        ]
    });
    let wav = match sound {
        Sound::Start => start,
        Sound::Stop => stop,
        Sound::Cancel => cancel,
    };
    vtt_platform::play_sound(wav.as_slice());
}
