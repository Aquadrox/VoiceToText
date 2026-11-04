//! Lecture d'un fichier audio ou vidéo (MP3, M4A/AAC, MP4, MKV/WebM, WAV,
//! FLAC, OGG) en 16 kHz mono, au fil de l'eau : un enregistrement de deux
//! heures n'est jamais chargé en entier.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use vtt_core::{SAMPLE_RATE, audio};

/// Extensions proposées dans le sélecteur de fichier.
pub const EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "aac", "mp4", "mov", "mkv", "webm", "wav", "flac", "ogg", "oga", "opus",
];

/// Audio converti par blocs d'environ une seconde avant rééchantillonnage.
const BLOCK_SECONDS: usize = 1;

/// Décode `path` ; `on_samples` reçoit l'audio en 16 kHz mono par blocs et
/// renvoie `false` pour arrêter. `progress` reçoit le pourcentage lu.
/// Renvoie la durée décodée (ms).
pub fn decode(
    path: &Path,
    mut on_samples: impl FnMut(&[f32]) -> bool,
    mut progress: impl FnMut(u8),
) -> Result<u64, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("format non reconnu : {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or("aucune piste audio dans ce fichier")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(SAMPLE_RATE);
    let total_frames = track.codec_params.n_frames;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("codec non pris en charge : {e}"))?;

    let mut pending: Vec<f32> = Vec::new();
    let mut decoded_frames: u64 = 0;
    let mut emitted: u64 = 0;
    let mut last_percent = u8::MAX;
    let block = rate as usize * BLOCK_SECONDS;
    let mut flush = |pending: &mut Vec<f32>, emitted: &mut u64| -> bool {
        let converted = audio::resample(pending, rate, SAMPLE_RATE);
        pending.clear();
        *emitted += converted.len() as u64;
        on_samples(&converted)
    };
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("lecture : {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // Paquet abîmé : on passe au suivant.
            Err(SymphoniaError::DecodeError(e)) => {
                log::debug!("paquet ignoré : {e}");
                continue;
            }
            Err(e) => return Err(format!("décodage : {e}")),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let mono = audio::downmix(buffer.samples(), channels);
        decoded_frames += mono.len() as u64;
        pending.extend(mono);
        if pending.len() >= block && !flush(&mut pending, &mut emitted) {
            return Ok(emitted * 1000 / u64::from(SAMPLE_RATE));
        }
        if let Some(total) = total_frames.filter(|&t| t > 0) {
            let percent = (decoded_frames * 100 / total).min(100) as u8;
            if percent != last_percent {
                last_percent = percent;
                progress(percent);
            }
        }
    }
    if !pending.is_empty() {
        flush(&mut pending, &mut emitted);
    }
    progress(100);
    Ok(emitted * 1000 / u64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_files_are_decoded_to_16k_mono_blocks() {
        let dir = std::env::temp_dir().join(format!("vtt-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.wav");
        // 2,5 s de 16 kHz mono : la conversion ne change pas la durée.
        let samples: Vec<f32> = (0..40_000)
            .map(|i| ((i % 100) as f32 / 100.0) - 0.5)
            .collect();
        std::fs::write(&path, vtt_core::meeting::wav_bytes(&samples)).unwrap();
        let mut received = 0;
        let mut blocks = 0;
        let mut last = 0;
        let duration = decode(
            &path,
            |block| {
                received += block.len();
                blocks += 1;
                true
            },
            |percent| last = percent,
        )
        .unwrap();
        assert_eq!(received, 40_000);
        assert_eq!(duration, 2_500);
        assert!(blocks >= 2, "par blocs : {blocks}");
        assert_eq!(last, 100);
        assert!(decode(&dir.join("absent.mp3"), |_| true, |_| {}).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Fichiers réels (MP3, M4A…) : `VTT_DECODE_FILES=a.mp3,b.m4a cargo test
    /// -p voicetotext -- --ignored real_files`.
    #[test]
    #[ignore]
    fn real_files_decode() {
        let files = std::env::var("VTT_DECODE_FILES").unwrap_or_default();
        for file in files.split(',').filter(|f| !f.is_empty()) {
            let mut samples = 0usize;
            let duration = decode(
                Path::new(file),
                |b| {
                    samples += b.len();
                    true
                },
                |_| {},
            )
            .unwrap();
            println!("{file} : {duration} ms, {samples} échantillons");
            assert!(duration > 1_000);
        }
    }
}
