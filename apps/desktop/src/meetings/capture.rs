//! Enregistrement en direct (micro + son du PC), import de fichiers audio/vidéo,
//! détection d'appels, proposition d'enregistrement et collage des nouvelles
//! répliques.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use vtt_core::calls::{
    self, Decision, Detector, EndDetector, EndStatus, Policy, Provider, StopReason,
};
use vtt_core::config::MeetingAudio;
use vtt_core::meeting::{
    self, Chunk, Chunker, MeetingKind, MeetingRecord, MeetingStatus, Output, Segment, Track,
};
use vtt_core::{SAMPLE_RATE, TextSink, audio, diarize};
use vtt_platform::ClipboardPaster;
use vtt_platform::capture::{Capture, Tracks};

use super::{Active, Meetings, MeetingsState, Session, lock, now_ms, recording, set_recording};
use crate::paths::Paths;
use crate::raw_paste;
use crate::settings::SharedConfig;
use crate::speakers;

/// L'audio capté est découpé et transcrit chaque seconde.
const TICK: Duration = Duration::from_secs(1);
/// Durée enregistrée dans meeting.json pendant l'enregistrement.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Relevé des programmes qui utilisent le micro (détection d'appel).
const DETECT_EVERY: Duration = Duration::from_secs(1);
/// Seuil RMS au-dessus duquel une piste est considérée active (parole/son).
const ACTIVITY_RMS_THRESHOLD: f32 = 0.008;
/// Si l'appel proposé disparaît du micro autant de temps sans réponse, on ferme
/// la proposition.
const OFFER_GONE_MS: u64 = 5_000;
/// Proposition sans réponse : refermée (jamais d'enregistrement par défaut).
const OFFER_TIMEOUT: Duration = Duration::from_secs(20);
const OFFER_LABEL: &str = "offer";
const OFFER_SIZE: (f64, f64) = (380.0, 132.0);
const OFFER_MARGIN: f64 = 16.0;

impl Session {
    pub(super) fn id(&self) -> String {
        lock(&self.record).id.clone()
    }

    /// Bloc capté : à l'analyse des voix s'il vient de la piste analysée.
    pub(super) fn feed_voices(&self, track: Track, chunk: &Chunk) {
        if Some(track) != self.voice_track || chunk.is_silent() {
            return;
        }
        if let Some(feed) = lock(&self.voices).as_ref() {
            feed.send(chunk.start_ms, chunk.samples.clone());
        }
    }

    /// Envoi en masse (fichier lu) : attend que l'analyse suive.
    pub(super) fn wait_voices(&self) {
        if let Some(feed) = lock(&self.voices).as_ref() {
            feed.wait_backlog();
        }
    }

    pub(super) fn transcribed(
        self: &Arc<Self>,
        track: Track,
        start_ms: u64,
        end_ms: u64,
        result: Result<Option<String>, vtt_core::Error>,
    ) {
        {
            let mut record = lock(&self.record);
            match result {
                Ok(Some(text)) => {
                    record.segments.push(Segment {
                        track,
                        start_ms,
                        end_ms,
                        text,
                        copied: false,
                        speaker: None,
                    });
                    meeting::sort_segments(&mut record.segments);
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("réunion {} : bloc non transcrit : {e}", record.id);
                    record.failed_chunks += 1;
                }
            }
            record.save(&self.dir);
        }
        let _ = self.app.emit("meetings", ());
        self.pending.fetch_sub(1, Ordering::SeqCst);
        self.finish_if_done();
    }

    /// Plus rien à capter ni à transcrire : réunion terminée (après
    /// l'analyse des voix, si elle a lieu).
    pub(super) fn finish_if_done(self: &Arc<Self>) {
        if self.capturing.load(Ordering::SeqCst) || self.pending.load(Ordering::SeqCst) > 0 {
            return;
        }
        if lock(&self.record).status == MeetingStatus::Terminee
            || self.finishing.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let Some(feed) = lock(&self.voices).take() else {
            self.complete(None);
            return;
        };
        {
            let mut record = lock(&self.record);
            record.identifying = true;
            record.save(&self.dir);
        }
        let _ = self.app.emit("meetings", ());
        let session = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("voices-finish".into())
            .spawn(move || {
                let result = feed.finish();
                session.complete(Some(result));
            });
        if let Err(e) = spawned {
            log::warn!("fin de l'analyse des voix : {e}");
            self.complete(Some(Err(e.to_string())));
        }
    }

    fn complete(&self, voices: Option<Result<diarize::Analysis, String>>) {
        let (id, audio, title, kind, output, completed) = {
            let mut record = lock(&self.record);
            if record.status == MeetingStatus::Terminee {
                return;
            }
            match voices {
                Some(Ok(analysis)) => {
                    let book = self
                        .app
                        .try_state::<MeetingsState>()
                        .map(|m| m.voicebook.load_or_empty());
                    record.set_speakers_with_voicebook(&analysis, book.as_ref());
                    if record.keeps_voices()
                        && let Err(e) = speakers::save_analysis(&self.dir, &analysis)
                    {
                        // Les tours de parole restent ; seul le changement
                        // du nombre d'intervenants demandera l'audio.
                        log::warn!("analyse des voix non enregistrée : {e}");
                    }
                }
                Some(Err(e)) => record
                    .notes
                    .push(format!("Intervenants non distingués : {e}")),
                None => {}
            }
            record.identifying = false;
            record.status = MeetingStatus::Terminee;
            record.progress = None;
            record.save(&self.dir);
            let completed = record.clone();
            (
                record.id.clone(),
                record.audio,
                record.title.clone(),
                record.kind,
                record.output,
                completed,
            )
        };
        if audio == MeetingAudio::Temporaire {
            meeting::delete_audio(&self.dir);
        }
        log::info!("réunion {id} transcrite");
        if let Some(meetings) = self.app.try_state::<MeetingsState>() {
            meetings.finished(&id, &title, kind, output);
            if meetings.config().meetings.semantic_search
                && let Some(semantic) = self.app.try_state::<crate::semantic::SemanticState>()
            {
                semantic.index_meeting(completed);
            }
        }
        let _ = self.app.emit("meetings", ());
    }
}

impl Meetings {
    /// Démarre l'enregistrement d'une réunion ; renvoie son identifiant.
    /// `call` : service de l'appel détecté, pour le titre.
    pub fn start(self: &Arc<Self>, call: Option<&str>) -> Result<String, String> {
        let kind = call.map_or_else(|| "Réunion".to_owned(), |label| format!("Appel {label}"));
        let system = self.config().meetings.system_audio;
        self.start_recording(
            MeetingKind::Reunion,
            &kind,
            true,
            system,
            Output::LesDeux,
            None,
        )
    }

    fn start_call(self: &Arc<Self>, provider: &'static Provider) -> Result<String, String> {
        let label = format!("Appel {}", provider.label);
        let system = self.config().meetings.system_audio;
        self.start_recording(
            MeetingKind::Reunion,
            &label,
            true,
            system,
            Output::LesDeux,
            Some(provider),
        )
    }

    /// Démarre l'enregistrement d'une conférence (micro, son du PC ou les
    /// deux).
    pub fn start_conference(
        self: &Arc<Self>,
        mic: bool,
        system: bool,
        output: Output,
    ) -> Result<String, String> {
        if !mic && !system {
            return Err("choisissez le micro, le son du PC, ou les deux".into());
        }
        self.start_recording(
            MeetingKind::Conference,
            "Conférence",
            mic,
            system,
            output,
            None,
        )
    }

    fn start_recording(
        self: &Arc<Self>,
        kind: MeetingKind,
        label: &str,
        mic: bool,
        system_audio: bool,
        output: Output,
        call_provider: Option<&'static Provider>,
    ) -> Result<String, String> {
        let mut active = lock(&self.active);
        if active.is_some() {
            return Err("un enregistrement est déjà en cours".into());
        }
        let config = self.config();
        let settings = config.meetings.clone();
        let device = Some(config.audio.device.trim()).filter(|d| !d.is_empty());
        let capture = Capture::start(device, system_audio)?;
        let started_at_ms = now_ms();
        let id = started_at_ms.to_string();
        let dir = self.dir.join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut notes = Vec::new();
        if system_audio && capture.outputs.is_empty() {
            if !mic {
                return Err("son du PC non capté (aucune sortie audio active)".into());
            }
            notes.push("Son du PC non capté : seul le micro est transcrit.".to_owned());
        }
        let mut record = MeetingRecord::new(id.clone(), default_title(label), started_at_ms, kind);
        record.audio = settings.audio;
        record.system = !capture.outputs.is_empty();
        record.notes = notes;
        // Écho : seulement pour un appel (micro et son du PC).
        record.echo_filter = settings.echo_filter && mic;
        record.output = output;
        record.mic = mic;
        record.save(&dir);
        log::info!(
            "réunion {id} : enregistrement (audio : {:?}, son du PC : {})",
            settings.audio,
            if capture.outputs.is_empty() {
                "non".to_owned()
            } else {
                capture.outputs.join(", ")
            }
        );
        let session = self.new_session(dir, record);
        lock(&self.sessions).insert(id.clone(), Arc::clone(&session));
        let last_audio_ms = Arc::new(AtomicU64::new(started_at_ms));
        let last_system_audio_ms = Arc::new(AtomicU64::new(0));
        let stopping_in_sec = Arc::new(AtomicU32::new(0));
        let (stop_tx, stop_rx) = mpsc::channel();
        let meetings = Arc::clone(self);
        let recorder = Arc::clone(&session);
        let audio_ts = Arc::clone(&last_audio_ms);
        let system_ts = Arc::clone(&last_system_audio_ms);
        thread::Builder::new()
            .name("meeting".into())
            .spawn(move || meetings.record(recorder, capture, stop_rx, audio_ts, system_ts))
            .map_err(|e| format!("thread de réunion : {e}"))?;
        *active = Some(Active {
            session,
            stop: stop_tx,
            started: Instant::now(),
            started_at_ms,
            kind,
            call_provider,
            last_audio_ms,
            last_system_audio_ms,
            stopping_in_sec,
        });
        drop(active);
        set_recording(true);
        self.emit_state(true);
        let _ = self.app.emit("meetings", ());
        Ok(id)
    }

    /// Arrête l'enregistrement ; la transcription des derniers blocs suit.
    pub fn stop(&self) -> Result<(), String> {
        let active = lock(&self.active)
            .take()
            .ok_or("aucune réunion en cours d'enregistrement")?;
        let _ = active.stop.send(());
        set_recording(false);
        self.emit_state(false);
        Ok(())
    }

    /// Arrêt automatique déclenché par [`EndDetector`] : consigne la raison
    /// dans la fiche, arrête l'enregistrement et prévient l'interface.
    fn stop_auto(&self, reason: &StopReason) {
        let (note, toast) = match reason {
            StopReason::CallEnded(provider) => (
                format!(
                    "Enregistrement arrêté automatiquement (fin de l'appel {}).",
                    provider.label
                ),
                format!("Fin de l'appel {} : réunion arrêtée", provider.label),
            ),
            StopReason::Silence(mins) => (
                format!("Enregistrement arrêté automatiquement (silence de {mins} min)."),
                format!("Silence ({mins} min) : réunion arrêtée"),
            ),
        };
        if let Some(active) = lock(&self.active).as_ref() {
            let mut record = lock(&active.session.record);
            record.notes.push(note.clone());
            record.save(&active.session.dir);
        }
        if let Err(e) = self.stop() {
            log::debug!("arrêt automatique ignoré ({e})");
            return;
        }
        log::info!("{note}");
        let _ = self.app.emit("meeting-auto-stopped", toast);
    }

    pub fn toggle(self: &Arc<Self>) {
        let result = if lock(&self.active).is_some() {
            self.stop()
        } else {
            self.start(None).map(|_| ())
        };
        if let Err(e) = result {
            log::warn!("réunion : {e}");
            let _ = self.app.emit("meeting-error", e);
        }
    }

    /// Détection d'appel (début et fin) : chaque seconde, relève les
    /// programmes qui utilisent le micro.
    /// - Hors enregistrement : propose ou démarre selon le choix du service.
    /// - En cours de réunion : détecte la fin de l'appel (ou un silence
    ///   prolongé) et arrête automatiquement l'enregistrement.
    pub fn spawn_detection(self: &Arc<Self>) {
        let meetings = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("call-detection".into())
            .spawn(move || {
                let mut detector = Detector::default();
                let mut end_watch: Option<(String, EndDetector)> = None;
                let mut offer_absent_since: Option<u64> = None;
                loop {
                    thread::sleep(DETECT_EVERY);
                    let settings = meetings.config().meetings;
                    let is_recording = recording();
                    let watch_end =
                        is_recording && (settings.auto_stop || settings.auto_stop_silence_min > 0);
                    if !settings.detect && !watch_end {
                        detector = Detector::default();
                        end_watch = None;
                        offer_absent_since = None;
                        continue;
                    }
                    let apps = vtt_platform::microphone_apps();
                    let now = now_ms();

                    // 1. Fin d'enregistrement automatique (réunions uniquement, jamais conférences).
                    if is_recording {
                        offer_absent_since = None;
                        let active_info = lock(&meetings.active).as_ref().map(|a| {
                            (
                                a.session.id(),
                                a.kind,
                                a.started_at_ms,
                                a.call_provider,
                                a.last_audio_ms.load(Ordering::Relaxed),
                                a.last_system_audio_ms.load(Ordering::Relaxed),
                                Arc::clone(&a.stopping_in_sec),
                            )
                        });
                        if let Some((
                            id,
                            MeetingKind::Reunion,
                            started_at_ms,
                            call_provider,
                            last_audio_ms,
                            last_system_audio_ms,
                            stopping_in_sec,
                        )) = active_info
                        {
                            let end = match &mut end_watch {
                                Some((watched_id, d)) if *watched_id == id => d,
                                _ => {
                                    end_watch =
                                        Some((id, EndDetector::new(started_at_ms, call_provider)));
                                    &mut end_watch.as_mut().unwrap().1
                                }
                            };
                            let grace_ms = u64::from(settings.auto_stop_delay_sec) * 1_000;
                            let silence_ms = (settings.auto_stop_silence_min > 0)
                                .then(|| u64::from(settings.auto_stop_silence_min) * 60_000);
                            match end.observe(
                                now,
                                &apps,
                                last_audio_ms,
                                last_system_audio_ms,
                                settings.auto_stop,
                                grace_ms,
                                silence_ms,
                            ) {
                                EndStatus::Active => {
                                    if stopping_in_sec.swap(0, Ordering::Relaxed) != 0 {
                                        meetings.emit_state(false);
                                    }
                                }
                                EndStatus::StoppingIn { remaining_ms, .. } => {
                                    let sec = remaining_ms.div_ceil(1_000).max(1) as u32;
                                    if stopping_in_sec.swap(sec, Ordering::Relaxed) != sec {
                                        meetings.emit_state(false);
                                    }
                                }
                                EndStatus::Stop(reason) => {
                                    stopping_in_sec.store(0, Ordering::Relaxed);
                                    meetings.stop_auto(&reason);
                                    if let StopReason::CallEnded(provider) = reason {
                                        detector.unsnooze(provider.key);
                                    }
                                    end_watch = None;
                                    continue;
                                }
                            }
                        } else {
                            end_watch = None;
                        }
                    } else {
                        end_watch = None;
                    }

                    // 2. Si une fenêtre de proposition est ouverte mais que l'appel s'est déjà
                    //    terminé avant la réponse, la refermer proprement.
                    let offered = lock(&meetings.offer).map(|(p, _)| p);
                    if let Some(provider) = offered {
                        let still_on = apps
                            .iter()
                            .filter_map(|e| calls::provider(e))
                            .any(|p| p.key == provider.key);
                        if still_on {
                            offer_absent_since = None;
                        } else {
                            let since = *offer_absent_since.get_or_insert(now);
                            if now.saturating_sub(since) >= OFFER_GONE_MS {
                                log::info!(
                                    "appel {} terminé avant réponse : proposition fermée",
                                    provider.label
                                );
                                meetings.close_offer();
                                detector.unsnooze(provider.key);
                                offer_absent_since = None;
                            }
                        }
                    } else {
                        offer_absent_since = None;
                    }

                    // 3. Détection de début d'appel.
                    if !settings.detect {
                        continue;
                    }
                    let decision =
                        detector.observe(now, &apps, is_recording, |key| settings.policy(key));
                    match decision {
                        Some(Decision::Offer(provider)) => {
                            log::info!("appel détecté : {}", provider.label);
                            meetings.show_offer(provider);
                        }
                        Some(Decision::Record(provider)) => {
                            log::info!(
                                "appel détecté : {} (enregistrement automatique)",
                                provider.label
                            );
                            if let Err(e) = meetings.start_call(provider) {
                                log::warn!("enregistrement automatique : {e}");
                            }
                        }
                        None => {}
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("détection d'appel : {e}");
        }
    }

    /// Petite fenêtre « Appel détecté : Enregistrer / Pas maintenant », en
    /// bas à droite, sans prendre le focus de l'appel.
    fn show_offer(self: &Arc<Self>, provider: &'static Provider) {
        *lock(&self.offer) = Some((provider, Instant::now()));
        if let Some(window) = self.app.get_webview_window(OFFER_LABEL) {
            let _ = self.app.emit_to(OFFER_LABEL, "offer", ());
            let _ = window.show();
        } else {
            let mut builder = WebviewWindowBuilder::new(
                &self.app,
                OFFER_LABEL,
                WebviewUrl::App("offer.html".into()),
            )
            .title("VoiceToText : appel détecté")
            .inner_size(OFFER_SIZE.0, OFFER_SIZE.1)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false);
            if let Ok(Some(monitor)) = self.app.primary_monitor() {
                let scale = monitor.scale_factor();
                let area = monitor.work_area();
                let right = f64::from(area.position.x) / scale + f64::from(area.size.width) / scale;
                let bottom =
                    f64::from(area.position.y) / scale + f64::from(area.size.height) / scale;
                builder = builder.position(
                    right - OFFER_SIZE.0 - OFFER_MARGIN,
                    bottom - OFFER_SIZE.1 - OFFER_MARGIN,
                );
            }
            if let Err(e) = builder.build() {
                log::warn!("fenêtre de proposition : {e}");
                return;
            }
        }
        // Sans réponse : refermée, rien n'est enregistré.
        let meetings = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(OFFER_TIMEOUT);
            let expired =
                lock(&meetings.offer).is_some_and(|(_, shown)| shown.elapsed() >= OFFER_TIMEOUT);
            if expired {
                meetings.close_offer();
            }
        });
    }

    fn close_offer(&self) {
        *lock(&self.offer) = None;
        if let Some(window) = self.app.get_webview_window(OFFER_LABEL) {
            let _ = window.close();
        }
    }

    /// Réponse à la proposition.
    pub(super) fn answer_offer(self: &Arc<Self>, choice: &str) -> Result<(), String> {
        let provider = lock(&self.offer).map(|(p, _)| p);
        self.close_offer();
        let Some(provider) = provider else {
            return Ok(());
        };
        let policy = match choice {
            "toujours" => Some(Policy::Toujours),
            "jamais" => Some(Policy::Jamais),
            _ => None,
        };
        if let Some(policy) = policy {
            self.save_policy(provider.key, policy)?;
        }
        if matches!(choice, "enregistrer" | "toujours") {
            self.start_call(provider)?;
        }
        Ok(())
    }

    fn save_policy(&self, key: &str, policy: Policy) -> Result<(), String> {
        let paths = self.app.state::<Paths>();
        let (mut config, error) = paths.load_config();
        if let Some(error) = error {
            return Err(error);
        }
        config.meetings.set_policy(key, policy);
        paths.write_config(&config)?;
        *self
            .app
            .state::<SharedConfig>()
            .write()
            .unwrap_or_else(|e| e.into_inner()) = config;
        log::info!("appels {key} : {policy:?}");
        Ok(())
    }

    /// Thread d'enregistrement : chaque seconde, l'audio capté est écrit
    /// (selon le réglage) et découpé en blocs pour la transcription.
    fn record(
        &self,
        session: Arc<Session>,
        capture: Capture,
        stop: mpsc::Receiver<()>,
        last_audio_ms: Arc<AtomicU64>,
        last_system_audio_ms: Arc<AtomicU64>,
    ) {
        let (dir, mut audio, system, echo_filter, mic_on) = {
            let record = lock(&session.record);
            (
                session.dir.clone(),
                record.audio,
                record.system,
                record.echo_filter,
                record.mic,
            )
        };
        let mut echo = meeting::EchoGate::default();
        let mut chunkers = [Chunker::default(), Chunker::default()];
        // `Capture::stop` consomme la capture : gardée jusqu'à l'arrêt.
        let mut capture = Some(capture);
        let started = Instant::now();
        let mut last_save = Instant::now();
        loop {
            let stopped = match stop.recv_timeout(TICK) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
                Err(RecvTimeoutError::Timeout) => false,
            };
            let tracks = if stopped {
                capture.take().map(Capture::stop).unwrap_or_default()
            } else {
                capture.as_ref().map(Capture::take).unwrap_or_default()
            };
            let Tracks { mic, system: other } = tracks;
            let tick_now = now_ms();
            // Son du PC d'abord : il sert à reconnaître l'écho dans le micro.
            let mut feeds = Vec::new();
            if system {
                if !audio::is_silent(&other, SAMPLE_RATE, ACTIVITY_RMS_THRESHOLD) {
                    last_system_audio_ms.store(tick_now, Ordering::Relaxed);
                    last_audio_ms.store(tick_now, Ordering::Relaxed);
                }
                echo.feed_system(&other);
                feeds.push((Track::Eux, other));
            }
            if mic_on {
                if !audio::is_silent(&mic, SAMPLE_RATE, ACTIVITY_RMS_THRESHOLD) {
                    last_audio_ms.store(tick_now, Ordering::Relaxed);
                }
                feeds.push((Track::Moi, mic));
            }
            for (track, samples) in feeds {
                if audio != MeetingAudio::Memoire
                    && let Err(e) = meeting::append_audio(
                        &dir.join(track.file()),
                        &samples,
                        vtt_platform::protect,
                    )
                {
                    // Pas de chiffrement : rien sur le disque, jamais en clair.
                    log::warn!("audio de la réunion non écrit : {e}");
                    audio = MeetingAudio::Memoire;
                    meeting::delete_audio(&dir);
                    let mut record = lock(&session.record);
                    record.audio = MeetingAudio::Memoire;
                    record.notes.push(format!(
                        "Audio non enregistré sur le disque ({e}) : gardé en mémoire seulement."
                    ));
                }
                let chunker = &mut chunkers[track as usize];
                let mut chunks = chunker.feed(&samples);
                if stopped {
                    chunks.extend(chunker.finish());
                }
                for chunk in chunks {
                    session.feed_voices(track, &chunk);
                    if track == Track::Moi && system && echo_filter && echo.is_echo(&chunk) {
                        lock(&session.record).echo_chunks += 1;
                        continue;
                    }
                    self.enqueue(&session, track, chunk);
                }
            }
            if stopped || last_save.elapsed() >= SAVE_EVERY {
                let mut record = lock(&session.record);
                record.duration_ms = started.elapsed().as_millis() as u64;
                if stopped {
                    record.status = MeetingStatus::Transcription;
                }
                record.save(&dir);
                last_save = Instant::now();
            }
            if stopped {
                break;
            }
        }
        log::info!("réunion {} : enregistrement arrêté", session.id());
        session.capturing.store(false, Ordering::SeqCst);
        let _ = self.app.emit("meetings", ());
        session.finish_if_done();
    }

    /// Fin de la transcription : compte rendu automatique si demandé.
    fn finished(self: &Arc<Self>, id: &str, title: &str, kind: MeetingKind, output: Output) {
        lock(&self.sessions).remove(id);
        let config = self.config();
        // Réunion : réglage « compte rendu automatique » ; conférence et
        // fichier : choix fait au lancement.
        let wanted = match kind {
            MeetingKind::Reunion => config.meetings.auto_summary,
            _ => output != Output::Transcription,
        };
        if wanted && config.llm.enabled {
            let meetings = Arc::clone(self);
            let id = id.to_owned();
            thread::spawn(move || match meetings.summarize(&id, None) {
                Ok(()) if output == Output::CompteRendu => {
                    let _ = meetings.update(&id, |r| r.segments.clear());
                    log::info!("{id} : transcription effacée (compte rendu seul)");
                }
                Ok(()) => {}
                Err(e) => log::warn!("compte rendu de {id} : {e}"),
            });
        }
        let what = match kind {
            MeetingKind::Reunion => "réunion transcrite",
            MeetingKind::Conference => "conférence transcrite",
            MeetingKind::Fichier => "fichier transcrit",
        };
        let reporter = self.app.state::<Arc<crate::status::StatusReporter>>();
        reporter.notify(
            &format!("VoiceToText : {what}"),
            &format!("« {title} » est prête dans la page Réunions."),
        );
    }

    /// Transcrit un fichier audio ou vidéo, lu au fil de l'eau ; renvoie
    /// l'identifiant de l'enregistrement créé.
    pub fn import(self: &Arc<Self>, path: PathBuf, output: Output) -> Result<String, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        let started_at_ms = now_ms();
        let id = started_at_ms.to_string();
        let dir = self.dir.join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut record = MeetingRecord::new(id.clone(), title, started_at_ms, MeetingKind::Fichier);
        record.status = MeetingStatus::Transcription;
        record.audio = MeetingAudio::Memoire;
        record.system = true;
        record.output = output;
        record.mic = false;
        record.source = Some(name);
        record.source_path = Some(path.display().to_string());
        record.progress = Some(0);
        record.save(&dir);
        let session = self.new_session(dir, record);
        lock(&self.sessions).insert(id.clone(), Arc::clone(&session));
        let meetings = Arc::clone(self);
        thread::Builder::new()
            .name("import".into())
            .spawn(move || meetings.read_file(&path, session))
            .map_err(|e| format!("thread d'import : {e}"))?;
        let _ = self.app.emit("meetings", ());
        log::info!("import {id} : lecture du fichier");
        Ok(id)
    }

    /// Lit le fichier importé et envoie ses blocs à la transcription, sans
    /// prendre d'avance (quelques blocs en attente au plus : la mémoire reste
    /// faible même pour des heures d'audio).
    fn read_file(&self, path: &Path, session: Arc<Session>) {
        const MAX_PENDING: usize = 6;
        let mut chunker = Chunker::default();
        let mut last_saved = 0;
        let result = crate::decode::decode(
            path,
            |samples| {
                for chunk in chunker.feed(samples) {
                    session.feed_voices(Track::Eux, &chunk);
                    self.enqueue(&session, Track::Eux, chunk);
                }
                while session.pending.load(Ordering::SeqCst) > MAX_PENDING {
                    thread::sleep(Duration::from_millis(200));
                }
                session.wait_voices();
                true
            },
            |percent| {
                if percent >= last_saved + 2 || percent == 100 {
                    last_saved = percent;
                    let mut record = lock(&session.record);
                    record.progress = Some(percent);
                    record.save(&session.dir);
                    drop(record);
                    let _ = self.app.emit("meetings", ());
                }
            },
        );
        if let Some(chunk) = chunker.finish() {
            session.feed_voices(Track::Eux, &chunk);
            self.enqueue(&session, Track::Eux, chunk);
        }
        {
            let mut record = lock(&session.record);
            match result {
                Ok(duration_ms) => record.duration_ms = duration_ms,
                Err(e) => {
                    log::warn!("import {} : {e}", record.id);
                    record
                        .notes
                        .push(format!("Lecture du fichier interrompue : {e}"));
                }
            }
            record.save(&session.dir);
        }
        session.capturing.store(false, Ordering::SeqCst);
        let _ = self.app.emit("meetings", ());
        session.finish_if_done();
    }

    /// « Copier les nouvelles répliques » : colle dans l'application active
    /// ce qui a été dit depuis la dernière copie (réunion en cours).
    pub fn paste_new(&self) {
        let session = lock(&self.active).as_ref().map(|a| Arc::clone(&a.session));
        let Some(session) = session else {
            let _ = self
                .app
                .emit("meeting-error", "aucune réunion en cours d'enregistrement");
            return;
        };
        let text = {
            let mut record = lock(&session.record);
            let text = collect_fresh_turns(&mut record);
            record.save(&session.dir);
            text
        };
        if text.is_empty() {
            let _ = self
                .app
                .emit("meeting-error", "rien de nouveau depuis la dernière copie");
            return;
        }
        if !raw_paste::wait_modifiers_released() {
            log::warn!("nouvelles répliques : touches du raccourci toujours enfoncées");
            return;
        }
        match ClipboardPaster.insert(&text) {
            Ok(()) => log::info!("nouvelles répliques collées ({} caractères)", text.len()),
            Err(e) => log::warn!("nouvelles répliques : {e}"),
        }
    }
}

/// Formate les répliques non encore copiées et les marque toutes comme copiées.
pub(super) fn collect_fresh_turns(record: &mut MeetingRecord) -> String {
    let fresh: Vec<Segment> = record
        .segments()
        .into_iter()
        .filter(|s| !s.copied)
        .collect();
    for segment in &mut record.segments {
        segment.copied = true;
    }
    let names = record.names();
    meeting::turns(&fresh)
        .into_iter()
        .map(|r| {
            let who = meeting::speaker_label(r.track, r.speaker, &names);
            format!("{who} : {}", r.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// « Réunion du 4 nov. 2026 à 14:30 », « Appel Zoom du … », « Conférence du … ».
pub(super) fn default_title(kind: &str) -> String {
    const MONTHS: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!(
        "{kind} du {} {} {} à {:02}:{:02}",
        now.day(),
        MONTHS[usize::from(u8::from(now.month())) - 1],
        now.year(),
        now.hour(),
        now.minute()
    )
}
