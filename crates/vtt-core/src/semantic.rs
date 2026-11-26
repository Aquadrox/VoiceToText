//! Index d'embeddings local, recherche hybride (mots-clés + sémantique) et
//! sélection enrichie des passages (RAG) pour « Poser une question ».
//!
//! Ce module ne dépend pas d'ONNX Runtime : l'inférence concrète vit dans
//! `vtt_asr::TextEmbedder` et implémente le trait [`Embedder`]. Toute la
//! logique de découpage, persistance compacte (`semantique.bin`), fusion de
//! scores (RRF) et construction du contexte LLM est portable et testée ici.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::Error;
use crate::meeting::{self, MeetingRecord, MeetingReport, Reply, timestamp};
use crate::search::{self, Parts};

/// Nom du fichier d'index sémantique stocké dans le dossier d'une réunion.
pub const INDEX_FILE: &str = "semantique.bin";

/// Identifiant du modèle inscrit dans l'index : changer de modèle invalide
/// automatiquement les anciens index.
pub const MODEL_ID: &str = "multilingual-e5-small-v1";

/// En-tête binaire de `semantique.bin`.
const MAGIC: &[u8; 8] = b"VTTSEM01";

/// Taille cible et plafond d'une fenêtre de transcription indexée (caractères).
const WINDOW_TARGET_CHARS: usize = 420;
const WINDOW_MAX_CHARS: usize = 650;

/// Nombre maximal de passages conservés par réunion (couvre > 4 h de parole).
const MAX_PASSAGES_PER_MEETING: usize = 600;

/// Longueur maximale du texte d'un passage stocké dans `semantique.bin`.
const MAX_PASSAGE_TEXT_BYTES: usize = 2_048;

/// Seuil minimal de similarité cosinus pour la recherche sémantique
/// (mesuré sur `multilingual-e5-small` en français : ~0,75–0,77 sans lien,
/// 0,786–0,86 sur des synonymes / reformulations pertinentes).
pub const DEFAULT_SEMANTIC_THRESHOLD: f32 = 0.785;

/// Écart maximal sous le meilleur score sémantique pour éviter le bruit quand
/// un passage correspond très fortement.
const SEMANTIC_MAX_DROP: f32 = 0.055;

/// Plancher légèrement plus souple pour la sélection de passages RAG (le LLM
/// filtre ensuite les détails utiles).
pub const RAG_SEMANTIC_FLOOR: f32 = 0.77;

/// Écart temporel sous lequel deux résultats de la même réunion sont fusionnés.
const DEDUP_WINDOW_MS: u64 = 12_000;

/// Constante RRF standard (Reciprocal Rank Fusion).
const RRF_K: f32 = 60.0;

/// Facteur de quantification Q15 pour vecteurs unitaires (`[-1, 1]` → `i16`).
const Q15_SCALE: f32 = 32_767.0;

/// Modèle d'embeddings de texte (implémenté par `vtt_asr::TextEmbedder`).
pub trait Embedder {
    /// Calcule le vecteur unitaire d'une requête ou question (`query: ...`).
    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, Error>;
    /// Calcule les vecteurs unitaires d'un lot de passages (`passage: ...`).
    fn embed_passages(&mut self, passages: &[&str]) -> Result<Vec<Vec<f32>>, Error>;
}

/// Passage extrait d'une réunion (fenêtre de transcription ou élément de
/// rapport complet), prêt à être indexé ou classé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub is_report: bool,
    pub section: Option<String>,
    pub start_ms: Option<u64>,
    /// Plage inclusive d'indices dans `meeting::turns(&record.segments())`.
    pub turn_start: Option<usize>,
    pub turn_end: Option<usize>,
    pub text: String,
}

/// Passage muni de son vecteur d'embedding quantifié sur 16 bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPassage {
    pub is_report: bool,
    pub section: Option<String>,
    pub start_ms: Option<u64>,
    pub turn_start: Option<usize>,
    pub turn_end: Option<usize>,
    pub text: String,
    pub vector: Vec<i16>,
}

/// Index vectoriel local d'une réunion (`semantique.bin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingIndex {
    pub model_id: String,
    pub content_hash: u64,
    pub dim: usize,
    pub passages: Vec<IndexedPassage>,
}

/// Nature d'une correspondance de recherche hybride.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Keyword,
    Hybrid,
    Semantic,
}

/// Résultat fusionné de la recherche hybride (mots-clés + sémantique).
#[derive(Debug, Clone, PartialEq)]
pub struct HybridHit {
    pub meeting_idx: usize,
    pub is_report: bool,
    pub section: Option<String>,
    pub start_ms: Option<u64>,
    pub parts: Parts,
    pub match_kind: MatchKind,
    /// Similarité cosinus (0..=1) quand le sens a contribué au résultat.
    pub semantic_score: Option<f32>,
    /// Score interne de classement (plus haut = plus pertinent).
    pub rank_score: f32,
}

/// Contexte assemblé pour la commande « Poser une question ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RagContext {
    pub context: String,
    pub sources: usize,
    pub used_semantic: bool,
}

/// Normalise `v` en norme L2 unitaire. Renvoie un vecteur nul si la norme est nulle.
pub fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON || !norm.is_finite() {
        return vec![0.0; v.len()];
    }
    v.iter().map(|x| x / norm).collect()
}

/// Quantifie un vecteur (normalisé L2) en entiers `i16` (`Q15`, erreur < 10⁻⁴).
pub fn quantize_unit(v: &[f32]) -> Vec<i16> {
    let unit = l2_normalize(v);
    unit.into_iter()
        .map(|x| (x.clamp(-1.0, 1.0) * Q15_SCALE).round() as i16)
        .collect()
}

/// Similarité cosinus entre un vecteur requête unitaire (`f32`) et un passage
/// quantifié (`Q15`). Renvoie `0.0` si les dimensions diffèrent ou sont vides.
pub fn cosine_q15(query_unit: &[f32], vec_q15: &[i16]) -> f32 {
    if query_unit.is_empty() || query_unit.len() != vec_q15.len() {
        return 0.0;
    }
    let dot: f32 = query_unit
        .iter()
        .zip(vec_q15.iter())
        .map(|(&q, &p)| q * f32::from(p))
        .sum();
    (dot / Q15_SCALE).clamp(-1.0, 1.0)
}

fn format_turn_text(turn: &Reply, names: &[String]) -> String {
    let text = turn.text.trim();
    if text.is_empty() {
        return String::new();
    }
    if turn.speaker.is_some() {
        format!(
            "{} : {text}",
            meeting::speaker_label(turn.track, turn.speaker, names)
        )
    } else {
        text.to_owned()
    }
}

fn push_report_passages(report: &MeetingReport, out: &mut Vec<Passage>) {
    let mut push = |section: &str, start_ms: Option<u64>, text: String| {
        let trimmed = text.trim();
        if trimmed.is_empty() || out.len() >= MAX_PASSAGES_PER_MEETING {
            return;
        }
        out.push(Passage {
            is_report: true,
            section: Some(section.to_owned()),
            start_ms,
            turn_start: None,
            turn_end: None,
            text: truncate_utf8(trimmed, MAX_PASSAGE_TEXT_BYTES).to_owned(),
        });
    };

    for para in report
        .synthesis
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        push("Synthèse", None, para.to_owned());
    }
    for point in &report.key_points {
        push("Point clé", point.at_ms, point.text.clone());
    }
    for chapter in &report.chapters {
        let start = Some(chapter.start_ms);
        let mut block = chapter.title.trim().to_owned();
        if !chapter.summary.trim().is_empty() {
            if !block.is_empty() {
                block.push_str(" — ");
            }
            block.push_str(chapter.summary.trim());
        }
        if !block.is_empty() {
            push("Chapitre", start, block);
        }
        for point in &chapter.points {
            push("Chapitre", point.at_ms.or(start), point.text.clone());
        }
        for decision in &chapter.decisions {
            push("Décision", decision.at_ms.or(start), decision.text.clone());
        }
        for quote in &chapter.quotes {
            push("Citation", quote.at_ms.or(start), quote.text.clone());
        }
    }
    for (section, points) in [
        ("Accord", &report.agreements),
        ("Désaccord", &report.disagreements),
        ("Risque", &report.risks),
        ("Chiffre", &report.figures),
        ("Question ouverte", &report.open_questions),
    ] {
        for point in points {
            push(section, point.at_ms, point.text.clone());
        }
    }
    for qa in &report.qa {
        let combined = if qa.answer.trim().is_empty() {
            qa.question.clone()
        } else {
            format!("{} — {}", qa.question.trim(), qa.answer.trim())
        };
        push("Question / Réponse", qa.at_ms, combined);
    }
    for action in &report.actions {
        let mut line = action.task.trim().to_owned();
        if !action.owner.trim().is_empty() {
            line.push_str(" (");
            line.push_str(action.owner.trim());
            if !action.due.trim().is_empty() {
                line.push_str(", ");
                line.push_str(action.due.trim());
            }
            line.push(')');
        } else if !action.due.trim().is_empty() {
            line.push_str(" (");
            line.push_str(action.due.trim());
            line.push(')');
        }
        push("Action", action.at_ms, line);
    }
}

/// Découpe une réunion en passages indexables : fenêtres glissantes de
/// répliques (avec recouvrement d'une réplique pour ne pas couper une réponse
/// de sa question) et sections du rapport complet s'il existe.
pub fn extract_passages(record: &MeetingRecord) -> Vec<Passage> {
    let mut out = Vec::new();
    let turns = meeting::turns(&record.segments());
    let names = record.names();

    let mut i = 0;
    while i < turns.len() && out.len() < MAX_PASSAGES_PER_MEETING {
        let start = i;
        let start_ms = turns[start].start_ms;
        let mut text = String::new();
        let mut end = start;
        while end < turns.len() {
            let line = format_turn_text(&turns[end], &names);
            if line.is_empty() {
                end += 1;
                continue;
            }
            let added = if text.is_empty() {
                line.len()
            } else {
                line.len() + 1
            };
            if !text.is_empty() && text.len() + added > WINDOW_MAX_CHARS {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&line);
            end += 1;
            if text.len() >= WINDOW_TARGET_CHARS {
                break;
            }
        }
        let last_turn = end.saturating_sub(1).max(start);
        if !text.is_empty() {
            out.push(Passage {
                is_report: false,
                section: None,
                start_ms: Some(start_ms),
                turn_start: Some(start),
                turn_end: Some(last_turn),
                text: truncate_utf8(&text, MAX_PASSAGE_TEXT_BYTES).to_owned(),
            });
        }
        let consumed = end.saturating_sub(start);
        if consumed <= 1 || end >= turns.len() {
            i = end.max(start + 1);
        } else {
            // Recouvrement d'une réplique entre fenêtres consécutives.
            i = end - 1;
        }
    }

    if let Some(report) = &record.report {
        push_report_passages(report, &mut out);
    }
    out
}

/// Empreinte FNV-1a 64 bits déterministe du contenu indexable d'une réunion.
/// Permet de savoir immédiatement si `semantique.bin` est toujours à jour.
pub fn content_hash(record: &MeetingRecord) -> u64 {
    let passages = extract_passages(record);
    hash_passages(&record.title, &passages)
}

fn hash_passages(title: &str, passages: &[Passage]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    mix(MODEL_ID.as_bytes());
    mix(&[0xff]);
    mix(title.as_bytes());
    mix(&[0xff]);
    for p in passages {
        mix(&[u8::from(p.is_report)]);
        if let Some(sec) = &p.section {
            mix(sec.as_bytes());
        }
        mix(&[0xfe]);
        if let Some(ms) = p.start_ms {
            mix(&ms.to_le_bytes());
        }
        mix(&[0xfd]);
        mix(p.text.as_bytes());
        mix(&[0xff]);
    }
    h
}

/// Construit l'index sémantique complet d'une réunion à l'aide d'un [`Embedder`].
pub fn build_meeting_index(
    record: &MeetingRecord,
    embedder: &mut dyn Embedder,
) -> Result<MeetingIndex, Error> {
    let passages = extract_passages(record);
    let content_hash = hash_passages(&record.title, &passages);
    if passages.is_empty() {
        return Ok(MeetingIndex {
            model_id: MODEL_ID.to_owned(),
            content_hash,
            dim: 0,
            passages: Vec::new(),
        });
    }

    const BATCH: usize = 8;
    let mut indexed = Vec::with_capacity(passages.len());
    let mut dim = 0usize;

    for chunk in passages.chunks(BATCH) {
        let texts: Vec<&str> = chunk.iter().map(|p| p.text.as_str()).collect();
        let vectors = embedder.embed_passages(&texts)?;
        if vectors.len() != chunk.len() {
            return Err(Error::Asr(format!(
                "embeddings : {} vecteurs reçus pour {} passages",
                vectors.len(),
                chunk.len()
            )));
        }
        for (p, vec) in chunk.iter().zip(vectors) {
            if dim == 0 {
                dim = vec.len();
            } else if vec.len() != dim {
                return Err(Error::Asr(format!(
                    "embeddings : dimension incohérente ({} != {dim})",
                    vec.len()
                )));
            }
            indexed.push(IndexedPassage {
                is_report: p.is_report,
                section: p.section.clone(),
                start_ms: p.start_ms,
                turn_start: p.turn_start,
                turn_end: p.turn_end,
                text: p.text.clone(),
                vector: quantize_unit(&vec),
            });
        }
    }

    Ok(MeetingIndex {
        model_id: MODEL_ID.to_owned(),
        content_hash,
        dim,
        passages: indexed,
    })
}

impl MeetingIndex {
    /// Vrai si cet index correspond toujours au modèle actif et au contenu de `record`.
    pub fn is_fresh_for(&self, record: &MeetingRecord) -> bool {
        self.model_id == MODEL_ID && self.content_hash == content_hash(record)
    }

    /// Sérialise l'index au format binaire compact `VTTSEM01`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.passages.len() * (self.dim * 2 + 64));
        out.extend_from_slice(MAGIC);
        write_str_u16(&mut out, &self.model_id);
        out.extend_from_slice(&self.content_hash.to_le_bytes());
        out.extend_from_slice(&(self.dim as u16).to_le_bytes());
        out.extend_from_slice(&(self.passages.len() as u32).to_le_bytes());

        for p in &self.passages {
            let mut flags = 0u8;
            if p.is_report {
                flags |= 1;
            }
            if p.start_ms.is_some() {
                flags |= 2;
            }
            if p.turn_start.is_some() && p.turn_end.is_some() {
                flags |= 4;
            }
            if p.section.is_some() {
                flags |= 8;
            }
            out.push(flags);
            if let Some(sec) = &p.section {
                write_str_u16(&mut out, sec);
            }
            if let Some(ms) = p.start_ms {
                out.extend_from_slice(&ms.to_le_bytes());
            }
            if let (Some(a), Some(b)) = (p.turn_start, p.turn_end) {
                out.extend_from_slice(&(a as u32).to_le_bytes());
                out.extend_from_slice(&(b as u32).to_le_bytes());
            }
            write_str_u32(&mut out, &p.text);
            for &v in &p.vector {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// Relit un index binaire `VTTSEM01` avec vérification stricte des bornes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = bytes;
        let magic = take(&mut cursor, 8)?;
        if magic != MAGIC {
            return Err("en-tête d'index sémantique inconnu".into());
        }
        let model_id = read_str_u16(&mut cursor)?;
        let content_hash = read_u64(&mut cursor)?;
        let dim = usize::from(read_u16(&mut cursor)?);
        let count = read_u32(&mut cursor)? as usize;
        if count > MAX_PASSAGES_PER_MEETING * 2 || dim > 4096 {
            return Err("dimensions d'index sémantique hors limites".into());
        }
        let mut passages = Vec::with_capacity(count);
        for _ in 0..count {
            let flags = take(&mut cursor, 1)?[0];
            let is_report = (flags & 1) != 0;
            let section = if (flags & 8) != 0 {
                Some(read_str_u16(&mut cursor)?)
            } else {
                None
            };
            let start_ms = if (flags & 2) != 0 {
                Some(read_u64(&mut cursor)?)
            } else {
                None
            };
            let (turn_start, turn_end) = if (flags & 4) != 0 {
                (
                    Some(read_u32(&mut cursor)? as usize),
                    Some(read_u32(&mut cursor)? as usize),
                )
            } else {
                (None, None)
            };
            let text = read_str_u32(&mut cursor)?;
            let raw_vec = take(&mut cursor, dim * 2)?;
            let mut vector = Vec::with_capacity(dim);
            for &pair in raw_vec.as_chunks::<2>().0 {
                vector.push(i16::from_le_bytes(pair));
            }
            passages.push(IndexedPassage {
                is_report,
                section,
                start_ms,
                turn_start,
                turn_end,
                text,
                vector,
            });
        }
        if !cursor.is_empty() {
            return Err("données superflues dans l'index sémantique".into());
        }
        Ok(Self {
            model_id,
            content_hash,
            dim,
            passages,
        })
    }
}

/// Enregistre `semantique.bin` de manière atomique dans le dossier `dir`.
pub fn save_index(dir: &Path, index: &MeetingIndex) -> Result<(), String> {
    let path = dir.join(INDEX_FILE);
    let tmp = dir.join(format!("{INDEX_FILE}.tmp"));
    fs::write(&tmp, index.to_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Charge `semantique.bin` depuis le dossier `dir`.
pub fn load_index(dir: &Path) -> Result<MeetingIndex, String> {
    let path = dir.join(INDEX_FILE);
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    MeetingIndex::from_bytes(&bytes)
}

/// Recherche hybride (mots-clés exacts + similarité sémantique par RRF) dans
/// toutes les réunions fournies.
///
/// Quand `query_vec` vaut `None`, se comporte exactement comme la recherche
/// par mots-clés (rapport complet + transcription).
pub fn hybrid_search(
    records: &[MeetingRecord],
    indices: &[Option<&MeetingIndex>],
    query: &str,
    query_vec: Option<&[f32]>,
    threshold: f32,
    max_results: usize,
) -> Vec<HybridHit> {
    let mut hits: Vec<HybridHit> = Vec::new();
    let mut kw_rank = 0usize;

    for (meeting_idx, record) in records.iter().enumerate() {
        if let Some(report) = &record.report {
            for hit in search::find_in_report(report, query) {
                kw_rank += 1;
                hits.push(HybridHit {
                    meeting_idx,
                    is_report: true,
                    section: Some(hit.section.to_owned()),
                    start_ms: hit.start_ms,
                    parts: hit.parts,
                    match_kind: MatchKind::Keyword,
                    semantic_score: None,
                    rank_score: 2.0 + 1.0 / (RRF_K + kw_rank as f32),
                });
                if hits.len() >= max_results && query_vec.is_none() {
                    return hits;
                }
            }
        }
        for hit in search::find(&record.segments(), query) {
            kw_rank += 1;
            hits.push(HybridHit {
                meeting_idx,
                is_report: false,
                section: None,
                start_ms: Some(hit.start_ms),
                parts: hit.parts,
                match_kind: MatchKind::Keyword,
                semantic_score: None,
                rank_score: 2.0 + 1.0 / (RRF_K + kw_rank as f32),
            });
            if hits.len() >= max_results && query_vec.is_none() {
                return hits;
            }
        }
    }

    let Some(raw_qvec) = query_vec else {
        hits.truncate(max_results);
        return hits;
    };
    let qvec = l2_normalize(raw_qvec);
    if qvec.is_empty() {
        hits.truncate(max_results);
        return hits;
    }

    // Score sémantique de tous les passages indexés.
    let mut sem_candidates: Vec<(f32, usize, &IndexedPassage)> = Vec::new();
    for (meeting_idx, opt_index) in indices.iter().enumerate() {
        if meeting_idx >= records.len() {
            break;
        }
        let Some(index) = opt_index else {
            continue;
        };
        if index.dim != qvec.len() {
            continue;
        }
        for p in &index.passages {
            let score = cosine_q15(&qvec, &p.vector);
            if score >= threshold {
                sem_candidates.push((score, meeting_idx, p));
            }
        }
    }
    sem_candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    if let Some(&(best, _, _)) = sem_candidates.first() {
        let floor = (best - SEMANTIC_MAX_DROP).max(threshold);
        sem_candidates.retain(|&(score, _, _)| score >= floor);
    }

    // Fusion RRF + dédoublonnage sur la même zone temporelle/section.
    for (sem_rank, (score, meeting_idx, passage)) in sem_candidates.into_iter().enumerate() {
        let rrf_boost = 1.0 / (RRF_K + (sem_rank + 1) as f32);
        if let Some(existing) = hits
            .iter_mut()
            .find(|h| is_same_spot(h, meeting_idx, passage))
        {
            if existing.match_kind == MatchKind::Keyword {
                existing.match_kind = MatchKind::Hybrid;
                existing.rank_score += 0.5 + rrf_boost;
            }
            existing.semantic_score = Some(existing.semantic_score.map_or(score, |s| s.max(score)));
            continue;
        }
        hits.push(HybridHit {
            meeting_idx,
            is_report: passage.is_report,
            section: passage.section.clone(),
            start_ms: passage.start_ms,
            parts: search::highlight_partial(&passage.text, query),
            match_kind: MatchKind::Semantic,
            semantic_score: Some(score),
            rank_score: score + rrf_boost,
        });
    }

    hits.sort_by(|a, b| b.rank_score.total_cmp(&a.rank_score));
    hits.truncate(max_results);
    hits
}

fn is_same_spot(hit: &HybridHit, meeting_idx: usize, passage: &IndexedPassage) -> bool {
    if hit.meeting_idx != meeting_idx || hit.is_report != passage.is_report {
        return false;
    }
    let hit_text: String = hit.parts.iter().map(|(s, _)| s.as_str()).collect();
    let text_overlaps = !hit_text.is_empty()
        && (hit_text.contains(passage.text.as_str()) || passage.text.contains(hit_text.as_str()));
    if hit.is_report {
        if hit.section != passage.section {
            return false;
        }
        return hit.start_ms == passage.start_ms || text_overlaps;
    }
    text_overlaps
        || match (hit.start_ms, passage.start_ms) {
            (Some(a), Some(b)) => a.abs_diff(b) <= DEDUP_WINDOW_MS,
            _ => false,
        }
}

/// Construit le contexte RAG pour « Poser une question » en combinant :
/// - classement lexical BM25-lite (mots significatifs + repli sigles courts) ;
/// - classement sémantique (si `query_vec` et index disponibles) fusionné par RRF ;
/// - diversité inter-réunions pour qu'une réunion longue n'éclipse pas les autres ;
/// - expansion des répliques voisines (`±1`) sans doublons ;
/// - inclusion des éléments pertinents du rapport complet (décisions, actions…).
pub fn build_rag_context(
    records: &[MeetingRecord],
    indices: &[Option<&MeetingIndex>],
    question: &str,
    query_vec: Option<&[f32]>,
    single_record: bool,
    max_chars: usize,
) -> RagContext {
    if records.is_empty() || max_chars == 0 {
        return RagContext {
            context: String::new(),
            sources: 0,
            used_semantic: false,
        };
    }

    // Un seul enregistrement qui tient entièrement dans la fenêtre : tout donner
    // (avec synthèse/décisions du rapport si ça rentre aussi).
    if single_record && records.len() == 1 {
        let record = &records[0];
        let segments = record.segments();
        let all = search::excerpts(&record.title, &segments, &[String::new()], &record.names());
        let mut full = String::new();
        if let Some(report) = &record.report
            && !report.synthesis.trim().is_empty()
        {
            full.push_str(&format!(
                "[{} · Rapport (Synthèse)] {}\n",
                record.title,
                report.synthesis.trim()
            ));
        }
        for line in &all {
            full.push_str(line);
            full.push('\n');
        }
        if !full.is_empty() && full.len() <= max_chars {
            return RagContext {
                context: full,
                sources: 1,
                used_semantic: false,
            };
        }
    }

    // Extraction des termes de la question (mots >= 4 lettres hors mots vides,
    // ou termes >= 2 caractères si la question ne contient que des sigles/mots courts).
    let mut query_terms = search::keywords(question);
    if query_terms.is_empty() {
        query_terms = search::terms(question);
    }
    query_terms.sort();
    query_terms.dedup();

    let qvec = query_vec.map(l2_normalize).filter(|v| !v.is_empty());

    struct Candidate {
        meeting_idx: usize,
        is_report: bool,
        section: Option<String>,
        start_ms: Option<u64>,
        turn_start: Option<usize>,
        turn_end: Option<usize>,
        text: String,
        folded: String,
        sem_score: Option<f32>,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (meeting_idx, record) in records.iter().enumerate() {
        let opt_index = indices.get(meeting_idx).copied().flatten();
        if let Some(index) = opt_index {
            for p in &index.passages {
                let sem_score = qvec
                    .as_deref()
                    .filter(|q| q.len() == index.dim)
                    .map(|q| cosine_q15(q, &p.vector));
                candidates.push(Candidate {
                    meeting_idx,
                    is_report: p.is_report,
                    section: p.section.clone(),
                    start_ms: p.start_ms,
                    turn_start: p.turn_start,
                    turn_end: p.turn_end,
                    folded: search::fold(&p.text),
                    text: p.text.clone(),
                    sem_score,
                });
            }
        } else {
            for p in extract_passages(record) {
                candidates.push(Candidate {
                    meeting_idx,
                    is_report: p.is_report,
                    section: p.section,
                    start_ms: p.start_ms,
                    turn_start: p.turn_start,
                    turn_end: p.turn_end,
                    folded: search::fold(&p.text),
                    text: p.text,
                    sem_score: None,
                });
            }
        }
    }

    if candidates.is_empty() {
        return RagContext {
            context: String::new(),
            sources: 0,
            used_semantic: false,
        };
    }

    // Document frequency pour pondération IDF (BM25-lite).
    let n_docs = candidates.len() as f32;
    let mut df = vec![0usize; query_terms.len()];
    for c in &candidates {
        for (ti, term) in query_terms.iter().enumerate() {
            if c.folded.contains(term.as_str()) {
                df[ti] += 1;
            }
        }
    }
    let idf: Vec<f32> = df
        .iter()
        .map(|&d| {
            ((n_docs - d as f32 + 0.5) / (d as f32 + 0.5) + 1.0)
                .ln()
                .max(0.1)
        })
        .collect();

    // Classement lexical.
    let mut lex_scored: Vec<(usize, f32)> = Vec::new();
    for (ci, c) in candidates.iter().enumerate() {
        let mut matched_terms = 0usize;
        let mut score = 0.0f32;
        for (ti, term) in query_terms.iter().enumerate() {
            let count = c.folded.matches(term.as_str()).count();
            if count > 0 {
                matched_terms += 1;
                let tf = (count as f32).min(4.0);
                let norm = 1.0 + 0.5 * (c.text.len() as f32 / WINDOW_TARGET_CHARS as f32);
                score += idf[ti] * (tf / (tf + norm));
            }
        }
        if matched_terms > 0 {
            // Bonus pour la couverture de plusieurs mots de la question.
            score *= 1.0 + 0.35 * (matched_terms as f32 / query_terms.len().max(1) as f32);
            lex_scored.push((ci, score));
        }
    }
    lex_scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Classement sémantique.
    let mut sem_scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(ci, c)| {
            c.sem_score
                .filter(|&s| s >= RAG_SEMANTIC_FLOOR)
                .map(|s| (ci, s))
        })
        .collect();
    sem_scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    let used_semantic = !sem_scored.is_empty();
    if lex_scored.is_empty() && sem_scored.is_empty() {
        return RagContext {
            context: String::new(),
            sources: 0,
            used_semantic: false,
        };
    }

    // Fusion RRF (Reciprocal Rank Fusion).
    let mut fused: HashMap<usize, f32> = HashMap::new();
    for (rank, &(ci, _)) in lex_scored.iter().enumerate() {
        *fused.entry(ci).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
    }
    for (rank, &(ci, sem_score)) in sem_scored.iter().enumerate() {
        let boost = if sem_score >= DEFAULT_SEMANTIC_THRESHOLD {
            1.15
        } else {
            0.85
        };
        *fused.entry(ci).or_insert(0.0) += boost / (RRF_K + (rank + 1) as f32);
    }

    let mut ranked: Vec<(usize, f32)> = fused.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Prépare les répliques par réunion pour l'expansion de voisinage (±1 réplique).
    let turns_by_meeting: Vec<Vec<Reply>> = records
        .iter()
        .map(|r| meeting::turns(&r.segments()))
        .collect();
    let names_by_meeting: Vec<Vec<String>> = records.iter().map(MeetingRecord::names).collect();

    // Sélection avec plafond par réunion au premier passage (diversité), puis
    // remplissage du budget restant au second passage.
    let distinct_meetings: HashSet<usize> = ranked
        .iter()
        .map(|&(ci, _)| candidates[ci].meeting_idx)
        .collect();
    let per_meeting_cap = if distinct_meetings.len() > 1 {
        (max_chars * 4 / 10).max(2_000)
    } else {
        max_chars
    };

    let mut chosen_turns: HashMap<usize, Vec<bool>> = HashMap::new();
    let mut chosen_reports: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut meeting_best_score: HashMap<usize, f32> = HashMap::new();
    let mut meeting_chars: HashMap<usize, usize> = HashMap::new();
    let mut total_chars = 0usize;

    let mut try_add = |ci: usize, score: f32, enforce_cap: bool| -> bool {
        let c = &candidates[ci];
        let m = c.meeting_idx;
        let used_m = *meeting_chars.get(&m).unwrap_or(&0);
        if enforce_cap && used_m >= per_meeting_cap {
            return total_chars >= max_chars;
        }
        if total_chars >= max_chars {
            return true;
        }
        if c.is_report {
            let list = chosen_reports.entry(m).or_default();
            if list.contains(&ci) {
                return false;
            }
            let est = records[m].title.len() + c.text.len() + 40;
            if total_chars + est > max_chars {
                return false;
            }
            list.push(ci);
            total_chars += est;
            *meeting_chars.entry(m).or_insert(0) += est;
            meeting_best_score
                .entry(m)
                .and_modify(|s| *s = s.max(score))
                .or_insert(score);
        } else if let (Some(ts), Some(te)) = (c.turn_start, c.turn_end) {
            let turns = &turns_by_meeting[m];
            if turns.is_empty() {
                return false;
            }
            let mask = chosen_turns
                .entry(m)
                .or_insert_with(|| vec![false; turns.len()]);
            let lo = ts.saturating_sub(1);
            let hi = (te + 1).min(turns.len() - 1);
            let mut added_chars = 0usize;
            for idx in lo..=hi {
                if !mask[idx] {
                    added_chars += records[m].title.len() + turns[idx].text.len() + 32;
                }
            }
            if added_chars == 0 || total_chars + added_chars > max_chars {
                return false;
            }
            for item in &mut mask[lo..=hi] {
                *item = true;
            }
            total_chars += added_chars;
            *meeting_chars.entry(m).or_insert(0) += added_chars;
            meeting_best_score
                .entry(m)
                .and_modify(|s| *s = s.max(score))
                .or_insert(score);
        }
        total_chars >= max_chars
    };

    let mut full = false;
    for &(ci, score) in &ranked {
        if try_add(ci, score, true) {
            full = true;
            break;
        }
    }
    if distinct_meetings.len() > 1 && !full {
        for &(ci, score) in &ranked {
            if try_add(ci, score, false) {
                break;
            }
        }
    }

    // Ordonner les réunions par pertinence décroissante, puis les passages dans
    // l'ordre chronologique à l'intérieur de chaque réunion.
    let mut meeting_order: Vec<(usize, f32)> = meeting_best_score.into_iter().collect();
    meeting_order.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut context = String::new();
    let mut sources = 0usize;

    for (m, _) in meeting_order {
        let record = &records[m];
        let mut meeting_added = false;

        if let Some(report_indices) = chosen_reports.get(&m) {
            for &ci in report_indices {
                let c = &candidates[ci];
                let sec = c.section.as_deref().unwrap_or("Rapport");
                let line = match c.start_ms {
                    Some(ms) => format!(
                        "[{} · Rapport ({sec}) · {}] {}",
                        record.title,
                        timestamp(ms),
                        c.text
                    ),
                    None => format!("[{} · Rapport ({sec})] {}", record.title, c.text),
                };
                if context.len() + line.len() + 1 > max_chars {
                    break;
                }
                context.push_str(&line);
                context.push('\n');
                meeting_added = true;
            }
        }

        if let Some(mask) = chosen_turns.get(&m) {
            let turns = &turns_by_meeting[m];
            let names = &names_by_meeting[m];
            for (idx, &keep) in mask.iter().enumerate() {
                if !keep {
                    continue;
                }
                let turn = &turns[idx];
                let body = format_turn_text(turn, names);
                if body.is_empty() {
                    continue;
                }
                let line = format!("[{} · {}] {body}", record.title, timestamp(turn.start_ms));
                if context.len() + line.len() + 1 > max_chars {
                    break;
                }
                context.push_str(&line);
                context.push('\n');
                meeting_added = true;
            }
        }

        if meeting_added {
            sources += 1;
        }
        if context.len() >= max_chars {
            break;
        }
    }

    RagContext {
        context,
        sources,
        used_semantic,
    }
}

fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn write_str_u16(out: &mut Vec<u8>, s: &str) {
    let bytes = truncate_utf8(s, u16::MAX as usize).as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn write_str_u32(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn take<'a>(cursor: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if cursor.len() < n {
        return Err("index sémantique tronqué".into());
    }
    let (head, rest) = cursor.split_at(n);
    *cursor = rest;
    Ok(head)
}

fn read_u16(cursor: &mut &[u8]) -> Result<u16, String> {
    let b = take(cursor, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32, String> {
    let b = take(cursor, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64(cursor: &mut &[u8]) -> Result<u64, String> {
    let b = take(cursor, 8)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn read_str_u16(cursor: &mut &[u8]) -> Result<String, String> {
    let len = usize::from(read_u16(cursor)?);
    let bytes = take(cursor, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "texte UTF-8 invalide dans l'index sémantique".into())
}

fn read_str_u32(cursor: &mut &[u8]) -> Result<String, String> {
    let len = read_u32(cursor)? as usize;
    if len > MAX_PASSAGE_TEXT_BYTES * 2 {
        return Err("passage trop long dans l'index sémantique".into());
    }
    let bytes = take(cursor, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "texte UTF-8 invalide dans l'index sémantique".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MeetingAudio;
    use crate::meeting::{MeetingKind, MeetingStatus, Segment, Track};

    fn seg(start_ms: u64, text: &str) -> Segment {
        Segment {
            track: Track::Eux,
            start_ms,
            end_ms: start_ms + 5_000,
            text: text.into(),
            copied: false,
            speaker: None,
        }
    }

    fn make_record(id: &str, title: &str, segments: Vec<Segment>) -> MeetingRecord {
        let mut r = MeetingRecord::new(
            id.into(),
            title.into(),
            1_700_000_000_000,
            MeetingKind::Reunion,
        );
        r.audio = MeetingAudio::Temporaire;
        r.status = MeetingStatus::Terminee;
        r.duration_ms = 300_000;
        r.segments = segments;
        r
    }

    /// Faux embedder sémantique à 4 thèmes : budget/coût, délai/livraison,
    /// recrutement/équipe, sécurité/chiffrement.
    struct TopicEmbedder;

    impl TopicEmbedder {
        fn vec_for(text: &str) -> Vec<f32> {
            let f = search::fold(text);
            let mut v = vec![0.05f32; 4];
            for kw in ["budget", "cout", "depense", "euro", "tarif", "financier"] {
                if f.contains(kw) {
                    v[0] += 1.0;
                }
            }
            for kw in [
                "delai",
                "retard",
                "livraison",
                "calendrier",
                "plann",
                "date",
            ] {
                if f.contains(kw) {
                    v[1] += 1.0;
                }
            }
            for kw in ["recrut", "poste", "candidat", "equipe", "embauch"] {
                if f.contains(kw) {
                    v[2] += 1.0;
                }
            }
            for kw in ["securit", "chiffr", "cle", "mot de passe", "prive"] {
                if f.contains(kw) {
                    v[3] += 1.0;
                }
            }
            l2_normalize(&v)
        }
    }

    impl Embedder for TopicEmbedder {
        fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, Error> {
            Ok(Self::vec_for(query))
        }

        fn embed_passages(&mut self, passages: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
            Ok(passages.iter().map(|p| Self::vec_for(p)).collect())
        }
    }

    #[test]
    fn q15_quantization_preserves_cosine_similarity() {
        let a = l2_normalize(&[0.3, -0.7, 0.2, 0.55, -0.1]);
        let b = l2_normalize(&[0.25, -0.65, 0.1, 0.6, 0.05]);
        let exact: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let approx = cosine_q15(&a, &quantize_unit(&b));
        assert!(
            (exact - approx).abs() < 1e-3,
            "exact={exact} approx={approx}"
        );
    }

    #[test]
    fn binary_roundtrip_and_freshness_hash() {
        let mut rec = make_record(
            "m1",
            "Comité produit",
            vec![
                seg(0, "Le budget marketing est validé à 5000 euros."),
                seg(10_000, "Nous recrutons deux développeurs."),
            ],
        );
        let mut embedder = TopicEmbedder;
        let idx = build_meeting_index(&rec, &mut embedder).unwrap();
        assert!(idx.is_fresh_for(&rec));

        let bytes = idx.to_bytes();
        let decoded = MeetingIndex::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, idx);

        // Un changement de texte invalide l'empreinte.
        rec.segments[0].text = "Le budget est doublé.".into();
        assert!(!decoded.is_fresh_for(&rec));

        // Fichier tronqué ou mauvais en-tête refusé proprement.
        assert!(MeetingIndex::from_bytes(b"NOPE").is_err());
        assert!(MeetingIndex::from_bytes(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn hybrid_search_finds_semantic_synonyms_and_boosts_hybrid_hits() {
        let rec1 = make_record(
            "m1",
            "Point finances",
            vec![
                seg(0, "Bonjour à tous."),
                seg(
                    20_000,
                    "Le budget de la campagne est fixé à cinq mille euros.",
                ),
            ],
        );
        let rec2 = make_record(
            "m2",
            "Point technique",
            vec![
                seg(0, "Nous avons un retard sur la livraison de la version."),
                seg(25_000, "Le chiffrement des clés est terminé."),
            ],
        );
        let mut embedder = TopicEmbedder;
        let idx1 = build_meeting_index(&rec1, &mut embedder).unwrap();
        let idx2 = build_meeting_index(&rec2, &mut embedder).unwrap();
        let records = [rec1, rec2];
        let indices = [Some(&idx1), Some(&idx2)];

        // Requête avec synonyme (« coût dépenses » ne figure pas mot pour mot dans « budget … euros »).
        let qvec = embedder.embed_query("coût et dépenses").unwrap();
        let hits = hybrid_search(&records, &indices, "coût dépenses", Some(&qvec), 0.80, 20);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_idx, 0);
        assert_eq!(hits[0].match_kind, MatchKind::Semantic);
        assert!(hits[0].semantic_score.unwrap() > 0.95);

        // Requête exacte + sémantique → promue en `Hybrid`.
        let qvec2 = embedder.embed_query("budget").unwrap();
        let hits2 = hybrid_search(&records, &indices, "budget", Some(&qvec2), 0.80, 20);
        assert_eq!(hits2[0].meeting_idx, 0);
        assert_eq!(hits2[0].match_kind, MatchKind::Hybrid);
    }

    #[test]
    fn rag_selects_semantic_passages_and_report_items_across_meetings() {
        let mut rec1 = make_record(
            "m1",
            "Comité finance",
            vec![
                seg(0, "Ouverture de la séance."),
                seg(10_000, "Le budget marketing est validé à 5000 euros."),
                seg(20_000, "Merci à tous."),
            ],
        );
        let mut report = MeetingReport::from_json(
            &serde_json::json!({
                "synthese": "Validation de l'enveloppe financière du trimestre.",
                "actions": [{"tache": "Envoyer le devis final", "responsable": "Julie", "t": "00:15"}]
            }),
            60_000,
        );
        report.finish(60_000);
        rec1.report = Some(report);

        let rec2 = make_record(
            "m2",
            "Point RH",
            vec![
                seg(0, "Deux postes ouverts pour le recrutement."),
                seg(10_000, "Les entretiens commencent lundi."),
            ],
        );
        let mut embedder = TopicEmbedder;
        let idx1 = build_meeting_index(&rec1, &mut embedder).unwrap();
        let idx2 = build_meeting_index(&rec2, &mut embedder).unwrap();
        let records = [rec1, rec2];
        let indices = [Some(&idx1), Some(&idx2)];

        // Question formulée sans le mot « budget » : trouvée grâce à la sémantique !
        let qvec = embedder
            .embed_query("Quelles dépenses ont été validées ?")
            .unwrap();
        let rag = build_rag_context(
            &records,
            &indices,
            "Quelles dépenses ont été validées ?",
            Some(&qvec),
            false,
            40_000,
        );
        assert!(rag.used_semantic);
        assert_eq!(rag.sources, 1);
        assert!(rag.context.contains("Comité finance"));
        assert!(rag.context.contains("5000 euros"));
        assert!(!rag.context.contains("Point RH"));
    }

    #[test]
    fn rag_without_semantic_ranks_lexical_and_handles_short_acronyms() {
        let rec1 = make_record("m1", "Bruit", vec![seg(0, "Bonjour, rien de spécial.")]);
        let rec2 = make_record(
            "m2",
            "Architecture",
            vec![
                seg(0, "Contexte avant."),
                seg(10_000, "L'API REST répond en 20 ms."),
                seg(20_000, "Contexte après."),
            ],
        );
        let rag = build_rag_context(&[rec1, rec2], &[None, None], "API ?", None, false, 40_000);
        assert!(!rag.used_semantic);
        assert_eq!(rag.sources, 1);
        assert!(rag.context.contains("Architecture"));
        assert!(rag.context.contains("Contexte avant."));
        assert!(rag.context.contains("L'API REST répond en 20 ms."));
        assert!(rag.context.contains("Contexte après."));
    }
}
