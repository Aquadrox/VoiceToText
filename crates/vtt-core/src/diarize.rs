//! Qui parle quand (diarisation), à la manière de pyannote 3 :
//!
//! 1. **Segmentation** : une fenêtre de 10 s glisse sur l'audio ; le modèle
//!    pyannote `segmentation-3.0` y repère jusqu'à trois voix (« locales » à
//!    la fenêtre), trame par trame (≈ 17 ms), chevauchements compris.
//! 2. **Empreintes** : pour chaque voix de chaque fenêtre, un modèle
//!    d'empreinte vocale (WeSpeaker, 3D-Speaker) résume ce qu'elle dit en un
//!    vecteur ; deux vecteurs proches = même voix.
//! 3. **Regroupement** : classification hiérarchique ascendante (lien moyen,
//!    distance cosinus) ; on s'arrête à un seuil, ou à un nombre de
//!    locuteurs imposé.
//! 4. **Reconstruction** : les voix locales deviennent des locuteurs
//!    globaux ; chaque trame garde autant de locuteurs que la segmentation en
//!    entendait ; on en tire des tours de parole.
//!
//! Les modèles eux-mêmes (ONNX Runtime) sont dans `vtt-asr` : ici, ils ne
//! sont vus qu'à travers le trait [`Models`], ce qui permet de tester tout
//! l'algorithme sans eux.

use crate::pipeline::{Error, SAMPLE_RATE, Word};

/// Fenêtre analysée par la segmentation : 10 s.
pub const WINDOW: usize = SAMPLE_RATE as usize * 10;
/// Écart entre deux trames de segmentation (échantillons).
pub const FRAME_STEP: usize = 270;
/// Champ récepteur d'une trame : son centre est à `FRAME_SIZE / 2`.
pub const FRAME_SIZE: usize = 991;
/// Voix repérées au plus par fenêtre.
pub const LOCAL_SPEAKERS: usize = 3;
/// Classes de sortie de la segmentation (« powerset ») : personne, une voix
/// parmi trois, ou deux voix ensemble.
pub const CLASSES: usize = 7;
const POWERSET: [[bool; LOCAL_SPEAKERS]; CLASSES] = [
    [false, false, false],
    [true, false, false],
    [false, true, false],
    [false, false, true],
    [true, true, false],
    [true, false, true],
    [false, true, true],
];
/// Début de la trame 0 : son champ récepteur, moins une demi-trame.
const FRAME_OFFSET: usize = (FRAME_SIZE - FRAME_STEP) / 2;
/// Parole minimale pour calculer une empreinte : 0,5 s.
const MIN_EMBEDDING_SAMPLES: usize = SAMPLE_RATE as usize / 2;
/// Fenêtres au plus (une heure au pas de 2 s) : au-delà, le pas s'allonge,
/// pour que le regroupement (mémoire en n²) reste sous ≈ 60 Mo.
const MAX_WINDOWS: usize = 1_800;

/// Les deux modèles de la diarisation.
pub trait Models {
    /// Scores (une ligne de [`CLASSES`] valeurs par trame) pour une fenêtre
    /// de [`WINDOW`] échantillons.
    fn segment(&mut self, window: &[f32]) -> Result<Vec<[f32; CLASSES]>, Error>;
    /// Empreinte vocale de quelques secondes de parole d'une seule voix.
    fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, Error>;
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Écart entre deux fenêtres (échantillons). Plus petit : plus précis,
    /// mais plus long (pyannote : 1 s ; mesuré : 2 s aussi juste, deux fois
    /// plus rapide ; 5 s confond des voix).
    pub step: usize,
    /// Distance cosinus au-delà de laquelle deux groupes de voix restent
    /// séparés. Dépend du modèle d'empreinte : 0,55 pour WeSpeaker
    /// ResNet34-LM (mesuré sur des enregistrements de 2 et 4 voix).
    pub threshold: f32,
    /// Nombre de locuteurs imposé (`None` : déduit du seuil).
    pub speakers: Option<usize>,
    /// Un groupe de moins d'empreintes est rattaché au groupe le plus proche
    /// (bruit, rires, quelques mots isolés).
    pub min_cluster_size: usize,
    /// Tour de parole plus court ignoré.
    pub min_on_ms: u64,
    /// Silence plus court entre deux tours d'un même locuteur : comblé.
    pub min_off_ms: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            step: SAMPLE_RATE as usize * 2,
            threshold: 0.55,
            speakers: None,
            min_cluster_size: 3,
            min_on_ms: 300,
            min_off_ms: 500,
        }
    }
}

/// Un locuteur parle de `start_ms` à `end_ms`. Locuteurs numérotés à partir
/// de 0 dans l'ordre où ils prennent la parole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: usize,
}

fn samples_to_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

/// Débuts des fenêtres : tous les `step`, la dernière calée sur la fin.
fn window_starts(len: usize, step: usize) -> Vec<usize> {
    if len <= WINDOW {
        return vec![0];
    }
    let last = len - WINDOW;
    let mut starts: Vec<usize> = (0..=last).step_by(step.max(FRAME_STEP)).collect();
    if starts.last() != Some(&last) {
        starts.push(last);
    }
    starts
}

/// Voix actives trame par trame : la classe la plus probable.
fn decode(scores: &[[f32; CLASSES]]) -> Vec<[bool; LOCAL_SPEAKERS]> {
    scores
        .iter()
        .map(|row| {
            let best = (0..CLASSES)
                .max_by(|&a, &b| row[a].total_cmp(&row[b]))
                .unwrap_or(0);
            POWERSET[best]
        })
        .collect()
}

/// Échantillons couverts par la trame `frame` d'une fenêtre.
fn frame_span(frame: usize) -> (usize, usize) {
    let start = FRAME_OFFSET + frame * FRAME_STEP;
    (start, start + FRAME_STEP)
}

struct Window {
    start: usize,
    activity: Vec<[bool; LOCAL_SPEAKERS]>,
}

/// Parole de la voix `speaker` dans la fenêtre : seule si possible (sans
/// chevauchement, l'empreinte est plus nette), sinon toute.
fn speech_of(audio: &[f32], window: &Window, speaker: usize) -> Vec<f32> {
    let gather = |alone: bool| {
        let mut samples = Vec::new();
        for (frame, voices) in window.activity.iter().enumerate() {
            let overlap = voices.iter().filter(|&&v| v).count() > 1;
            if voices[speaker] && !(alone && overlap) {
                let (a, b) = frame_span(frame);
                let (a, b) = (
                    (window.start + a).min(audio.len()),
                    (window.start + b).min(audio.len()),
                );
                samples.extend_from_slice(&audio[a..b]);
            }
        }
        samples
    };
    let alone = gather(true);
    if alone.len() >= MIN_EMBEDDING_SAMPLES {
        alone
    } else {
        gather(false)
    }
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

/// Classification hiérarchique ascendante, lien moyen, sur des vecteurs
/// normés. Algorithme de la « chaîne des plus proches voisins » : O(n²).
/// Renvoie un numéro de groupe par vecteur.
pub fn cluster(
    embeddings: &[Vec<f32>],
    threshold: f32,
    speakers: Option<usize>,
    min_cluster_size: usize,
) -> Vec<usize> {
    let n = embeddings.len();
    if n <= 1 {
        return vec![0; n];
    }
    let mut dist = vec![0.0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let d = cosine_distance(&embeddings[i], &embeddings[j]);
            dist[i * n + j] = d;
            dist[j * n + i] = d;
        }
    }
    let mut size = vec![1usize; n];
    let mut active = vec![true; n];
    // (distance, a, b) : `a` absorbe `b`.
    let mut merges: Vec<(f32, usize, usize)> = Vec::with_capacity(n - 1);
    let mut chain: Vec<usize> = Vec::new();
    while merges.len() < n - 1 {
        if chain.is_empty() {
            chain.push(
                active
                    .iter()
                    .position(|&a| a)
                    .expect("au moins deux groupes"),
            );
        }
        let top = *chain.last().expect("chaîne non vide");
        let previous = chain.len().checked_sub(2).map(|i| chain[i]);
        // Plus proche voisin ; à égalité, l'élément précédent de la chaîne
        // (sinon l'algorithme peut boucler).
        let mut nearest = previous;
        let mut best = previous.map_or(f32::INFINITY, |p| dist[top * n + p]);
        for (other, &alive) in active.iter().enumerate() {
            if alive && other != top && dist[top * n + other] < best {
                best = dist[top * n + other];
                nearest = Some(other);
            }
        }
        let nearest = nearest.expect("un autre groupe actif");
        if Some(nearest) == previous {
            chain.truncate(chain.len() - 2);
            let (a, b) = (top.min(nearest), top.max(nearest));
            merges.push((best, a, b));
            // Lance-Williams, lien moyen.
            let (sa, sb) = (size[a] as f32, size[b] as f32);
            for k in 0..n {
                if active[k] && k != a && k != b {
                    let d = (sa * dist[a * n + k] + sb * dist[b * n + k]) / (sa + sb);
                    dist[a * n + k] = d;
                    dist[k * n + a] = d;
                }
            }
            size[a] += size[b];
            active[b] = false;
        } else {
            chain.push(nearest);
        }
    }

    // Le lien moyen est « réductible » : trier les fusions par distance
    // redonne l'arbre ; on le coupe au seuil ou au nombre voulu.
    merges.sort_by(|x, y| x.0.total_cmp(&y.0));
    let keep = match speakers {
        Some(k) => n.saturating_sub(k.clamp(1, n)),
        None => merges.iter().take_while(|m| m.0 <= threshold).count(),
    };
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for &(_, a, b) in &merges[..keep] {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        parent[rb] = ra;
    }
    let roots: Vec<usize> = (0..n).map(|i| root(&mut parent, i)).collect();

    // Petits groupes rattachés au grand groupe le plus proche (centroïde).
    let mut members: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, &r) in roots.iter().enumerate() {
        match members.iter_mut().find(|(root, _)| *root == r) {
            Some((_, list)) => list.push(i),
            None => members.push((r, vec![i])),
        }
    }
    let (large, small): (Vec<_>, Vec<_>) = members
        .into_iter()
        .partition(|(_, list)| list.len() >= min_cluster_size);
    let mut labels = vec![0usize; n];
    if large.is_empty() || speakers.is_some() {
        // Rien d'assez grand, ou nombre imposé : groupes gardés tels quels.
        let mut all = large;
        all.extend(small);
        for (label, (_, list)) in all.iter().enumerate() {
            list.iter().for_each(|&i| labels[i] = label);
        }
        return labels;
    }
    let centroids: Vec<Vec<f32>> = large
        .iter()
        .map(|(_, list)| {
            let mut c = vec![0.0f32; embeddings[0].len()];
            for &i in list {
                c.iter_mut().zip(&embeddings[i]).for_each(|(c, x)| *c += x);
            }
            normalize(c)
        })
        .collect();
    for (label, (_, list)) in large.iter().enumerate() {
        list.iter().for_each(|&i| labels[i] = label);
    }
    for (_, list) in &small {
        for &i in list {
            labels[i] = (0..centroids.len())
                .min_by(|&a, &b| {
                    cosine_distance(&embeddings[i], &centroids[a])
                        .total_cmp(&cosine_distance(&embeddings[i], &centroids[b]))
                })
                .expect("au moins un grand groupe");
        }
    }
    labels
}

/// Tours de parole de `audio` (16 kHz mono). `progress` reçoit l'avancement
/// entre 0 et 1.
pub fn diarize(
    audio: &[f32],
    models: &mut impl Models,
    options: &Options,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Turn>, Error> {
    let step = options.step.max(audio.len() / MAX_WINDOWS);
    let starts = window_starts(audio.len(), step);
    // Segmentation ≈ 1/3 du temps, empreintes ≈ 2/3.
    let mut windows = Vec::with_capacity(starts.len());
    let mut buffer = vec![0.0f32; WINDOW];
    for (i, &start) in starts.iter().enumerate() {
        let end = (start + WINDOW).min(audio.len());
        buffer[..end - start].copy_from_slice(&audio[start..end]);
        buffer[end - start..].fill(0.0);
        let mut activity = decode(&models.segment(&buffer)?);
        // Trames au-delà de la fin de l'audio (fenêtre complétée) : muettes.
        activity.truncate((end - start).saturating_sub(FRAME_OFFSET) / FRAME_STEP);
        windows.push(Window { start, activity });
        progress((i + 1) as f32 / starts.len() as f32 / 3.0);
    }

    // Une empreinte par voix locale qui parle assez.
    let mut owners: Vec<(usize, usize)> = Vec::new();
    let mut embeddings: Vec<Vec<f32>> = Vec::new();
    for (w, window) in windows.iter().enumerate() {
        for speaker in 0..LOCAL_SPEAKERS {
            let speech = speech_of(audio, window, speaker);
            if speech.len() >= MIN_EMBEDDING_SAMPLES {
                embeddings.push(normalize(models.embed(&speech)?));
                owners.push((w, speaker));
            }
        }
        progress((1.0 + 2.0 * (w + 1) as f32 / windows.len() as f32) / 3.0);
    }
    if embeddings.is_empty() {
        return Ok(Vec::new());
    }
    let labels = cluster(
        &embeddings,
        options.threshold,
        options.speakers,
        options.min_cluster_size,
    );
    let clusters = labels.iter().max().map_or(0, |m| m + 1);
    let mut local_to_global = vec![[None; LOCAL_SPEAKERS]; windows.len()];
    for (&(w, s), &label) in owners.iter().zip(&labels) {
        local_to_global[w][s] = Some(label);
    }

    // Votes par trame globale : voix entendues (moyenne des fenêtres) et
    // locuteur de chacune.
    let frames = audio.len().saturating_sub(FRAME_OFFSET) / FRAME_STEP + 1;
    let mut votes = vec![0.0f32; frames * clusters];
    let mut heard = vec![0.0f32; frames];
    let mut seen = vec![0.0f32; frames];
    for (w, window) in windows.iter().enumerate() {
        let offset = (window.start + FRAME_STEP / 2) / FRAME_STEP;
        for (f, voices) in window.activity.iter().enumerate() {
            let g = offset + f;
            if g >= frames {
                break;
            }
            seen[g] += 1.0;
            for (s, &on) in voices.iter().enumerate() {
                if on {
                    heard[g] += 1.0;
                    if let Some(k) = local_to_global[w][s] {
                        votes[g * clusters + k] += 1.0;
                    }
                }
            }
        }
    }
    let mut active = vec![false; frames * clusters];
    let mut order: Vec<usize> = (0..clusters).collect();
    for g in 0..frames {
        if seen[g] == 0.0 {
            continue;
        }
        let count = (heard[g] / seen[g]).round() as usize;
        let row = &votes[g * clusters..(g + 1) * clusters];
        order.sort_by(|&a, &b| row[b].total_cmp(&row[a]));
        for &k in order.iter().take(count) {
            if row[k] > 0.0 {
                active[g * clusters + k] = true;
            }
        }
    }

    // Trames → tours de parole, petits silences comblés, bribes ignorées.
    let to_ms = |frame: usize| samples_to_ms(FRAME_OFFSET + frame * FRAME_STEP);
    let mut turns = Vec::new();
    for k in 0..clusters {
        let mut own: Vec<Turn> = Vec::new();
        let mut start = None;
        for g in 0..=frames {
            let on = g < frames && active[g * clusters + k];
            match (on, start) {
                (true, None) => start = Some(g),
                (false, Some(s)) => {
                    let turn = Turn {
                        start_ms: to_ms(s),
                        end_ms: to_ms(g).min(samples_to_ms(audio.len())),
                        speaker: k,
                    };
                    match own.last_mut() {
                        Some(last) if turn.start_ms - last.end_ms < options.min_off_ms => {
                            last.end_ms = turn.end_ms;
                        }
                        _ => own.push(turn),
                    }
                    start = None;
                }
                _ => {}
            }
        }
        turns.extend(
            own.into_iter()
                .filter(|t| t.end_ms - t.start_ms >= options.min_on_ms),
        );
    }
    turns.sort_by_key(|t| (t.start_ms, t.speaker));

    // Locuteurs renumérotés dans l'ordre d'arrivée.
    let mut arrival: Vec<usize> = Vec::new();
    for t in &turns {
        if !arrival.contains(&t.speaker) {
            arrival.push(t.speaker);
        }
    }
    for t in &mut turns {
        t.speaker = arrival
            .iter()
            .position(|&s| s == t.speaker)
            .expect("locuteur vu ci-dessus");
    }
    progress(1.0);
    Ok(turns)
}

/// Locuteur qui parle le plus entre `start_ms` et `end_ms` (`None` : aucun).
pub fn speaker_between(turns: &[Turn], start_ms: u64, end_ms: u64) -> Option<usize> {
    let mut time: Vec<(usize, u64)> = Vec::new();
    for t in turns {
        let overlap = t
            .end_ms
            .min(end_ms)
            .saturating_sub(t.start_ms.max(start_ms));
        if overlap > 0 {
            match time.iter_mut().find(|(s, _)| *s == t.speaker) {
                Some((_, total)) => *total += overlap,
                None => time.push((t.speaker, overlap)),
            }
        }
    }
    time.into_iter().max_by_key(|&(_, t)| t).map(|(s, _)| s)
}

/// Locuteur au moment `ms`, ou le plus proche dans le temps.
fn speaker_near(turns: &[Turn], ms: u64) -> Option<usize> {
    turns
        .iter()
        .min_by_key(|t| {
            if ms < t.start_ms {
                t.start_ms - ms
            } else {
                ms.saturating_sub(t.end_ms)
            }
        })
        .map(|t| t.speaker)
}

/// Réplique d'un locuteur : mots consécutifs attribués au même locuteur.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub speaker: Option<usize>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Découpe une transcription horodatée mot à mot en répliques par locuteur
/// (chaque mot va au locuteur de son milieu).
pub fn lines(words: &[Word], turns: &[Turn]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for word in words {
        let speaker = speaker_near(turns, (word.start_ms + word.end_ms) / 2);
        match out.last_mut() {
            Some(line) if line.speaker == speaker => {
                line.end_ms = word.end_ms;
                line.text.push(' ');
                line.text.push_str(&word.text);
            }
            _ => out.push(Line {
                speaker,
                start_ms: word.start_ms,
                end_ms: word.end_ms,
                text: word.text.clone(),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: usize = SAMPLE_RATE as usize;

    /// Faux modèles : l'audio est un signal constant dont la valeur dit qui
    /// parle (0 : silence, 0,1 : A, 0,2 : B, 0,3 : C). La segmentation voit
    /// au plus trois voix, numérotées dans l'ordre d'apparition dans la
    /// fenêtre, comme le vrai modèle (les numéros changent d'une fenêtre à
    /// l'autre : c'est le regroupement qui doit les relier).
    struct Fake;

    fn voice(sample: f32) -> Option<usize> {
        match (sample * 10.0).round() as i32 {
            1 => Some(0),
            2 => Some(1),
            3 => Some(2),
            _ => None,
        }
    }

    impl Models for Fake {
        fn segment(&mut self, window: &[f32]) -> Result<Vec<[f32; CLASSES]>, Error> {
            let frames = (WINDOW - FRAME_SIZE) / FRAME_STEP + 1;
            let mut local: Vec<usize> = Vec::new();
            Ok((0..frames)
                .map(|f| {
                    let (a, b) = frame_span(f);
                    let mut row = [0.0; CLASSES];
                    let class = match voice(window[(a + b) / 2]) {
                        None => 0,
                        Some(v) => {
                            let slot = local.iter().position(|&x| x == v).unwrap_or_else(|| {
                                local.push(v);
                                local.len() - 1
                            });
                            1 + slot.min(2)
                        }
                    };
                    row[class] = 1.0;
                    row
                })
                .collect())
        }

        fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>, Error> {
            // Empreinte : un axe par voix, un peu de « bruit » commun.
            let mut e = vec![0.1f32; 3];
            if let Some(v) = voice(samples[samples.len() / 2]) {
                e[v] = 1.0;
            }
            Ok(e)
        }
    }

    fn conversation(parts: &[(f32, usize)]) -> Vec<f32> {
        parts
            .iter()
            .flat_map(|&(value, seconds)| std::iter::repeat_n(value, seconds * SECOND))
            .collect()
    }

    #[test]
    fn windows_cover_the_whole_audio() {
        assert_eq!(window_starts(5 * SECOND, 2 * SECOND), vec![0]);
        assert_eq!(window_starts(WINDOW, 2 * SECOND), vec![0]);
        let starts = window_starts(25 * SECOND, 2 * SECOND);
        assert_eq!(starts.first(), Some(&0));
        assert_eq!(starts.last(), Some(&(15 * SECOND)));
        assert!(starts.windows(2).all(|w| w[1] - w[0] <= 2 * SECOND));
    }

    #[test]
    fn powerset_classes_decode_to_voices() {
        let mut row = [0.0; CLASSES];
        row[5] = 1.0;
        assert_eq!(decode(&[row]), vec![[true, false, true]]);
    }

    #[test]
    fn two_speakers_taking_turns_are_told_apart() {
        // A 6 s, silence 1 s, B 6 s, A 6 s, B 6 s.
        let audio = conversation(&[(0.1, 6), (0.0, 1), (0.2, 6), (0.1, 6), (0.2, 6)]);
        let turns = diarize(&audio, &mut Fake, &Options::default(), &mut |_| {}).unwrap();
        let speakers: Vec<usize> = turns.iter().map(|t| t.speaker).collect();
        assert_eq!(speakers, vec![0, 1, 0, 1], "{turns:?}");
        // Frontières à moins d'une demi-seconde près.
        let near = |ms: u64, expected: u64| ms.abs_diff(expected) < 500;
        assert!(near(turns[0].end_ms, 6_000), "{turns:?}");
        assert!(near(turns[1].start_ms, 7_000), "{turns:?}");
        assert!(near(turns[2].start_ms, 13_000), "{turns:?}");
        assert!(near(turns[3].end_ms, 25_000), "{turns:?}");
    }

    #[test]
    fn a_third_speaker_appears_later() {
        let audio = conversation(&[(0.1, 8), (0.2, 8), (0.3, 8), (0.1, 8)]);
        let turns = diarize(&audio, &mut Fake, &Options::default(), &mut |_| {}).unwrap();
        let speakers: Vec<usize> = turns.iter().map(|t| t.speaker).collect();
        assert_eq!(speakers, vec![0, 1, 2, 0], "{turns:?}");
    }

    #[test]
    fn forced_speaker_count_merges_voices() {
        let audio = conversation(&[(0.1, 8), (0.2, 8), (0.3, 8)]);
        let options = Options {
            speakers: Some(2),
            ..Options::default()
        };
        let turns = diarize(&audio, &mut Fake, &options, &mut |_| {}).unwrap();
        let mut speakers: Vec<usize> = turns.iter().map(|t| t.speaker).collect();
        speakers.dedup();
        assert!(speakers.iter().all(|&s| s < 2), "{turns:?}");
    }

    #[test]
    fn silence_gives_no_turn() {
        let audio = vec![0.0; 12 * SECOND];
        let turns = diarize(&audio, &mut Fake, &Options::default(), &mut |_| {}).unwrap();
        assert!(turns.is_empty());
    }

    #[test]
    fn progress_ends_at_one() {
        let audio = conversation(&[(0.1, 30)]);
        let mut last = 0.0;
        diarize(&audio, &mut Fake, &Options::default(), &mut |p| {
            assert!(p >= last, "l'avancement ne recule pas");
            last = p;
        })
        .unwrap();
        assert_eq!(last, 1.0);
    }

    #[test]
    fn clustering_stops_at_the_threshold() {
        let e = |x: f32, y: f32| normalize(vec![x, y]);
        let embeddings = vec![e(1.0, 0.0), e(0.99, 0.05), e(0.0, 1.0), e(0.05, 0.99)];
        let labels = cluster(&embeddings, 0.3, None, 1);
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[2], labels[3]);
        assert_ne!(labels[0], labels[2]);
        assert_eq!(cluster(&embeddings, 1.5, None, 1), vec![0; 4]);
        assert_eq!(cluster(&embeddings, 0.3, Some(1), 1), vec![0; 4]);
    }

    #[test]
    fn small_clusters_join_the_nearest_large_one() {
        let e = |x: f32, y: f32| normalize(vec![x, y]);
        let embeddings = vec![
            e(1.0, 0.0),
            e(1.0, 0.02),
            e(1.0, 0.04),
            e(0.0, 1.0),
            e(0.02, 1.0),
            e(0.04, 1.0),
            // Isolé, plus proche du premier groupe.
            e(1.0, 0.6),
        ];
        let labels = cluster(&embeddings, 0.1, None, 2);
        assert_eq!(labels[6], labels[0]);
        assert_ne!(labels[0], labels[3]);
    }

    #[test]
    fn line_goes_to_whoever_speaks_most() {
        let turns = [
            Turn {
                start_ms: 0,
                end_ms: 4_000,
                speaker: 0,
            },
            Turn {
                start_ms: 4_000,
                end_ms: 10_000,
                speaker: 1,
            },
        ];
        assert_eq!(speaker_between(&turns, 3_000, 9_000), Some(1));
        assert_eq!(speaker_between(&turns, 0, 3_000), Some(0));
        assert_eq!(speaker_between(&turns, 11_000, 12_000), None);
    }

    #[test]
    fn words_are_split_into_lines_per_speaker() {
        let turns = [
            Turn {
                start_ms: 0,
                end_ms: 2_000,
                speaker: 0,
            },
            Turn {
                start_ms: 2_500,
                end_ms: 5_000,
                speaker: 1,
            },
        ];
        let word = |text: &str, start_ms, end_ms| Word {
            text: text.into(),
            start_ms,
            end_ms,
        };
        let words = [
            word("Bonjour", 100, 600),
            word("Paul.", 700, 1_200),
            word("Salut", 2_600, 3_000),
            word("Anne.", 3_100, 3_500),
        ];
        let lines = lines(&words, &turns);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Bonjour Paul.");
        assert_eq!(lines[0].speaker, Some(0));
        assert_eq!(lines[1].text, "Salut Anne.");
        assert_eq!((lines[1].start_ms, lines[1].end_ms), (2_600, 3_500));
    }
}
