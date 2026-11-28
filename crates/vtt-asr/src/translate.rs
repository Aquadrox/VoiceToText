//! Traduction locale hors-ligne via ONNX Runtime (`Xenova/m2m100_418M`, MIT).
//!
//! Réutilise exactement le même `Environment` ONNX Runtime que Parakeet, la
//! diarisation et la recherche sémantique ([`crate::parakeet::environment`]),
//! avec un tokenizer BPE SentencePiece pur Rust lisant le `tokenizer.json` de
//! Hugging Face — aucune dépendance C++ ni crate supplémentaire.
//!
//! Les sessions ONNX sont chargées paresseusement au premier appel à
//! [`M2m100Translator::translate`] et libérées par [`Translator::unload`] afin
//! de ne pas occuper de mémoire vive hors mode traduction.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use parakeet_rs::ort::session::{Session, SessionInputValue};
use parakeet_rs::ort::value::{DynValue, Tensor};
use serde_json::Value;
use vtt_core::config::m2m100_lang_code;
use vtt_core::{Error, Translator};

use crate::parakeet::environment;

/// Caractère de début de mot SentencePiece (Metaspace U+2581).
const METASPACE: char = '\u{2581}';

/// Paramètres d'architecture de `M2M100-418M`.
const NUM_LAYERS: usize = 12;
const NUM_HEADS: usize = 16;
const HEAD_DIM: usize = 64;

/// Plafond de jetons source par segment (au-delà, on découpe aux phrases).
const MAX_SRC_TOKENS: usize = 180;

/// Plafond absolu de jetons générés par segment (M2M100 est entraîné jusqu'à 200).
const MAX_GEN_TOKENS: usize = 200;

fn ort_error(context: &str) -> impl Fn(parakeet_rs::ort::Error) -> Error + '_ {
    move |e| Error::PostProcess(format!("traduction locale ({context}) : {e}"))
}

/// Tokenizer BPE SentencePiece de M2M100, chargé depuis `tokenizer.json`.
pub(crate) struct M2m100Tokenizer {
    vocab: HashMap<Box<str>, i64>,
    id_to_token: Vec<Box<str>>,
    /// Paire `"a b"` → rang de fusion (plus petit = fusionné en premier).
    merges: HashMap<Box<str>, u32>,
    /// Code langue ISO (ex. `"fr"`, `"en"`, `"ja"`) → identifiant `__xx__`.
    lang_ids: HashMap<Box<str>, i64>,
    bos_id: i64,
    pad_id: i64,
    eos_id: i64,
    unk_id: i64,
}

impl M2m100Tokenizer {
    pub(crate) fn from_file(path: &Path) -> Result<Self, Error> {
        let raw = fs::read_to_string(path).map_err(|e| {
            Error::PostProcess(format!("tokenizer M2M100 {} : {e}", path.display()))
        })?;
        Self::from_json_str(&raw)
    }

    pub(crate) fn from_json_str(raw: &str) -> Result<Self, Error> {
        let root: Value = serde_json::from_str(raw)
            .map_err(|e| Error::PostProcess(format!("tokenizer.json M2M100 invalide : {e}")))?;

        let vocab_obj = root
            .pointer("/model/vocab")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::PostProcess("tokenizer.json : model.vocab absent".into()))?;

        let mut max_id = 0usize;
        let mut vocab: HashMap<Box<str>, i64> = HashMap::with_capacity(vocab_obj.len());
        for (piece, id_val) in vocab_obj {
            let Some(id) = id_val.as_i64() else {
                continue;
            };
            if id >= 0 {
                max_id = max_id.max(id as usize);
                vocab.insert(piece.as_str().into(), id);
            }
        }

        let mut bos_id = 0i64;
        let mut pad_id = 1i64;
        let mut eos_id = 2i64;
        let mut unk_id = 3i64;
        let mut lang_ids = HashMap::new();

        if let Some(added) = root.get("added_tokens").and_then(Value::as_array) {
            for tok in added {
                let (Some(id), Some(content)) = (
                    tok.get("id").and_then(Value::as_i64),
                    tok.get("content").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if id >= 0 {
                    max_id = max_id.max(id as usize);
                }
                match content {
                    "<s>" => bos_id = id,
                    "<pad>" => pad_id = id,
                    "</s>" => eos_id = id,
                    "<unk>" => unk_id = id,
                    _ => {
                        if let Some(code) = content
                            .strip_prefix("__")
                            .and_then(|s| s.strip_suffix("__"))
                            && !code.is_empty()
                        {
                            lang_ids.insert(code.into(), id);
                        }
                    }
                }
            }
        }

        let mut id_to_token: Vec<Box<str>> = vec!["".into(); max_id + 1];
        for (piece, &id) in &vocab {
            if let Some(slot) = id_to_token.get_mut(id as usize) {
                *slot = piece.clone();
            }
        }

        let merges_arr = root
            .pointer("/model/merges")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::PostProcess("tokenizer.json : model.merges absent".into()))?;
        let mut merges = HashMap::with_capacity(merges_arr.len());
        for (rank, entry) in merges_arr.iter().enumerate() {
            if let Some(pair) = entry.as_str() {
                merges.entry(pair.into()).or_insert(rank as u32);
            } else if let Some(pair) = entry.as_array()
                && pair.len() == 2
                && let (Some(a), Some(b)) = (pair[0].as_str(), pair[1].as_str())
            {
                let key = format!("{a} {b}");
                merges.entry(key.into()).or_insert(rank as u32);
            }
        }

        if vocab.is_empty() || lang_ids.is_empty() {
            return Err(Error::PostProcess(
                "tokenizer.json M2M100 incomplet (vocabulaire ou langues absents)".into(),
            ));
        }

        Ok(Self {
            vocab,
            id_to_token,
            merges,
            lang_ids,
            bos_id,
            pad_id,
            eos_id,
            unk_id,
        })
    }

    pub(crate) fn lang_id(&self, code: &str) -> Option<i64> {
        self.lang_ids.get(code).copied()
    }

    /// Encode un texte sans les jetons de langue ni `</s>`.
    pub(crate) fn encode_pieces(&self, text: &str) -> Vec<i64> {
        let mut ids = Vec::new();
        let mut key_buf = String::with_capacity(64);
        for word in text.split_whitespace() {
            let mut symbols: Vec<String> = Vec::with_capacity(word.len() + 1);
            symbols.push(METASPACE.to_string());
            for ch in word.chars() {
                symbols.push(ch.to_string());
            }
            // Fusionne d'abord `▁` + premier caractère si présent dans `merges`,
            // puis applique l'algorithme BPE standard par rang minimal.
            while symbols.len() >= 2 {
                let mut best_rank = u32::MAX;
                let mut best_idx = usize::MAX;
                for i in 0..symbols.len() - 1 {
                    key_buf.clear();
                    key_buf.push_str(&symbols[i]);
                    key_buf.push(' ');
                    key_buf.push_str(&symbols[i + 1]);
                    if let Some(&rank) = self.merges.get(key_buf.as_str())
                        && rank < best_rank
                    {
                        best_rank = rank;
                        best_idx = i;
                    }
                }
                if best_idx == usize::MAX {
                    break;
                }
                let merged_a = symbols[best_idx].clone();
                let merged_b = symbols[best_idx + 1].clone();
                let mut next = Vec::with_capacity(symbols.len() - 1);
                let mut i = 0;
                while i < symbols.len() {
                    if i + 1 < symbols.len() && symbols[i] == merged_a && symbols[i + 1] == merged_b
                    {
                        let mut combined = String::with_capacity(merged_a.len() + merged_b.len());
                        combined.push_str(&merged_a);
                        combined.push_str(&merged_b);
                        next.push(combined);
                        i += 2;
                    } else {
                        next.push(std::mem::take(&mut symbols[i]));
                        i += 1;
                    }
                }
                symbols = next;
            }
            for sym in symbols {
                let id = self.vocab.get(sym.as_str()).copied().unwrap_or(self.unk_id);
                ids.push(id);
            }
        }
        ids
    }

    /// Encode une séquence prête pour l'encodeur M2M100 : `[__src__, ..., </s>]`.
    pub(crate) fn encode(&self, text: &str, src_lang_id: i64, max_tokens: usize) -> Vec<i64> {
        let mut ids = self.encode_pieces(text);
        if ids.len() > max_tokens {
            ids.truncate(max_tokens);
        }
        let mut out = Vec::with_capacity(ids.len() + 2);
        out.push(src_lang_id);
        out.extend(ids);
        out.push(self.eos_id);
        out
    }

    /// Décode les identifiants générés en texte UTF-8 propre.
    pub(crate) fn decode(&self, ids: &[i64]) -> String {
        let mut raw = String::new();
        for &id in ids {
            if id == self.bos_id || id == self.pad_id || id == self.eos_id || id < 0 {
                continue;
            }
            // Les jetons de langue (`__xx__`) ne sont pas dans `id_to_token`
            // (chaîne vide) et sont donc naturellement ignorés.
            if let Some(piece) = self.id_to_token.get(id as usize)
                && !piece.is_empty()
            {
                raw.push_str(piece);
            }
        }
        let replaced = raw.replace(METASPACE, " ");
        replaced.trim().to_owned()
    }
}

struct M2m100Sessions {
    encoder: Session,
    decoder: Session,
    tokenizer: M2m100Tokenizer,
}

/// Traducteur local multilingue M2M100 (100 langues, ONNX Runtime).
pub struct M2m100Translator {
    encoder_path: PathBuf,
    decoder_path: PathBuf,
    tokenizer_path: PathBuf,
    runtime_path: PathBuf,
    threads: usize,
    sessions: Mutex<Option<M2m100Sessions>>,
}

impl M2m100Translator {
    /// Prépare un traducteur paresseux : les sessions ONNX ne sont chargées
    /// qu'au premier appel à [`Translator::translate`].
    pub fn new(
        encoder: PathBuf,
        decoder: PathBuf,
        tokenizer: PathBuf,
        runtime: PathBuf,
        threads: usize,
    ) -> Self {
        Self {
            encoder_path: encoder,
            decoder_path: decoder,
            tokenizer_path: tokenizer,
            runtime_path: runtime,
            threads: threads.max(1),
            sessions: Mutex::new(None),
        }
    }

    fn load_sessions(&self) -> Result<M2m100Sessions, Error> {
        let started = Instant::now();
        let env = environment(&self.runtime_path)
            .map_err(|e| Error::PostProcess(format!("traduction locale : {e}")))?;

        let encoder = Session::builder(&env)
            .map_err(ort_error("encodeur"))?
            .with_intra_threads(self.threads)
            .map_err(|e| Error::PostProcess(format!("traduction locale (encodeur) : {e}")))?
            .commit_from_file(&self.encoder_path)
            .map_err(ort_error("encodeur"))?;

        let decoder = Session::builder(&env)
            .map_err(ort_error("décodeur"))?
            .with_intra_threads(self.threads)
            .map_err(|e| Error::PostProcess(format!("traduction locale (décodeur) : {e}")))?
            .commit_from_file(&self.decoder_path)
            .map_err(ort_error("décodeur"))?;

        let tokenizer = M2m100Tokenizer::from_file(&self.tokenizer_path)?;

        log::info!(
            "traduction locale M2M100 chargée en {} ms ({} langues, {} threads)",
            started.elapsed().as_millis(),
            tokenizer.lang_ids.len(),
            self.threads
        );

        Ok(M2m100Sessions {
            encoder,
            decoder,
            tokenizer,
        })
    }
}

impl Translator for M2m100Translator {
    fn translate(&self, text: &str, source_lang: &str, target_lang: &str) -> Result<String, Error> {
        if text.trim().is_empty() {
            return Ok(String::new());
        }
        let src_code = m2m100_lang_code(source_lang).unwrap_or("fr");
        let tgt_code = m2m100_lang_code(target_lang).ok_or_else(|| {
            Error::PostProcess(format!(
                "traduction locale M2M100 : langue « {target_lang} » non reconnue (utilisez un nom courant ou un code ISO à 2 lettres)"
            ))
        })?;
        if src_code == tgt_code {
            return Ok(text.to_owned());
        }

        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(self.load_sessions()?);
        }
        let sessions = guard.as_mut().expect("sessions initialisées");
        let src_id = sessions.tokenizer.lang_id(src_code).ok_or_else(|| {
            Error::PostProcess(format!(
                "traduction locale M2M100 : code source « {src_code} » absent du modèle"
            ))
        })?;
        let tgt_id = sessions.tokenizer.lang_id(tgt_code).ok_or_else(|| {
            Error::PostProcess(format!(
                "traduction locale M2M100 : code cible « {tgt_code} » absent du modèle"
            ))
        })?;

        let started = Instant::now();
        // Préserve les retours à la ligne (commandes vocales « nouvelle ligne »).
        let mut out_lines = Vec::new();
        for line in text.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                out_lines.push(String::new());
                continue;
            }
            let segments = split_for_m2m100(trimmed, &sessions.tokenizer);
            let mut translated_parts = Vec::with_capacity(segments.len());
            for seg in segments {
                let part = translate_segment(sessions, &seg, src_id, tgt_id)?;
                if !part.is_empty() {
                    translated_parts.push(part);
                }
            }
            out_lines.push(translated_parts.join(" "));
        }
        let result = out_lines.join("\n");
        log::info!(
            "traduction locale M2M100 ({src_code} → {tgt_code}) : {} ms",
            started.elapsed().as_millis()
        );
        Ok(result)
    }

    fn unload(&self) -> bool {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let loaded = guard.is_some();
        *guard = None;
        if loaded {
            log::info!("traduction locale M2M100 déchargée de la mémoire");
        }
        loaded
    }
}

/// Découpe un paragraphe trop long aux frontières de phrases afin de ne jamais
/// tronquer une dictée longue à la limite de 200 jetons de M2M100.
fn split_for_m2m100(text: &str, tokenizer: &M2m100Tokenizer) -> Vec<String> {
    if tokenizer.encode_pieces(text).len() <= MAX_SRC_TOKENS {
        return vec![text.to_owned()];
    }
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?' | ';') {
            let next = i + ch.len_utf8();
            if next == bytes.len() || text[next..].starts_with(char::is_whitespace) {
                let s = text[start..next].trim();
                if !s.is_empty() {
                    sentences.push(s.to_owned());
                }
                start = next;
            }
        }
    }
    if start < text.len() {
        let rest = text[start..].trim();
        if !rest.is_empty() {
            sentences.push(rest.to_owned());
        }
    }
    if sentences.is_empty() {
        return vec![text.to_owned()];
    }

    // Regroupe plusieurs phrases courtes jusqu'à `MAX_SRC_TOKENS`.
    let mut chunks = Vec::new();
    let mut current = String::new();
    for sent in sentences {
        let candidate = if current.is_empty() {
            sent.clone()
        } else {
            format!("{current} {sent}")
        };
        if !current.is_empty() && tokenizer.encode_pieces(&candidate).len() > MAX_SRC_TOKENS {
            chunks.push(std::mem::take(&mut current));
            current = sent;
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn translate_segment(
    sessions: &mut M2m100Sessions,
    text: &str,
    src_id: i64,
    tgt_id: i64,
) -> Result<String, Error> {
    let src_ids = sessions.tokenizer.encode(text, src_id, MAX_SRC_TOKENS);
    let src_len = src_ids.len();
    let attn_vec = vec![1i64; src_len];

    // 1. Passe encodeur.
    let enc_ids = Tensor::from_array(([1usize, src_len], src_ids))
        .map_err(ort_error("encodeur input_ids"))?;
    let enc_mask = Tensor::from_array(([1usize, src_len], attn_vec.clone()))
        .map_err(ort_error("encodeur attention_mask"))?;
    let mut enc_outputs = sessions
        .encoder
        .run(parakeet_rs::ort::inputs![
            "input_ids" => enc_ids,
            "attention_mask" => enc_mask,
        ])
        .map_err(ort_error("encodeur run"))?;
    let encoder_hidden = enc_outputs
        .remove("last_hidden_state")
        .ok_or_else(|| Error::PostProcess("M2M100 : sortie last_hidden_state absente".into()))?;
    drop(enc_outputs);

    let enc_attn_mask: DynValue = Tensor::from_array(([1usize, src_len], attn_vec))
        .map_err(ort_error("décodeur encoder_attention_mask"))?
        .into();

    // 2. Étape 0 du décodeur (`use_cache_branch = false`, entrée `[</s>, __tgt__]`).
    let eos_id = sessions.tokenizer.eos_id;
    let pad_id = sessions.tokenizer.pad_id;
    let bos_id = sessions.tokenizer.bos_id;
    let unk_id = sessions.tokenizer.unk_id;

    let mut step0_inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
        Vec::with_capacity(4 + NUM_LAYERS * 4);
    step0_inputs.push((
        Cow::Borrowed("encoder_attention_mask"),
        SessionInputValue::from(&enc_attn_mask),
    ));
    step0_inputs.push((
        Cow::Borrowed("input_ids"),
        SessionInputValue::from(
            Tensor::from_array(([1usize, 2usize], vec![eos_id, tgt_id]))
                .map_err(ort_error("décodeur step0 input_ids"))?,
        ),
    ));
    step0_inputs.push((
        Cow::Borrowed("encoder_hidden_states"),
        SessionInputValue::from(&encoder_hidden),
    ));
    step0_inputs.push((
        Cow::Borrowed("use_cache_branch"),
        SessionInputValue::from(
            Tensor::from_array(([1usize], vec![false]))
                .map_err(ort_error("décodeur use_cache_branch"))?,
        ),
    ));
    for layer in 0..NUM_LAYERS {
        for kind in [
            "decoder.key",
            "decoder.value",
            "encoder.key",
            "encoder.value",
        ] {
            let empty =
                Tensor::from_array(([1usize, NUM_HEADS, 0usize, HEAD_DIM], Vec::<f32>::new()))
                    .map_err(ort_error("past_key_values vide"))?;
            step0_inputs.push((
                Cow::Owned(format!("past_key_values.{layer}.{kind}")),
                SessionInputValue::from(empty),
            ));
        }
    }

    let mut outputs = sessions
        .decoder
        .run(step0_inputs)
        .map_err(ort_error("décodeur step0"))?;

    let mut next_token = argmax_last_logit(&outputs, &[pad_id, bos_id, unk_id])?;
    let mut decoder_kv: Vec<(DynValue, DynValue)> = Vec::with_capacity(NUM_LAYERS);
    let mut encoder_kv: Vec<(DynValue, DynValue)> = Vec::with_capacity(NUM_LAYERS);
    for layer in 0..NUM_LAYERS {
        let dk = outputs
            .remove(format!("present.{layer}.decoder.key"))
            .ok_or_else(|| Error::PostProcess(format!("present.{layer}.decoder.key absent")))?;
        let dv = outputs
            .remove(format!("present.{layer}.decoder.value"))
            .ok_or_else(|| Error::PostProcess(format!("present.{layer}.decoder.value absent")))?;
        let ek = outputs
            .remove(format!("present.{layer}.encoder.key"))
            .ok_or_else(|| Error::PostProcess(format!("present.{layer}.encoder.key absent")))?;
        let ev = outputs
            .remove(format!("present.{layer}.encoder.value"))
            .ok_or_else(|| Error::PostProcess(format!("present.{layer}.encoder.value absent")))?;
        decoder_kv.push((dk, dv));
        encoder_kv.push((ek, ev));
    }
    drop(outputs);

    let max_new = (src_len * 3 + 16).min(MAX_GEN_TOKENS);
    let mut generated = Vec::with_capacity(src_len + 16);

    // 3. Boucle autorégressive avec KV-cache (`use_cache_branch = true`).
    for _ in 0..max_new {
        if next_token == eos_id {
            break;
        }
        generated.push(next_token);
        if has_repetition_loop(&generated) {
            break;
        }

        let mut inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
            Vec::with_capacity(4 + NUM_LAYERS * 4);
        inputs.push((
            Cow::Borrowed("encoder_attention_mask"),
            SessionInputValue::from(&enc_attn_mask),
        ));
        inputs.push((
            Cow::Borrowed("input_ids"),
            SessionInputValue::from(
                Tensor::from_array(([1usize, 1usize], vec![next_token]))
                    .map_err(ort_error("décodeur input_ids"))?,
            ),
        ));
        inputs.push((
            Cow::Borrowed("encoder_hidden_states"),
            SessionInputValue::from(&encoder_hidden),
        ));
        inputs.push((
            Cow::Borrowed("use_cache_branch"),
            SessionInputValue::from(
                Tensor::from_array(([1usize], vec![true]))
                    .map_err(ort_error("décodeur use_cache_branch"))?,
            ),
        ));
        for layer in 0..NUM_LAYERS {
            inputs.push((
                Cow::Owned(format!("past_key_values.{layer}.decoder.key")),
                SessionInputValue::from(&decoder_kv[layer].0),
            ));
            inputs.push((
                Cow::Owned(format!("past_key_values.{layer}.decoder.value")),
                SessionInputValue::from(&decoder_kv[layer].1),
            ));
            inputs.push((
                Cow::Owned(format!("past_key_values.{layer}.encoder.key")),
                SessionInputValue::from(&encoder_kv[layer].0),
            ));
            inputs.push((
                Cow::Owned(format!("past_key_values.{layer}.encoder.value")),
                SessionInputValue::from(&encoder_kv[layer].1),
            ));
        }

        let mut step_out = sessions
            .decoder
            .run(inputs)
            .map_err(ort_error("décodeur step"))?;
        next_token = argmax_last_logit(&step_out, &[pad_id, bos_id, unk_id])?;
        for (layer, slot) in decoder_kv.iter_mut().enumerate() {
            let dk = step_out
                .remove(format!("present.{layer}.decoder.key"))
                .ok_or_else(|| Error::PostProcess(format!("present.{layer}.decoder.key absent")))?;
            let dv = step_out
                .remove(format!("present.{layer}.decoder.value"))
                .ok_or_else(|| {
                    Error::PostProcess(format!("present.{layer}.decoder.value absent"))
                })?;
            *slot = (dk, dv);
        }
    }

    Ok(sessions.tokenizer.decode(&generated))
}

fn argmax_last_logit(
    outputs: &parakeet_rs::ort::session::SessionOutputs<'_>,
    blocked: &[i64],
) -> Result<i64, Error> {
    let (shape, logits) = outputs["logits"]
        .try_extract_tensor::<f32>()
        .map_err(ort_error("logits"))?;
    if shape.len() != 3 || shape[0] < 1 || shape[1] < 1 || shape[2] < 1 {
        return Err(Error::PostProcess(format!(
            "M2M100 : forme logits inattendue {shape:?}"
        )));
    }
    let seq_len = shape[1] as usize;
    let vocab = shape[2] as usize;
    let offset = (seq_len - 1) * vocab;
    let slice = &logits[offset..offset + vocab];
    let mut best_id = 2i64;
    let mut best_val = f32::NEG_INFINITY;
    for (idx, &val) in slice.iter().enumerate() {
        let id = idx as i64;
        if blocked.contains(&id) {
            continue;
        }
        if val > best_val {
            best_val = val;
            best_id = id;
        }
    }
    Ok(best_id)
}

/// Coupe une boucle dégénérée éventuelle (même motif répété 4 fois de suite).
fn has_repetition_loop(tokens: &[i64]) -> bool {
    let n = tokens.len();
    for period in 1..=8 {
        if n >= period * 4 {
            let tail = &tokens[n - period..];
            if tokens[n - period * 2..n - period] == *tail
                && tokens[n - period * 3..n - period * 2] == *tail
                && tokens[n - period * 4..n - period * 3] == *tail
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI_TOKENIZER: &str = r#"{
        "model": {
            "type": "BPE",
            "vocab": {
                "<s>": 0,
                "<pad>": 1,
                "</s>": 2,
                "<unk>": 3,
                "▁": 4,
                "b": 5,
                "o": 6,
                "n": 7,
                "j": 8,
                "u": 9,
                "r": 10,
                "▁b": 11,
                "▁bo": 12,
                "▁bon": 13,
                "ou": 14,
                "our": 15,
                "▁bonj": 16,
                "▁bonjour": 17,
                "!": 18
            },
            "merges": [
                "▁ b",
                "▁b o",
                "▁bo n",
                "o u",
                "ou r",
                "▁bon j",
                "▁bonj our"
            ]
        },
        "added_tokens": [
            { "id": 0, "content": "<s>" },
            { "id": 1, "content": "<pad>" },
            { "id": 2, "content": "</s>" },
            { "id": 3, "content": "<unk>" },
            { "id": 128022, "content": "__en__" },
            { "id": 128028, "content": "__fr__" }
        ]
    }"#;

    #[test]
    fn bpe_tokenizer_encodes_and_decodes_with_language_tokens() {
        let tok = M2m100Tokenizer::from_json_str(MINI_TOKENIZER).unwrap();
        assert_eq!(tok.lang_id("fr"), Some(128028));
        assert_eq!(tok.lang_id("en"), Some(128022));

        let ids = tok.encode("bonjour!", 128028, 32);
        assert_eq!(ids, vec![128028, 17, 18, 2]);
        assert_eq!(tok.decode(&[17, 18, 2]), "bonjour!");
    }

    #[test]
    fn detects_repetition_loops() {
        assert!(!has_repetition_loop(&[10, 20, 10, 20, 10, 20]));
        assert!(has_repetition_loop(&[10, 20, 10, 20, 10, 20, 10, 20]));
        assert!(has_repetition_loop(&[5, 9, 9, 9, 9]));
    }
}
