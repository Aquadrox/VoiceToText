//! Capture commentée : un raccourci démarre l'enregistrement (micro +
//! captures de l'écran quand il change) ; le tenir enfoncé permet de dessiner
//! à l'écran avec la souris. À l'arrêt : narration transcrite mot par mot,
//! images aux moments clés (traits, « ici », « là »…) annotées et réunies en
//! une planche, texte mis en forme par le LLM, puis planche et texte collés
//! dans l'application active.
//!
//! Les captures restent en mémoire ; elles ne vont sur le disque que si
//! l'utilisateur choisit de les garder.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vtt_core::annotation::{self, Click, Image, Region, Stroke};
use vtt_core::config::CAPTURE_RAW_TEMPLATE;
use vtt_core::{AudioClip, TextSink, Word};
use vtt_llm::CAPTURE_TEMPLATES;
use vtt_platform::capture::Capture;
use vtt_platform::{ClickRecorder, ClipboardPaster};

use crate::controller::{Work, WorkerHandle};
use crate::costs::{self, LedgerStore};
use crate::raw_paste;
use crate::settings::{self, SharedConfig};
use crate::status::{Status, StatusReporter};

/// Écran capturé à ce rythme, gardé seulement s'il a changé.
const FRAME_EVERY: Duration = Duration::from_millis(700);
/// Au plus ce nombre d'écrans en mémoire (les plus anciens sont oubliés).
const MAX_FRAMES: usize = 300;
/// Au-delà, la capture s'arrête d'elle-même.
const MAX_DURATION: Duration = Duration::from_secs(300);
/// Tenir le raccourci plus longtemps que ça : dessin.
const HOLD_TO_DRAW: Duration = Duration::from_millis(250);
/// Délai entre le collage de la planche et celui du texte.
const PASTE_GAP: Duration = Duration::from_millis(400);
const INK_LABEL: &str = "ink";
/// Images envoyées au LLM (description) : au plus cette largeur.
const VISION_WIDTH: u32 = 1280;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Écran capturé, compressé en PNG pour tenir en mémoire.
struct Frame {
    at_ms: u64,
    png: Vec<u8>,
}

fn encode_png(image: &Image) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&image.rgba)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

fn decode_png(png: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut rgba = vec![0; reader.output_buffer_size().ok_or("image trop grande")?];
    let info = reader.next_frame(&mut rgba).map_err(|e| e.to_string())?;
    rgba.truncate(info.buffer_size());
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Empreinte grossière d'un écran (un pixel sur 97) : a-t-il changé ?
fn fingerprint(image: &Image) -> u64 {
    image
        .rgba
        .iter()
        .step_by(97 * 4)
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
}

/// Capture en cours.
struct Recording {
    capture: Capture,
    started: Instant,
    frames: Arc<Mutex<Vec<Frame>>>,
    strokes: Arc<Mutex<Vec<Stroke>>>,
    stop_frames: Arc<AtomicBool>,
    /// Pixels physiques par pixel CSS de la fenêtre d'encre.
    scale: f64,
    /// Fenêtre ou zone capturée (`None` : tout l'écran principal).
    region: Option<Region>,
    /// Zone en cours de tracé : clics ignorés jusqu'à la fin.
    choosing_zone: bool,
    clicks: Arc<Mutex<Vec<Click>>>,
    /// Hook souris, retiré à l'arrêt.
    _click_recorder: Option<ClickRecorder>,
}

#[derive(Default)]
struct Keys {
    /// Le relâchement du raccourci qui a démarré la capture : ignoré.
    ignore_release: bool,
    /// Moment de l'appui en cours.
    pressed: Option<Instant>,
    drawing: bool,
}

/// Dernière capture produite (collable de nouveau).
#[derive(Clone)]
struct Produced {
    sheet: Option<Image>,
    text: String,
}

pub struct Annotator {
    app: AppHandle,
    worker: WorkerHandle,
    dir: PathBuf,
    recording: Mutex<Option<Recording>>,
    processing: AtomicBool,
    keys: Mutex<Keys>,
    last: Mutex<Option<Produced>>,
}

pub type AnnotatorState = Arc<Annotator>;

/// État affiché par la pastille.
#[derive(Clone, Serialize)]
pub struct AnnotateState {
    recording: bool,
    drawing: bool,
    /// Zone à tracer (fenêtre d'encre).
    choosing_zone: bool,
    elapsed_ms: u64,
    shortcut: String,
}

impl Annotator {
    pub fn new(app: AppHandle, captures_dir: PathBuf, worker: WorkerHandle) -> AnnotatorState {
        Arc::new(Self {
            app,
            worker,
            dir: captures_dir,
            recording: Mutex::new(None),
            processing: AtomicBool::new(false),
            keys: Mutex::new(Keys::default()),
            last: Mutex::new(None),
        })
    }

    fn config(&self) -> vtt_core::Config {
        settings::read(&self.app.state::<SharedConfig>()).clone()
    }

    pub fn state(&self) -> AnnotateState {
        let recording = lock(&self.recording);
        AnnotateState {
            recording: recording.is_some(),
            drawing: lock(&self.keys).drawing,
            choosing_zone: recording.as_ref().is_some_and(|r| r.choosing_zone),
            elapsed_ms: recording
                .as_ref()
                .map_or(0, |r| r.started.elapsed().as_millis() as u64),
            shortcut: self.config().hotkey.annotate,
        }
    }

    fn emit_state(&self) {
        let _ = self.app.emit("annotate", self.state());
    }

    fn reporter(&self) -> Arc<StatusReporter> {
        Arc::clone(&self.app.state::<Arc<StatusReporter>>())
    }

    // ---------- Raccourci : appui, maintien, relâchement ----------

    fn on_press(self: &Arc<Self>) {
        if self.processing.load(Ordering::SeqCst) {
            return;
        }
        if lock(&self.recording).is_none() {
            lock(&self.keys).ignore_release = true;
            if let Err(e) = self.start() {
                self.reporter()
                    .set(Status::Error(format!("capture commentée : {e}")));
            }
            return;
        }
        let pressed = Instant::now();
        lock(&self.keys).pressed = Some(pressed);
        let annotator = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(HOLD_TO_DRAW);
            let still_held = lock(&annotator.keys).pressed == Some(pressed);
            if still_held {
                annotator.set_drawing(true);
            }
        });
    }

    fn on_release(self: &Arc<Self>) {
        let (drawing, tapped) = {
            let mut keys = lock(&self.keys);
            if std::mem::take(&mut keys.ignore_release) {
                return;
            }
            (keys.drawing, keys.pressed.take().is_some())
        };
        if drawing {
            self.set_drawing(false);
        } else if tapped {
            self.stop();
        }
    }

    fn set_drawing(&self, drawing: bool) {
        lock(&self.keys).drawing = drawing;
        if let Some(ink) = self.app.get_webview_window(INK_LABEL) {
            // Dessin : la fenêtre d'encre reçoit la souris ; sinon les clics
            // passent à travers.
            let _ = ink.set_ignore_cursor_events(!drawing);
            let _ = self.app.emit_to(INK_LABEL, "ink-draw", drawing);
        }
        self.emit_state();
    }

    // ---------- Enregistrement ----------

    fn start(self: &Arc<Self>) -> Result<(), String> {
        let mut recording = lock(&self.recording);
        if recording.is_some() {
            return Ok(());
        }
        let config = self.config();
        let device = Some(config.audio.device.trim()).filter(|d| !d.is_empty());
        // Vérifie tout de suite que la capture d'écran fonctionne.
        let first = vtt_platform::screenshot()?;
        let capture = Capture::start(device, false)?;
        let zone = config.captures.area == "zone";
        // Fenêtre active au démarrage (la pastille ne prend pas le focus).
        let region = (config.captures.area == "fenetre")
            .then(|| window_region(first.width, first.height))
            .flatten();
        let scale = self.open_ink(zone);
        let started = Instant::now();
        let clicks: Arc<Mutex<Vec<Click>>> = Arc::default();
        let click_recorder = config
            .captures
            .clicks
            .then(|| self.record_clicks(started, Arc::clone(&clicks)))
            .flatten();
        let frames = Arc::new(Mutex::new(vec![Frame {
            at_ms: 0,
            png: encode_png(&first)?,
        }]));
        let stop_frames = Arc::new(AtomicBool::new(false));
        spawn_frames(
            Arc::clone(&frames),
            Arc::clone(&stop_frames),
            started,
            fingerprint(&first),
        );
        *recording = Some(Recording {
            capture,
            started,
            frames,
            strokes: Arc::default(),
            stop_frames,
            scale,
            region,
            choosing_zone: zone,
            clicks,
            _click_recorder: click_recorder,
        });
        drop(recording);
        log::info!("capture commentée : enregistrement");
        // Arrêt automatique au-delà de la durée maximale.
        let annotator = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(MAX_DURATION);
            let same = lock(&annotator.recording)
                .as_ref()
                .is_some_and(|r| r.started == started);
            if same {
                log::info!("capture commentée : durée maximale atteinte");
                annotator.stop();
            }
        });
        self.emit_state();
        Ok(())
    }

    /// Clics de la souris pendant la capture (hors tracé de la zone).
    fn record_clicks(
        self: &Arc<Self>,
        started: Instant,
        clicks: Arc<Mutex<Vec<Click>>>,
    ) -> Option<ClickRecorder> {
        let (tx, rx) = mpsc::channel::<(i32, i32)>();
        let recorder = match ClickRecorder::start(tx) {
            Ok(recorder) => recorder,
            Err(e) => {
                log::warn!("clics non enregistrés : {e}");
                return None;
            }
        };
        let annotator = Arc::clone(self);
        thread::spawn(move || {
            for (x, y) in rx {
                let choosing = lock(&annotator.recording)
                    .as_ref()
                    .is_none_or(|r| r.choosing_zone);
                if !choosing {
                    lock(&clicks).push(Click {
                        x: x as f32,
                        y: y as f32,
                        at_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
        });
        Some(recorder)
    }

    /// Zone tracée dans la fenêtre d'encre (pixels CSS) ; `None` : tout
    /// l'écran.
    fn set_zone(&self, zone: Option<(f64, f64, f64, f64)>) {
        let mut recording = lock(&self.recording);
        let Some(recording) = recording.as_mut() else {
            return;
        };
        let scale = recording.scale;
        recording.choosing_zone = false;
        recording.region = zone
            .filter(|&(_, _, w, h)| w >= 8.0 && h >= 8.0)
            .map(|(x, y, w, h)| Region {
                x: (x.max(0.0) * scale) as u32,
                y: (y.max(0.0) * scale) as u32,
                width: (w * scale) as u32,
                height: (h * scale) as u32,
            });
        if let Some(ink) = self.app.get_webview_window(INK_LABEL) {
            let _ = ink.set_ignore_cursor_events(true);
        }
    }

    /// Fenêtre transparente sur tout l'écran principal pour l'encre (et le
    /// tracé de la zone si `zone`) ; renvoie l'échelle pixels physiques / CSS.
    fn open_ink(&self, zone: bool) -> f64 {
        let Ok(Some(monitor)) = self.app.primary_monitor() else {
            return 1.0;
        };
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let position = monitor.position();
        if self.app.get_webview_window(INK_LABEL).is_none() {
            let built =
                WebviewWindowBuilder::new(&self.app, INK_LABEL, WebviewUrl::App("ink.html".into()))
                    .title("VoiceToText : encre")
                    .transparent(true)
                    .decorations(false)
                    .shadow(false)
                    .resizable(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .focused(false)
                    .position(f64::from(position.x) / scale, f64::from(position.y) / scale)
                    .inner_size(
                        f64::from(size.width) / scale,
                        f64::from(size.height) / scale,
                    )
                    .build();
            match built {
                Ok(ink) => {
                    let _ = ink.set_ignore_cursor_events(!zone);
                    if zone {
                        // Échap doit annuler le tracé : la fenêtre prend le clavier.
                        let _ = ink.set_focus();
                        // La page doit être chargée pour recevoir l'événement.
                        let app = self.app.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(400));
                            let _ = app.emit_to(INK_LABEL, "ink-zone", ());
                        });
                    }
                    #[cfg(windows)]
                    if let Ok(hwnd) = ink.hwnd() {
                        vtt_platform::exclude_from_capture(hwnd.0 as isize);
                    }
                }
                Err(e) => log::warn!("fenêtre d'encre : {e}"),
            }
        }
        scale
    }

    fn close_ink(&self) {
        if let Some(ink) = self.app.get_webview_window(INK_LABEL) {
            let _ = ink.close();
        }
    }

    /// Trait terminé, en pixels CSS de la fenêtre d'encre.
    fn add_stroke(&self, points: Vec<(f64, f64)>) {
        let recording = lock(&self.recording);
        let Some(recording) = recording.as_ref() else {
            return;
        };
        let scale = recording.scale;
        lock(&recording.strokes).push(Stroke {
            points: points
                .into_iter()
                .map(|(x, y)| ((x * scale) as f32, (y * scale) as f32))
                .collect(),
            end_ms: recording.started.elapsed().as_millis() as u64,
        });
    }

    /// Arrête la capture et lance le traitement (transcription, images,
    /// texte, collage) en arrière-plan.
    fn stop(self: &Arc<Self>) {
        let Some(recording) = lock(&self.recording).take() else {
            return;
        };
        *lock(&self.keys) = Keys::default();
        self.close_ink();
        recording.stop_frames.store(true, Ordering::SeqCst);
        let duration_ms = recording.started.elapsed().as_millis() as u64;
        let mic = recording.capture.stop().mic;
        let frames = std::mem::take(&mut *lock(&recording.frames));
        let strokes = std::mem::take(&mut *lock(&recording.strokes));
        let clicks = std::mem::take(&mut *lock(&recording.clicks));
        let region = recording.region;
        self.processing.store(true, Ordering::SeqCst);
        self.emit_state();
        self.reporter().set(Status::Processing);
        log::info!(
            "capture commentée : {} s, {} écran(s), {} trait(s)",
            duration_ms / 1000,
            frames.len(),
            strokes.len()
        );
        let annotator = Arc::clone(self);
        thread::spawn(move || {
            let marks = Marks {
                strokes,
                clicks,
                region,
            };
            let result = annotator.process(AudioClip { samples: mic }, frames, marks, duration_ms);
            annotator.processing.store(false, Ordering::SeqCst);
            match result {
                Ok(()) => annotator.reporter().set(Status::Idle),
                Err(e) => annotator
                    .reporter()
                    .set(Status::Error(format!("capture commentée : {e}"))),
            }
        });
    }

    fn transcribe(&self, clip: AudioClip) -> Result<Vec<Word>, String> {
        let worker = lock(&self.worker)
            .clone()
            .ok_or("transcription indisponible (modèle en cours de chargement ?)")?;
        let (reply, response) = mpsc::channel();
        worker
            .send(Work::Transcribe(clip, reply))
            .map_err(|_| "transcription indisponible".to_owned())?;
        response
            .recv()
            .map_err(|_| "transcription interrompue".to_owned())?
    }

    fn process(
        &self,
        clip: AudioClip,
        frames: Vec<Frame>,
        marks: Marks,
        duration_ms: u64,
    ) -> Result<(), String> {
        let Marks {
            strokes,
            clicks,
            region,
        } = marks;
        let words = self.transcribe(clip)?;
        let moments = annotation::key_moments(&words, &strokes, &clicks, duration_ms);
        // Coordonnées ramenées dans la fenêtre ou la zone capturée.
        let local = |point: (f32, f32)| region.map_or(point, |r| r.local(point));
        let times: Vec<u64> = frames.iter().map(|f| f.at_ms).collect();
        let mut images = Vec::new();
        let mut previous = 0;
        for &moment in &moments {
            let Some(index) = annotation::frame_at(&times, moment) else {
                continue;
            };
            let mut image = decode_png(&frames[index].png)?;
            if let Some(region) = &region {
                image = image.crop(region);
            }
            // Traits et clics depuis l'image précédente.
            let window = |at: u64| at > previous && at <= moment + 500;
            let drawn: Vec<Stroke> = strokes
                .iter()
                .filter(|s| window(s.end_ms))
                .map(|s| Stroke {
                    points: s.points.iter().map(|&p| local(p)).collect(),
                    end_ms: s.end_ms,
                })
                .collect();
            annotation::draw_strokes(&mut image, &drawn.iter().collect::<Vec<_>>());
            for click in clicks.iter().filter(|c| window(c.at_ms)) {
                annotation::draw_click(&mut image, local((click.x, click.y)));
            }
            images.push(image);
            previous = moment + 500;
        }
        let sheet = annotation::contact_sheet(&images);
        let narration = annotation::transcript_with_markers(&words, &moments);
        let text = self.format(&narration, &images);
        if let Some(sheet) = &sheet {
            self.keep(sheet, &images, &text);
        }
        let result = Produced { sheet, text };
        *lock(&self.last) = Some(result.clone());
        paste(&result)?;
        log::info!(
            "capture commentée collée : {} image(s), {} caractères",
            images.len(),
            result.text.chars().count()
        );
        let _ = self.app.emit("annotate-done", images.len());
        Ok(())
    }

    /// Texte mis en forme par le LLM selon le modèle choisi ; la narration
    /// telle quelle si « brut », sans LLM, ou en cas d'échec.
    fn format(&self, narration: &str, images: &[Image]) -> String {
        if narration.trim().is_empty() {
            return match images.len() {
                0 => String::new(),
                1 => "[image 1]".into(),
                n => format!("[images 1 à {n}]"),
            };
        }
        let config = self.config();
        let template = CAPTURE_TEMPLATES
            .iter()
            .find(|t| t.id == config.captures.template);
        let (Some(template), Some(cleaner)) = (template, settings::summarizer(&config)) else {
            if config.captures.template != CAPTURE_RAW_TEMPLATE {
                log::info!("capture commentée : narration brute (LLM désactivé)");
            }
            return narration.to_owned();
        };
        let (cleaner, pricing) =
            vision_client(&config).unwrap_or((cleaner, config.llm.pricing.clone()));
        let urls: Vec<String> = if config.captures.vision {
            images
                .iter()
                .filter_map(|image| encode_png(&image.scaled_to(VISION_WIDTH)).ok())
                .map(|png| vtt_llm::png_data_url(&png))
                .collect()
        } else {
            Vec::new()
        };
        let result = cleaner
            .format_capture(template, narration, &urls)
            .or_else(|e| {
                if urls.is_empty() {
                    return Err(e);
                }
                // Modèle qui ne lit pas les images : texte seul.
                log::warn!("capture commentée : images refusées par le LLM ({e}), texte seul");
                cleaner.format_capture(template, narration, &[])
            });
        match result {
            Ok(cleaned) => {
                if let Some(usage) = &cleaned.usage {
                    costs::record(&self.app.state::<LedgerStore>(), None, usage, &pricing);
                }
                cleaned.text
            }
            Err(e) => {
                log::warn!("capture commentée : mise en forme impossible ({e}), narration brute");
                narration.to_owned()
            }
        }
    }

    /// Garde la capture sur le disque si demandé ; supprime les anciennes.
    fn keep(&self, sheet: &Image, images: &[Image], text: &str) {
        let keep_days = self.config().captures.keep_days;
        self.cleanup(keep_days);
        if keep_days == 0 {
            return;
        }
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
            .to_string();
        let dir = self.dir.join(id);
        let saved = fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                fs::write(dir.join("planche.png"), encode_png(sheet)?)
                    .map_err(|e| e.to_string())?;
                for (i, image) in images.iter().enumerate() {
                    fs::write(dir.join(format!("image-{}.png", i + 1)), encode_png(image)?)
                        .map_err(|e| e.to_string())?;
                }
                fs::write(dir.join("texte.md"), text).map_err(|e| e.to_string())
            });
        if let Err(e) = saved {
            log::warn!("capture commentée non gardée : {e}");
        }
    }

    /// Supprime les captures plus vieilles que `keep_days` (toutes si 0).
    fn cleanup(&self, keep_days: u32) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let max_age = Duration::from_secs(u64::from(keep_days) * 86_400);
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age >= max_age);
            if old && let Err(e) = fs::remove_dir_all(entry.path()) {
                log::warn!("suppression de {} : {e}", entry.path().display());
            }
        }
    }

    /// Colle de nouveau la dernière capture.
    pub fn paste_last(&self) {
        let last = lock(&self.last).clone();
        let Some(last) = last else {
            self.reporter().set(Status::Error(
                "aucune capture commentée depuis le démarrage".into(),
            ));
            return;
        };
        if let Err(e) = paste(&last) {
            self.reporter()
                .set(Status::Error(format!("capture commentée : {e}")));
        }
    }
}

/// Traits, clics et zone d'une capture.
struct Marks {
    strokes: Vec<Stroke>,
    clicks: Vec<Click>,
    region: Option<Region>,
}

/// Client du profil choisi pour la description des images, s'il y en a un.
fn vision_client(
    config: &vtt_core::Config,
) -> Option<(vtt_llm::LlmCleaner, vtt_core::config::Pricing)> {
    if !config.captures.vision {
        return None;
    }
    let name = config.captures.vision_profile.trim();
    let profile = config.llm_profiles.iter().find(|p| p.name == name)?;
    Some((
        settings::llm_client(config, &profile.llm),
        profile.llm.pricing.clone(),
    ))
}

/// Fenêtre au premier plan, bornée à l'écran principal.
fn window_region(screen_width: u32, screen_height: u32) -> Option<Region> {
    let (left, top, right, bottom) = vtt_platform::foreground_rect()?;
    let x = left.clamp(0, screen_width as i32);
    let y = top.clamp(0, screen_height as i32);
    let width = right.clamp(0, screen_width as i32) - x;
    let height = bottom.clamp(0, screen_height as i32) - y;
    (width >= 8 && height >= 8).then_some(Region {
        x: x as u32,
        y: y as u32,
        width: width as u32,
        height: height as u32,
    })
}

/// Colle la planche puis le texte dans l'application active.
fn paste(result: &Produced) -> Result<(), String> {
    if !raw_paste::wait_modifiers_released() {
        return Err("touches du raccourci toujours enfoncées".into());
    }
    if let Some(sheet) = &result.sheet {
        vtt_platform::paste_image(sheet)?;
        thread::sleep(PASTE_GAP);
    }
    if !result.text.is_empty() {
        ClipboardPaster
            .insert(&format!("\n{}", result.text))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Capture l'écran régulièrement ; garde l'image quand elle a changé.
fn spawn_frames(
    frames: Arc<Mutex<Vec<Frame>>>,
    stop: Arc<AtomicBool>,
    started: Instant,
    mut last: u64,
) {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(FRAME_EVERY);
            let image = match vtt_platform::screenshot() {
                Ok(image) => image,
                Err(e) => {
                    log::warn!("capture d'écran : {e}");
                    continue;
                }
            };
            let print = fingerprint(&image);
            if print == last {
                continue;
            }
            last = print;
            let at_ms = started.elapsed().as_millis() as u64;
            match encode_png(&image) {
                Ok(png) => {
                    let mut frames = lock(&frames);
                    if frames.len() >= MAX_FRAMES {
                        frames.remove(1);
                    }
                    frames.push(Frame { at_ms, png });
                }
                Err(e) => log::warn!("compression de l'écran : {e}"),
            }
        }
    });
}

/// Raccourci : appui (démarrer, arrêter), maintien (dessiner).
pub fn register(app: &AppHandle, shortcut: &str, annotator: AnnotatorState) -> Result<(), String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Ok(());
    }
    app.global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| {
            let annotator = Arc::clone(&annotator);
            match event.state {
                ShortcutState::Pressed => {
                    thread::spawn(move || annotator.on_press());
                }
                ShortcutState::Released => {
                    thread::spawn(move || annotator.on_release());
                }
            }
        })
        .map(|()| log::info!("capture commentée : {shortcut}"))
        .map_err(|e| {
            log::warn!("raccourci « {shortcut} » indisponible : {e}");
            format!("raccourci « {shortcut} » indisponible (déjà pris par une autre application ?)")
        })
}

// ---------- Commandes ----------

#[tauri::command]
pub fn annotate_state(annotator: State<'_, AnnotatorState>) -> AnnotateState {
    annotator.state()
}

/// Trait dessiné dans la fenêtre d'encre (points en pixels CSS).
#[tauri::command]
pub fn annotate_stroke(annotator: State<'_, AnnotatorState>, points: Vec<(f64, f64)>) {
    if !points.is_empty() {
        annotator.add_stroke(points);
    }
}

/// Zone tracée (pixels CSS : x, y, largeur, hauteur) ; rien : tout l'écran.
#[tauri::command]
pub fn annotate_zone(annotator: State<'_, AnnotatorState>, zone: Option<(f64, f64, f64, f64)>) {
    annotator.set_zone(zone);
}

/// Démarre ou arrête depuis le menu de l'icône.
pub fn toggle(annotator: &AnnotatorState) {
    if lock(&annotator.recording).is_some() {
        annotator.stop();
    } else if let Err(e) = annotator.start() {
        annotator
            .reporter()
            .set(Status::Error(format!("capture commentée : {e}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip_and_change_detection() {
        let mut image = Image::new(64, 32, [10, 20, 30, 255]);
        let decoded = decode_png(&encode_png(&image).unwrap()).unwrap();
        assert_eq!(decoded, image);
        let before = fingerprint(&image);
        image.rgba[0] = 200;
        assert_ne!(fingerprint(&image), before, "premier pixel échantillonné");
    }
}
