//! Embeddings de texte locaux via ONNX Runtime (`multilingual-e5-small`).
//!
//! Réutilise exactement le même `Environment` ONNX Runtime que Parakeet et la
//! diarisation ([`crate::parakeet::environment`]), avec un tokenizer Unigram
//! (SentencePiece) pur Rust lisant le `tokenizer.json` de Hugging Face — aucune
//! dépendance C++ ou crate de tokenisation supplémentaire.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use parakeet_rs::ort::session::Session;
use parakeet_rs::ort::value::Tensor;
use serde_json::Value;
use vtt_core::Error;
use vtt_core::semantic::{Embedder, l2_normalize};

use crate::parakeet::environment;

/// Longueur maximale d'une séquence acceptée par `multilingual-e5-small`.
const MAX_SEQ_LEN: usize = 512;

/// Caractère de début de mot SentencePiece (Metaspace U+2581).
const METASPACE: char = '\u{2581}';

fn ort_error(context: &str) -> impl Fn(parakeet_rs::ort::Error) -> Error + '_ {
    move |e| Error::Asr(format!("{context} : {e}"))
}

/// Tokenizer Unigram (SentencePiece / XLM-RoBERTa) chargé depuis `tokenizer.json`.
pub(crate) struct UnigramTokenizer {
    vocab: HashMap<Box<str>, (i64, f32)>,
    max_piece_bytes: usize,
    bos_id: i64,
    pad_id: i64,
    eos_id: i64,
    unk_id: i64,
    unk_penalty: f32,
}

impl UnigramTokenizer {
    pub(crate) fn from_file(path: &Path) -> Result<Self, Error> {
        let raw = fs::read_to_string(path)
            .map_err(|e| Error::Asr(format!("tokenizer {} : {e}", path.display())))?;
        Self::from_json_str(&raw)
    }

    pub(crate) fn from_json_str(raw: &str) -> Result<Self, Error> {
        let root: Value = serde_json::from_str(raw)
            .map_err(|e| Error::Asr(format!("tokenizer.json invalide : {e}")))?;

        let mut bos_id = 0i64;
        let mut pad_id = 1i64;
        let mut eos_id = 2i64;
        let mut unk_id = root
            .pointer("/model/unk_id")
            .and_then(Value::as_i64)
            .unwrap_or(3);

        let mut special_ids = vec![bos_id, pad_id, eos_id, unk_id];
        if let Some(added) = root.get("added_tokens").and_then(Value::as_array) {
            for tok in added {
                let Some(id) = tok.get("id").and_then(Value::as_i64) else {
                    continue;
                };
                let content = tok.get("content").and_then(Value::as_str).unwrap_or("");
                match content {
                    "<s>" | "[CLS]" => bos_id = id,
                    "<pad>" | "[PAD]" => pad_id = id,
                    "</s>" | "[SEP]" => eos_id = id,
                    "<unk>" | "[UNK]" => unk_id = id,
                    _ => {}
                }
                if tok.get("special").and_then(Value::as_bool).unwrap_or(false) {
                    special_ids.push(id);
                }
            }
        }

        let entries = root
            .pointer("/model/vocab")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Asr("tokenizer.json : champ model.vocab absent".into()))?;

        let mut vocab = HashMap::with_capacity(entries.len());
        let mut max_piece_bytes = 0usize;
        let mut min_score = 0.0f32;

        for (idx, item) in entries.iter().enumerate() {
            let id = idx as i64;
            if special_ids.contains(&id) {
                continue;
            }
            let Some(pair) = item.as_array() else {
                continue;
            };
            let (Some(piece), Some(score)) = (
                pair.first().and_then(Value::as_str),
                pair.get(1).and_then(Value::as_f64),
            ) else {
                continue;
            };
            if piece.is_empty() {
                continue;
            }
            let score = score as f32;
            if score < min_score {
                min_score = score;
            }
            if piece.len() > max_piece_bytes {
                max_piece_bytes = piece.len();
            }
            vocab.entry(piece.into()).or_insert((id, score));
        }

        if vocab.is_empty() {
            return Err(Error::Asr("tokenizer.json : vocabulaire vide".into()));
        }

        Ok(Self {
            vocab,
            max_piece_bytes,
            bos_id,
            pad_id,
            eos_id,
            unk_id,
            unk_penalty: min_score - 10.0,
        })
    }

    /// Normalisation SentencePiece Metaspace : espaces repliés, remplacés par `▁`
    /// avec préfixe `▁`.
    fn preprocess(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 4);
        let mut in_space = true;
        for c in text.chars() {
            if c.is_whitespace() {
                in_space = true;
            } else {
                if in_space {
                    out.push(METASPACE);
                    in_space = false;
                }
                out.push(c);
            }
        }
        out
    }

    /// Encode `text` en identifiants `[<s>, ..., </s>]` bornés à `max_len`.
    pub(crate) fn encode(&self, text: &str, max_len: usize) -> Vec<i64> {
        let max_body = max_len.saturating_sub(2);
        let normalized = Self::preprocess(text);
        if normalized.is_empty() || max_body == 0 {
            return vec![self.bos_id, self.eos_id];
        }

        let mut bounds: Vec<usize> = normalized.char_indices().map(|(i, _)| i).collect();
        bounds.push(normalized.len());
        let n = bounds.len() - 1;

        let mut best_score = vec![f32::NEG_INFINITY; n + 1];
        let mut prev_idx = vec![0usize; n + 1];
        let mut prev_id = vec![self.unk_id; n + 1];
        best_score[0] = 0.0;

        for i in 0..n {
            let base = best_score[i];
            if !base.is_finite() {
                continue;
            }
            let start = bounds[i];
            let mut matched_single = false;
            for j in (i + 1)..=n {
                let end = bounds[j];
                if end - start > self.max_piece_bytes {
                    break;
                }
                if let Some(&(id, score)) = self.vocab.get(&normalized[start..end]) {
                    if j == i + 1 {
                        matched_single = true;
                    }
                    let cand = base + score;
                    if cand > best_score[j] {
                        best_score[j] = cand;
                        prev_idx[j] = i;
                        prev_id[j] = id;
                    }
                }
            }
            if !matched_single {
                let cand = base + self.unk_penalty;
                if cand > best_score[i + 1] {
                    best_score[i + 1] = cand;
                    prev_idx[i + 1] = i;
                    prev_id[i + 1] = self.unk_id;
                }
            }
        }

        let mut pieces = Vec::new();
        let mut pos = n;
        while pos > 0 {
            let id = prev_id[pos];
            // SentencePiece fusionne les caractères inconnus consécutifs en un seul <unk>.
            if !(id == self.unk_id && pieces.last() == Some(&self.unk_id)) {
                pieces.push(id);
            }
            pos = prev_idx[pos];
        }
        pieces.reverse();
        pieces.truncate(max_body);

        let mut ids = Vec::with_capacity(pieces.len() + 2);
        ids.push(self.bos_id);
        ids.extend_from_slice(&pieces);
        ids.push(self.eos_id);
        ids
    }
}

/// Encodeur d'embeddings de texte multilingue sur CPU via ONNX Runtime.
pub struct TextEmbedder {
    session: Session,
    tokenizer: UnigramTokenizer,
    has_token_type_ids: bool,
}

impl TextEmbedder {
    /// Charge `model` (`model.onnx`) et `tokenizer` (`tokenizer.json`) en
    /// réutilisant la bibliothèque `runtime` (`onnxruntime`).
    pub fn load(
        model: &Path,
        tokenizer: &Path,
        runtime: &Path,
        threads: usize,
    ) -> Result<Self, Error> {
        let started = Instant::now();
        let environment = environment(runtime)?;
        let threads = threads.max(1);
        let context = model.display().to_string();
        let session = Session::builder(&environment)
            .map_err(ort_error(&context))?
            .with_intra_threads(threads)
            .map_err(|e| Error::Asr(format!("{context} : {e}")))?
            .commit_from_file(model)
            .map_err(ort_error(&context))?;
        let has_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let tokenizer = UnigramTokenizer::from_file(tokenizer)?;
        log::info!(
            "embeddings : modèle chargé en {} ms ({} entrées, {} pièces)",
            started.elapsed().as_millis(),
            session.inputs().len(),
            tokenizer.vocab.len()
        );
        Ok(Self {
            session,
            tokenizer,
            has_token_type_ids,
        })
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, Error> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encoded: Vec<Vec<i64>> = texts
            .iter()
            .map(|t| self.tokenizer.encode(t, MAX_SEQ_LEN))
            .collect();
        let batch = encoded.len();
        let seq_len = encoded.iter().map(Vec::len).max().unwrap_or(2);

        let mut input_ids = vec![self.tokenizer.pad_id; batch * seq_len];
        let mut attention_mask = vec![0i64; batch * seq_len];
        for (b, seq) in encoded.iter().enumerate() {
            let offset = b * seq_len;
            input_ids[offset..offset + seq.len()].copy_from_slice(seq);
            attention_mask[offset..offset + seq.len()].fill(1);
        }

        let ids_tensor = Tensor::from_array(([batch, seq_len], input_ids))
            .map_err(ort_error("embeddings input_ids"))?;
        let mask_tensor = Tensor::from_array(([batch, seq_len], attention_mask.clone()))
            .map_err(ort_error("embeddings attention_mask"))?;

        let outputs = if self.has_token_type_ids {
            let type_ids = Tensor::from_array(([batch, seq_len], vec![0i64; batch * seq_len]))
                .map_err(ort_error("embeddings token_type_ids"))?;
            self.session
                .run(parakeet_rs::ort::inputs![
                    "input_ids" => ids_tensor,
                    "attention_mask" => mask_tensor,
                    "token_type_ids" => type_ids,
                ])
                .map_err(ort_error("embeddings"))?
        } else {
            self.session
                .run(parakeet_rs::ort::inputs![
                    "input_ids" => ids_tensor,
                    "attention_mask" => mask_tensor,
                ])
                .map_err(ort_error("embeddings"))?
        };

        let (shape, hidden) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_error("embeddings sortie"))?;
        if shape.len() != 3 || shape[0] as usize != batch || shape[1] as usize != seq_len {
            return Err(Error::Asr(format!(
                "embeddings : sortie {shape:?} inattendue (attendu [{batch}, {seq_len}, D])"
            )));
        }
        let dim = shape[2] as usize;
        if dim == 0 {
            return Err(Error::Asr("embeddings : dimension de sortie nulle".into()));
        }

        // Mean pooling pondéré par attention_mask puis normalisation L2.
        let mut results = Vec::with_capacity(batch);
        for b in 0..batch {
            let mut sum = vec![0.0f32; dim];
            let mut count = 0.0f32;
            for t in 0..seq_len {
                if attention_mask[b * seq_len + t] == 0 {
                    continue;
                }
                count += 1.0;
                let base = (b * seq_len + t) * dim;
                let slice = &hidden[base..base + dim];
                for (acc, &val) in sum.iter_mut().zip(slice) {
                    *acc += val;
                }
            }
            if count > 0.0 {
                for v in &mut sum {
                    *v /= count;
                }
            }
            results.push(l2_normalize(&sum));
        }
        Ok(results)
    }
}

impl Embedder for TextEmbedder {
    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, Error> {
        let prefixed = vec![format!("query: {}", query.trim())];
        let mut out = self.embed_batch(&prefixed)?;
        out.pop()
            .ok_or_else(|| Error::Asr("embeddings : aucun vecteur produit".into()))
    }

    fn embed_passages(&mut self, passages: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        let prefixed: Vec<String> = passages
            .iter()
            .map(|p| format!("passage: {}", p.trim()))
            .collect();
        self.embed_batch(&prefixed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mini_tokenizer() -> UnigramTokenizer {
        let json = r#"{
            "added_tokens": [
                {"id": 0, "content": "<s>", "special": true},
                {"id": 1, "content": "<pad>", "special": true},
                {"id": 2, "content": "</s>", "special": true},
                {"id": 3, "content": "<unk>", "special": true}
            ],
            "model": {
                "type": "Unigram",
                "unk_id": 3,
                "vocab": [
                    ["<s>", 0.0],
                    ["<pad>", 0.0],
                    ["</s>", 0.0],
                    ["<unk>", 0.0],
                    ["▁", -4.0],
                    ["▁budget", -2.0],
                    ["▁bud", -5.0],
                    ["get", -5.0],
                    ["▁validé", -2.5],
                    ["▁de", -3.0],
                    ["e", -6.0]
                ]
            }
        }"#;
        UnigramTokenizer::from_json_str(json).unwrap()
    }

    #[test]
    fn unigram_viterbi_picks_highest_score_and_collapses_unk() {
        let tok = mini_tokenizer();
        // « budget » entier (-2.0) bat « bud » + « get » (-10.0).
        assert_eq!(tok.encode("  budget\t validé ", 32), vec![0, 5, 8, 2]);
        // Caractères inconnus consécutifs regroupés en un seul <unk> (id 3).
        assert_eq!(tok.encode("xyz budget", 32), vec![0, 4, 3, 5, 2]);
        // Troncature respecte <s> ... </s>.
        assert_eq!(tok.encode("budget validé", 3), vec![0, 5, 2]);
    }
}
