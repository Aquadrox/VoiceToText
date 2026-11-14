//! Capture commentée sous Linux : capture d'écran, clics de la souris,
//! fenêtre active, collage d'une image.
//!
//! Capture d'écran : sous X11, lecture directe de l'écran (GetImage) ; sous
//! Wayland, par le portail du bureau (une application n'a pas le droit de
//! lire l'écran elle-même) : la première fois, KDE demande l'autorisation.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use vtt_core::annotation::Image;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};

use crate::linux::{connect, wayland_session};
use crate::portal;

/// Délai de réponse du portail : la première capture attend que
/// l'utilisateur l'autorise.
const PORTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Capture de l'écran (tous les écrans sous Wayland, l'écran X11 sinon), en
/// RGBA.
pub fn screenshot() -> Result<Image, String> {
    if wayland_session() {
        match screenshot_portal() {
            Ok(image) => return Ok(image),
            Err(e) => return Err(format!("capture d'écran (portail du bureau) : {e}")),
        }
    }
    screenshot_x11()
}

fn screenshot_x11() -> Result<Image, String> {
    let (conn, screen) = connect()?;
    let root = &conn.setup().roots[screen];
    let (width, height) = (root.width_in_pixels, root.height_in_pixels);
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root.root, 0, 0, width, height, !0)
        .map_err(|e| format!("capture d'écran : {e}"))?
        .reply()
        .map_err(|e| format!("capture d'écran : {e}"))?;
    if reply.depth != 24 && reply.depth != 32 {
        return Err(format!("profondeur d'écran {} non gérée", reply.depth));
    }
    let pixels = usize::from(width) * usize::from(height);
    if reply.data.len() < pixels * 4 {
        return Err("capture d'écran incomplète".into());
    }
    // Pixels BGRX (32 bits) → RGBA opaque.
    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in reply.data[..pixels * 4].as_chunks::<4>().0 {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    Ok(Image {
        width: u32::from(width),
        height: u32::from(height),
        rgba,
    })
}

fn screenshot_portal() -> Result<Image, String> {
    let connection = portal::session()?;
    let screenshot = portal::proxy(
        connection,
        portal::DESTINATION,
        portal::PATH,
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|e| e.to_string())?;
    let results = portal::request(connection, PORTAL_TIMEOUT, |token| {
        let options = std::collections::HashMap::from([
            ("handle_token", portal::text(token)),
            ("interactive", zbus::zvariant::Value::from(false)),
        ]);
        screenshot.call_method("Screenshot", &("", options))?;
        Ok(())
    })?;
    let uri: String = results
        .get("uri")
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .ok_or("pas d'image dans la réponse")?;
    let path = portal::file_uri_path(&uri).ok_or("adresse d'image illisible")?;
    let png = std::fs::read(&path).map_err(|e| format!("{} : {e}", path.display()));
    // L'image a été écrite pour nous (dans ~/Images ou un dossier
    // temporaire) : elle ne doit pas s'accumuler, une par seconde.
    let _ = std::fs::remove_file(&path);
    decode_png(&png?)
}

/// PNG (RGB ou RGBA, 8 bits) → RGBA.
fn decode_png(png: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("PNG : {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG trop grand")?];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| format!("PNG : {e}"))?;
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("PNG {other:?} non géré")),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Clics de la souris (bouton gauche ou droit), envoyés en pixels de
/// l'écran tant que l'enregistreur vit (capture commentée). Sous X11, tous
/// les clics ; sous Wayland, ceux que KDE laisse voir aux applications X11.
pub struct ClickRecorder {
    stop: Arc<AtomicBool>,
}

impl ClickRecorder {
    pub fn start(tx: Sender<(i32, i32)>) -> Result<Self, String> {
        let (conn, screen) = connect()?;
        let root = conn.setup().roots[screen].root;
        conn.xinput_xi_query_version(2, 2)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("XInput2 indisponible : {e}"))?;
        conn.xinput_xi_select_events(
            root,
            &[xinput::EventMask {
                deviceid: 1,
                mask: vec![xinput::XIEventMask::RAW_BUTTON_PRESS],
            }],
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("abonnement aux clics : {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        thread::Builder::new()
            .name("mouse-hook".into())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    let event = match conn.poll_for_event() {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        Err(_) => return,
                    };
                    // Boutons 1 (gauche) et 3 (droit) ; la molette (4, 5) non.
                    if let Event::XinputRawButtonPress(raw) = event
                        && matches!(raw.detail, 1 | 3)
                        && let Ok(Ok(pointer)) = conn.query_pointer(root).map(|c| c.reply())
                    {
                        let _ = tx.send((i32::from(pointer.root_x), i32::from(pointer.root_y)));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { stop })
    }
}

impl Drop for ClickRecorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Colle une image dans l'application active (presse-papier + Ctrl+V).
pub fn paste_image(image: &Image) -> Result<(), String> {
    crate::linux::set_clipboard_image(image)?;
    crate::linux::paste_shortcut().map_err(|e| format!("{e} : image laissée dans le presse-papier"))
}

/// Les fenêtres ne peuvent pas être exclues des captures sous Linux : la
/// pastille est masquée pendant la capture commentée (voir l'application).
pub fn exclude_from_capture(_window: isize) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_decoding() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let image = decode_png(&png).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(decode_png(b"pas un png").is_err());
    }
}
