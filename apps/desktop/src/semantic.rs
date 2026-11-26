//! Recherche sémantique locale (ONNX Runtime) et indexation incrémentale des
//! réunions sur le PC.
//!
//! - Modèle multilingue léger (`intfloat/multilingual-e5-small`, quantifié
//!   8 bits, ~135 Mo + ONNX Runtime s'il n'est pas déjà installé par Parakeet
//!   ou la diarisation), téléchargé uniquement à l'activation du réglage et
//!   vérifié par taille + SHA-256.
//! - Index compact (`semantique.bin`, vecteurs unitaires quantifiés Q15) stocké
//!   dans le dossier de chaque réunion, invalidé automatiquement si la
//!   transcription, les noms d'intervenants ou le rapport changent.
//! - Tout reste 100 % local : aucune donnée n'est envoyée à l'extérieur.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use vtt_asr::TextEmbedder;
use vtt_core::meeting::{MeetingRecord, MeetingStatus};
use vtt_core::semantic::{
    self, Embedder, MeetingIndex, build_meeting_index, load_index, save_index,
};

use crate::model;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Default)]
struct Download {
    running: bool,
    percent: u8,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct IndexStatus {
    running: bool,
    indexed: usize,
    total: usize,
}

/// État affiché dans Réglages → Réunions → Recherche sémantique locale.
#[derive(Debug, Clone, Serialize)]
pub struct SemanticView {
    pub installed: bool,
    /// Reste à télécharger (Mo).
    pub missing_mb: u64,
    pub downloading: bool,
    pub percent: u8,
    pub error: Option<String>,
    pub indexing: bool,
    pub indexed_meetings: usize,
    pub total_meetings: usize,
}

type Events = Box<dyn Fn(SemanticView) + Send + Sync>;

pub struct Semantic {
    events: Events,
    models_dir: PathBuf,
    meetings_dir: PathBuf,
    embedder: Mutex<Option<TextEmbedder>>,
    download: Mutex<Download>,
    index_status: Mutex<IndexStatus>,
    /// Cache mémoire des index déjà lus ou calculés (`id -> MeetingIndex`).
    cache: Mutex<HashMap<String, MeetingIndex>>,
}

pub type SemanticState = Arc<Semantic>;

impl Semantic {
    pub fn new(app: AppHandle, models_dir: PathBuf, meetings_dir: PathBuf) -> SemanticState {
        Self::with_events(
            models_dir,
            meetings_dir,
            Box::new(move |view| {
                let _ = app.emit("semantic", view);
            }),
        )
    }

    fn with_events(models_dir: PathBuf, meetings_dir: PathBuf, events: Events) -> SemanticState {
        Arc::new(Self {
            events,
            models_dir,
            meetings_dir,
            embedder: Mutex::new(None),
            download: Mutex::new(Download::default()),
            index_status: Mutex::new(IndexStatus::default()),
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn installed(&self) -> bool {
        model::semantic_local(&self.models_dir).is_some()
    }

    pub fn view(&self) -> SemanticView {
        let download = lock(&self.download).clone();
        let idx = lock(&self.index_status).clone();
        SemanticView {
            installed: self.installed(),
            missing_mb: model::semantic_missing_mb(&self.models_dir),
            downloading: download.running,
            percent: download.percent,
            error: download.error,
            indexing: idx.running,
            indexed_meetings: idx.indexed,
            total_meetings: idx.total,
        }
    }

    fn emit(&self) {
        (self.events)(self.view());
    }

    /// Libère le modèle ONNX de la mémoire (quand l'option est désactivée).
    pub fn unload(&self) {
        *lock(&self.embedder) = None;
    }

    /// Supprime un index du cache mémoire (après suppression d'une réunion).
    pub fn remove_cached(&self, id: &str) {
        lock(&self.cache).remove(id);
    }

    /// Met à jour les compteurs `indexed / total` d'après les fichiers présents
    /// sur disque (sans lancer d'inférence ONNX).
    pub fn refresh_counts(&self, records: &[MeetingRecord]) {
        let mut total = 0usize;
        let mut indexed = 0usize;
        for record in records {
            if record.status != MeetingStatus::Terminee || record.segments.is_empty() {
                continue;
            }
            total += 1;
            if self.cached_or_disk_index(record).is_some() {
                indexed += 1;
            }
        }
        {
            let mut status = lock(&self.index_status);
            status.total = total;
            status.indexed = indexed;
        }
    }

    /// Télécharge le modèle d'embeddings en arrière-plan, puis lance
    /// l'indexation des réunions fournies par `records_provider`.
    pub fn download(
        self: &Arc<Self>,
        records_provider: impl FnOnce() -> Vec<MeetingRecord> + Send + 'static,
    ) {
        if self.installed() {
            let records = records_provider();
            self.sync_all(records);
            return;
        }
        {
            let mut download = lock(&self.download);
            if download.running {
                return;
            }
            *download = Download {
                running: true,
                ..Download::default()
            };
        }
        self.emit();
        let semantic = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("semantic-download".into())
            .spawn(move || {
                let result = model::ensure_semantic(&semantic.models_dir, |percent| {
                    lock(&semantic.download).percent = percent;
                    semantic.emit();
                });
                let installed_now = {
                    let mut download = lock(&semantic.download);
                    download.running = false;
                    match result {
                        Ok(_) => {
                            log::info!("modèle de recherche sémantique installé");
                            true
                        }
                        Err(e) => {
                            log::warn!("modèle de recherche sémantique : {e}");
                            download.error = Some(e);
                            false
                        }
                    }
                };
                semantic.emit();
                if installed_now {
                    let records = records_provider();
                    semantic.sync_all(records);
                }
            });
        if let Err(e) = spawned {
            log::warn!("téléchargement du modèle sémantique : {e}");
            lock(&self.download).running = false;
        }
    }

    /// Indexe en arrière-plan toutes les réunions terminées dont
    /// `semantique.bin` est absent ou obsolète (un seul travail à la fois).
    pub fn sync_all(self: &Arc<Self>, records: Vec<MeetingRecord>) {
        if !self.installed() {
            return;
        }
        {
            let mut status = lock(&self.index_status);
            if status.running {
                return;
            }
            status.running = true;
        }
        self.emit();
        let semantic = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("semantic-index".into())
            .spawn(move || {
                semantic.run_sync(&records);
                lock(&semantic.index_status).running = false;
                semantic.emit();
            });
        if let Err(e) = spawned {
            log::warn!("indexation sémantique : {e}");
            lock(&self.index_status).running = false;
            self.emit();
        }
    }

    /// Met à jour l'index d'une réunion précise en arrière-plan (après fin de
    /// transcription, diarisation, rapport ou renommage d'intervenant).
    pub fn index_meeting(self: &Arc<Self>, record: MeetingRecord) {
        if !self.installed() || record.status != MeetingStatus::Terminee {
            return;
        }
        let semantic = Arc::clone(self);
        let _ = thread::Builder::new()
            .name("semantic-one".into())
            .spawn(move || {
                if let Err(e) = semantic.ensure_meeting_index(&record) {
                    log::warn!("index sémantique de {} : {e}", record.id);
                } else {
                    semantic.emit();
                }
            });
    }

    fn run_sync(&self, records: &[MeetingRecord]) {
        let eligible: Vec<&MeetingRecord> = records
            .iter()
            .filter(|r| r.status == MeetingStatus::Terminee && !r.segments.is_empty())
            .collect();
        let total = eligible.len();
        {
            let mut status = lock(&self.index_status);
            status.total = total;
            status.indexed = 0;
        }
        self.emit();

        let started = Instant::now();
        let mut rebuilt = 0usize;
        let mut indexed = 0usize;

        for record in eligible {
            if self.cached_or_disk_index(record).is_some() {
                indexed += 1;
            } else {
                match self.ensure_meeting_index(record) {
                    Ok(_) => {
                        indexed += 1;
                        rebuilt += 1;
                    }
                    Err(e) => {
                        log::warn!("index sémantique de {} : {e}", record.id);
                    }
                }
            }
            {
                let mut status = lock(&self.index_status);
                status.indexed = indexed;
                status.total = total;
            }
            self.emit();
        }
        if rebuilt > 0 {
            log::info!(
                "index sémantique : {rebuilt} réunion(s) indexée(s) en {} ms ({indexed}/{total})",
                started.elapsed().as_millis()
            );
        }
    }

    fn meeting_dir(&self, id: &str) -> Option<PathBuf> {
        vtt_core::meeting::meeting_dir(&self.meetings_dir, id).ok()
    }

    /// Renvoie l'index à jour d'une réunion depuis le cache mémoire ou disque,
    /// sans jamais appeler ONNX.
    fn cached_or_disk_index(&self, record: &MeetingRecord) -> Option<MeetingIndex> {
        if let Some(cached) = lock(&self.cache).get(&record.id)
            && cached.is_fresh_for(record)
        {
            return Some(cached.clone());
        }
        let dir = self.meeting_dir(&record.id)?;
        let loaded = load_index(&dir).ok()?;
        if !loaded.is_fresh_for(record) {
            return None;
        }
        lock(&self.cache).insert(record.id.clone(), loaded.clone());
        Some(loaded)
    }

    fn ensure_meeting_index(&self, record: &MeetingRecord) -> Result<MeetingIndex, String> {
        if let Some(fresh) = self.cached_or_disk_index(record) {
            return Ok(fresh);
        }
        let dir = self
            .meeting_dir(&record.id)
            .ok_or_else(|| format!("identifiant de réunion invalide : {}", record.id))?;
        if !dir.is_dir() {
            return Err("dossier de réunion introuvable".into());
        }
        let index =
            self.with_embedder(|e| build_meeting_index(record, e).map_err(|err| err.to_string()))?;
        save_index(&dir, &index)?;
        lock(&self.cache).insert(record.id.clone(), index.clone());
        Ok(index)
    }

    /// Charge tous les index à jour disponibles pour `records` (très rapide,
    /// aucune inférence ONNX). Si `index_missing` vaut `true`, indexe à la
    /// volée les réunions manquantes (utile pour « Poser une question »).
    pub fn indices_for(
        &self,
        records: &[MeetingRecord],
        index_missing: bool,
    ) -> Vec<Option<MeetingIndex>> {
        records
            .iter()
            .map(|r| {
                if let Some(fresh) = self.cached_or_disk_index(r) {
                    return Some(fresh);
                }
                if index_missing && r.status == MeetingStatus::Terminee && !r.segments.is_empty() {
                    return self.ensure_meeting_index(r).ok();
                }
                None
            })
            .collect()
    }

    /// Calcule l'embedding d'une requête ou question.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, String> {
        self.with_embedder(|e| e.embed_query(query).map_err(|err| err.to_string()))
    }

    fn with_embedder<R>(
        &self,
        f: impl FnOnce(&mut TextEmbedder) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut slot = lock(&self.embedder);
        if slot.is_none() {
            let models = model::semantic_local(&self.models_dir)
                .ok_or("modèle de recherche sémantique absent")?;
            let threads = thread::available_parallelism()
                .map(|n| (n.get() / 2).clamp(1, 4))
                .unwrap_or(2);
            let embedder =
                TextEmbedder::load(&models.model, &models.tokenizer, &models.runtime, threads)
                    .map_err(|e| e.to_string())?;
            *slot = Some(embedder);
        }
        f(slot.as_mut().expect("modèle chargé ci-dessus"))
    }
}

/// Supprime `semantique.bin` s'il existe (utile pour forcer une réindexation).
pub fn clear_disk_indices(meetings_dir: &Path) {
    for entry in std::fs::read_dir(meetings_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path().join(semantic::INDEX_FILE);
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}
