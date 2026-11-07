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
//!
//! Crédits : idée reprise de pytranscribe (Xavier Beheydt, MIT,
//! <https://github.com/XavierBeheydt/pytranscribe>), qui enchaîne WhisperX
//! et pyannote en Python ; méthode et modèle de segmentation de
//! pyannote.audio (Hervé Bredin, CNRS, MIT) ; empreintes WeSpeaker
//! (CC BY 4.0) ; conversions ONNX et implémentation de référence de
//! sherpa-onnx (k2-fsa, Apache 2.0). Aucune ligne de code n'est copiée :
//! tout est réécrit ici.

use serde::{Deserialize, Serialize};

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
/// Empreintes regroupées au plus (mémoire en n² : ≈ 36 Mo, soit environ une
/// heure et demie d'audio) ; au-delà, les autres rejoignent le groupe le plus
/// proche.
const MAX_CLUSTERED: usize = 3_000;
/// Écart toléré entre deux morceaux consécutifs (arrondis à la
/// milliseconde) : 2 ms.
const CONTIGUOUS: usize = SAMPLE_RATE as usize / 500;
/// Fenêtres analysées ensemble : les deux modèles alternent moins, leurs
/// threads se gênent moins (mesuré : fenêtre par fenêtre, 1,6 fois plus
/// lent).
const BATCH: usize = 8;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: usize,
}

fn samples_to_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

fn ms_to_samples(ms: u64) -> usize {
    (ms * SAMPLE_RATE as u64 / 1000) as usize
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

/// Voix entendues dans une fenêtre, trame par trame.
#[derive(Debug, Clone, PartialEq)]
struct Window {
    /// Début (échantillons depuis le début de l'audio).
    start: usize,
    activity: Vec<[bool; LOCAL_SPEAKERS]>,
}

/// Parole de la voix `speaker` dans la fenêtre (`audio` : ses échantillons) :
/// seule si possible (sans chevauchement, l'empreinte est plus nette), sinon
/// toute.
fn speech_of(audio: &[f32], activity: &[[bool; LOCAL_SPEAKERS]], speaker: usize) -> Vec<f32> {
    let gather = |alone: bool| {
        let mut samples = Vec::new();
        for (frame, voices) in activity.iter().enumerate() {
            let overlap = voices.iter().filter(|&&v| v).count() > 1;
            if voices[speaker] && !(alone && overlap) {
                let (a, b) = frame_span(frame);
                samples.extend_from_slice(&audio[a.min(audio.len())..b.min(audio.len())]);
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

/// Centre (normé) de chaque groupe.
fn centroids(embeddings: &[Vec<f32>], labels: &[usize]) -> Vec<Vec<f32>> {
    let groups = labels.iter().max().map_or(0, |m| m + 1);
    let dim = embeddings.first().map_or(0, Vec::len);
    let mut sums = vec![vec![0.0f32; dim]; groups];
    for (e, &label) in embeddings.iter().zip(labels) {
        sums[label].iter_mut().zip(e).for_each(|(s, x)| *s += x);
    }
    sums.into_iter().map(normalize).collect()
}

/// Groupe dont le centre est le plus proche.
fn nearest(embedding: &[f32], centroids: &[Vec<f32>]) -> usize {
    (0..centroids.len())
        .min_by(|&a, &b| {
            cosine_distance(embedding, &centroids[a])
                .total_cmp(&cosine_distance(embedding, &centroids[b]))
        })
        .unwrap_or(0)
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
    let mut large_labels = Vec::new();
    let mut large_embeddings = Vec::new();
    for (label, (_, list)) in large.iter().enumerate() {
        for &i in list {
            labels[i] = label;
            large_labels.push(label);
            large_embeddings.push(embeddings[i].clone());
        }
    }
    let centers = centroids(&large_embeddings, &large_labels);
    for (_, list) in &small {
        for &i in list {
            labels[i] = nearest(&embeddings[i], &centers);
        }
    }
    labels
}

/// Analyse des voix au fil de l'eau : l'audio arrive par morceaux (blocs
/// d'une réunion, fichier lu peu à peu) et n'est pas gardé ; seules restent
/// les voix entendues dans chaque fenêtre et leurs empreintes. Les tours de
/// parole se calculent ensuite ([`Analysis::turns`]), autant de fois qu'on
/// veut (nombre de locuteurs changé), sans l'audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    step: usize,
    windows: Vec<Window>,
    /// Pour chaque empreinte : sa fenêtre et sa voix locale.
    owners: Vec<(usize, usize)>,
    embeddings: Vec<Vec<f32>>,
    /// Fin de l'audio reçu (échantillons depuis le début).
    end: usize,
    /// Audio de la zone en cours, à partir de `region_start`, pas encore
    /// entièrement analysé.
    region_start: usize,
    buffer: Vec<f32>,
    /// Début de la prochaine fenêtre.
    next: usize,
    /// Fenêtres prêtes, analysées par lots de [`BATCH`].
    pending: Vec<usize>,
    /// La zone en cours a déjà au moins une fenêtre.
    covered: bool,
}

impl Analysis {
    /// `step` : écart entre deux fenêtres (voir [`Options::step`]).
    pub fn new(step: usize) -> Self {
        Self {
            step: step.max(FRAME_STEP),
            windows: Vec::new(),
            owners: Vec::new(),
            embeddings: Vec::new(),
            end: 0,
            region_start: 0,
            buffer: Vec::new(),
            next: 0,
            pending: Vec::new(),
            covered: false,
        }
    }

    /// Analyse `samples` (16 kHz mono), qui commencent à `start_ms`. Un
    /// morceau qui ne suit pas le précédent (silence sauté) ouvre une
    /// nouvelle zone : aucune fenêtre ne couvre le trou.
    pub fn add(
        &mut self,
        start_ms: u64,
        samples: &[f32],
        models: &mut impl Models,
    ) -> Result<(), Error> {
        let start = ms_to_samples(start_ms);
        if !self.buffer.is_empty() {
            let region_end = self.region_start + self.buffer.len();
            if start.abs_diff(region_end) > CONTIGUOUS {
                self.flush(models)?;
            }
        }
        if self.buffer.is_empty() {
            self.region_start = start;
            self.next = start;
            self.covered = false;
        }
        self.buffer.extend_from_slice(samples);
        let region_end = self.region_start + self.buffer.len();
        self.end = self.end.max(region_end);
        while self.next + WINDOW <= region_end {
            self.pending.push(self.next);
            self.next += self.step;
            self.covered = true;
        }
        if self.pending.len() >= BATCH {
            let ready = std::mem::take(&mut self.pending);
            self.windows_at(&ready, models)?;
        }
        // Gardé : les fenêtres en attente, et de quoi caler une dernière
        // fenêtre sur la fin de la zone.
        let keep_from = self
            .next
            .saturating_sub(self.step)
            .min(self.pending.first().copied().unwrap_or(usize::MAX))
            .max(self.region_start)
            .min(region_end);
        self.buffer.drain(..keep_from - self.region_start);
        self.region_start = keep_from;
        Ok(())
    }

    /// Fin de l'audio : analyse ce qui reste de la zone en cours.
    pub fn finish(&mut self, models: &mut impl Models) -> Result<(), Error> {
        self.flush(models)
    }

    /// Dernière fenêtre de la zone, calée sur sa fin (complétée par du
    /// silence si la zone dure moins de 10 s).
    fn flush(&mut self, models: &mut impl Models) -> Result<(), Error> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let region_end = self.region_start + self.buffer.len();
        let mut ready = std::mem::take(&mut self.pending);
        if !self.covered {
            ready.push(self.region_start);
        } else if self.next - self.step + WINDOW < region_end {
            ready.push(region_end - WINDOW);
        }
        self.windows_at(&ready, models)?;
        self.buffer.clear();
        Ok(())
    }

    /// Analyse les fenêtres qui commencent à `starts` (dans la zone en
    /// cours) : toutes les segmentations, puis toutes les empreintes (les
    /// deux modèles se succèdent moins : mesuré plus rapide).
    fn windows_at(&mut self, starts: &[usize], models: &mut impl Models) -> Result<(), Error> {
        let mut padded = vec![0.0f32; WINDOW];
        let first = self.windows.len();
        for &start in starts {
            let audio = self.window_audio(start);
            padded[..audio.len()].copy_from_slice(audio);
            padded[audio.len()..].fill(0.0);
            let real = audio.len();
            let mut activity = decode(&models.segment(&padded)?);
            // Trames au-delà de l'audio (fenêtre complétée) : muettes.
            activity.truncate(real.saturating_sub(FRAME_OFFSET) / FRAME_STEP);
            self.windows.push(Window { start, activity });
        }
        for w in first..self.windows.len() {
            for speaker in 0..LOCAL_SPEAKERS {
                let window = &self.windows[w];
                let speech = speech_of(self.window_audio(window.start), &window.activity, speaker);
                if speech.len() >= MIN_EMBEDDING_SAMPLES {
                    self.embeddings.push(normalize(models.embed(&speech)?));
                    self.owners.push((w, speaker));
                }
            }
        }
        Ok(())
    }

    /// Audio de la fenêtre qui commence à `start` (moins de 10 s en fin de
    /// zone).
    fn window_audio(&self, start: usize) -> &[f32] {
        let offset = start - self.region_start;
        &self.buffer[offset..(offset + WINDOW).min(self.buffer.len())]
    }

    /// Durée d'audio reçue.
    pub fn duration_ms(&self) -> u64 {
        samples_to_ms(self.end)
    }

    /// Tours de parole d'après l'analyse (`options.step` ne sert pas ici).
    pub fn turns(&self, options: &Options) -> Vec<Turn> {
        let n = self.embeddings.len();
        if n == 0 {
            return Vec::new();
        }
        let labels = if n > MAX_CLUSTERED {
            // Échantillon régulier regroupé ; les autres empreintes
            // rejoignent le groupe le plus proche.
            let picked: Vec<Vec<f32>> = (0..MAX_CLUSTERED)
                .map(|i| self.embeddings[i * n / MAX_CLUSTERED].clone())
                .collect();
            let picked_labels = cluster(
                &picked,
                options.threshold,
                options.speakers,
                options.min_cluster_size,
            );
            let centers = centroids(&picked, &picked_labels);
            self.embeddings
                .iter()
                .map(|e| nearest(e, &centers))
                .collect()
        } else {
            cluster(
                &self.embeddings,
                options.threshold,
                options.speakers,
                options.min_cluster_size,
            )
        };
        let clusters = labels.iter().max().map_or(0, |m| m + 1);
        let mut local_to_global = vec![[None; LOCAL_SPEAKERS]; self.windows.len()];
        for (&(w, s), &label) in self.owners.iter().zip(&labels) {
            local_to_global[w][s] = Some(label);
        }

        // Votes par trame globale : voix entendues (moyenne des fenêtres) et
        // locuteur de chacune.
        let frames = self.end.saturating_sub(FRAME_OFFSET) / FRAME_STEP + 1;
        let mut votes = vec![0.0f32; frames * clusters];
        let mut heard = vec![0.0f32; frames];
        let mut seen = vec![0.0f32; frames];
        for (w, window) in self.windows.iter().enumerate() {
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
                            end_ms: to_ms(g).min(samples_to_ms(self.end)),
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
        turns
    }

    /// Analyse enregistrable (voix par fenêtre et empreintes ; jamais
    /// d'audio). À chiffrer : une empreinte vocale identifie une personne.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.step as u32).to_le_bytes());
        out.extend_from_slice(&(self.end as u64).to_le_bytes());
        out.extend_from_slice(&(self.windows.len() as u32).to_le_bytes());
        for window in &self.windows {
            out.extend_from_slice(&(window.start as u64).to_le_bytes());
            out.extend_from_slice(&(window.activity.len() as u32).to_le_bytes());
            out.extend(window.activity.iter().map(|voices| {
                voices
                    .iter()
                    .enumerate()
                    .fold(0u8, |bits, (i, &on)| bits | (u8::from(on) << i))
            }));
        }
        let dim = self.embeddings.first().map_or(0, Vec::len);
        out.extend_from_slice(&(self.embeddings.len() as u32).to_le_bytes());
        out.extend_from_slice(&(dim as u32).to_le_bytes());
        for (&(w, s), embedding) in self.owners.iter().zip(&self.embeddings) {
            out.extend_from_slice(&(w as u32).to_le_bytes());
            out.push(s as u8);
            for x in embedding {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out
    }

    /// Relit [`Analysis::to_bytes`] (tout est vérifié : fichier tronqué ou
    /// modifié refusé).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.take(MAGIC.len())? != MAGIC {
            return Err("format d'analyse des voix inconnu".into());
        }
        let step = reader.u32()? as usize;
        let end = reader.u64()? as usize;
        let mut analysis = Self::new(step);
        analysis.end = end;
        for _ in 0..reader.u32()? {
            let start = reader.u64()? as usize;
            let frames = reader.u32()? as usize;
            let activity = reader
                .take(frames)?
                .iter()
                .map(|&bits| std::array::from_fn(|i| bits & (1 << i) != 0))
                .collect();
            analysis.windows.push(Window { start, activity });
        }
        let count = reader.u32()? as usize;
        let dim = reader.u32()? as usize;
        for _ in 0..count {
            let w = reader.u32()? as usize;
            let s = reader.take(1)?[0] as usize;
            if w >= analysis.windows.len() || s >= LOCAL_SPEAKERS {
                return Err("analyse des voix incohérente".into());
            }
            let embedding = reader
                .take(dim * 4)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            analysis.owners.push((w, s));
            analysis.embeddings.push(embedding);
        }
        if reader.at != bytes.len() {
            return Err("analyse des voix incohérente".into());
        }
        Ok(analysis)
    }
}

/// En-tête du fichier d'analyse (version 1).
const MAGIC: &[u8] = b"VTTVOIX1";

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or("analyse des voix tronquée")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().expect("8 octets")))
    }
}

/// Tours de parole de `audio` (16 kHz mono). `progress` reçoit l'avancement
/// entre 0 et 1.
pub fn diarize(
    audio: &[f32],
    models: &mut impl Models,
    options: &Options,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Turn>, Error> {
    const PIECE: usize = SAMPLE_RATE as usize * 60;
    let mut analysis = Analysis::new(options.step);
    for (i, piece) in audio.chunks(PIECE).enumerate() {
        analysis.add(samples_to_ms(i * PIECE), piece, models)?;
        let done = ((i + 1) * PIECE).min(audio.len());
        progress(done as f32 / audio.len() as f32 * 0.99);
    }
    analysis.finish(models)?;
    let turns = analysis.turns(options);
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

/// Répartit le texte d'un passage transcrit (`start_ms` à `end_ms`) entre
/// les locuteurs qui y parlent. Le moteur ne donne pas l'heure de chaque
/// mot : les mots sont étalés, dans l'ordre, sur le temps de parole de
/// chacun. Un passage sans tour de parole va au locuteur le plus proche
/// dans le temps.
pub fn split(text: &str, start_ms: u64, end_ms: u64, turns: &[Turn]) -> Vec<Line> {
    let whole = |speaker| {
        vec![Line {
            speaker,
            start_ms,
            end_ms,
            text: text.to_owned(),
        }]
    };
    // Tours coupés au passage ; un instant n'a qu'un locuteur (le premier).
    let mut spans: Vec<(u64, u64, usize)> = turns
        .iter()
        .filter_map(|t| {
            let (a, b) = (t.start_ms.max(start_ms), t.end_ms.min(end_ms));
            (b > a).then_some((a, b, t.speaker))
        })
        .collect();
    spans.sort();
    let mut timeline: Vec<(u64, u64, usize)> = Vec::new();
    for (a, b, speaker) in spans {
        let a = timeline.last().map_or(a, |last| a.max(last.1));
        if b > a {
            timeline.push((a, b, speaker));
        }
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some(first) = timeline.first() else {
        return whole(speaker_near(turns, (start_ms + end_ms) / 2));
    };
    if words.is_empty() || timeline.iter().all(|t| t.2 == first.2) {
        return whole(Some(first.2));
    }
    let total: u64 = timeline.iter().map(|(a, b, _)| b - a).sum();
    // Position sur le temps de parole → (heure, locuteur).
    let locate = |position: u64| {
        let mut before = 0;
        for &(a, b, speaker) in &timeline {
            if position < before + (b - a) {
                return (a + position - before, speaker);
            }
            before += b - a;
        }
        let &(_, b, speaker) = timeline.last().expect("tours non vides");
        (b, speaker)
    };
    let n = words.len() as u64;
    let mut out: Vec<Line> = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let i = i as u64;
        let (_, speaker) = locate((2 * i + 1) * total / (2 * n));
        let (from, _) = locate(i * total / n);
        let to = locate(((i + 1) * total / n).saturating_sub(1)).0 + 1;
        match out.last_mut() {
            Some(line) if line.speaker == Some(speaker) => {
                line.end_ms = to;
                line.text.push(' ');
                line.text.push_str(word);
            }
            _ => out.push(Line {
                speaker: Some(speaker),
                start_ms: from,
                end_ms: to,
                text: (*word).to_owned(),
            }),
        }
    }
    // Le passage garde ses bornes : début du premier, fin du dernier.
    if let Some(line) = out.first_mut() {
        line.start_ms = start_ms;
    }
    if let Some(line) = out.last_mut() {
        line.end_ms = end_ms;
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
        let starts = |pieces: &[(u64, usize)]| {
            let mut analysis = Analysis::new(2 * SECOND);
            for &(start_ms, seconds) in pieces {
                analysis
                    .add(start_ms, &vec![0.0; seconds * SECOND], &mut Fake)
                    .unwrap();
            }
            analysis.finish(&mut Fake).unwrap();
            analysis.windows.iter().map(|w| w.start).collect::<Vec<_>>()
        };
        assert_eq!(starts(&[(0, 5)]), vec![0]);
        assert_eq!(starts(&[(0, 10)]), vec![0]);
        let long = starts(&[(0, 25)]);
        assert_eq!(long.first(), Some(&0));
        assert_eq!(long.last(), Some(&(15 * SECOND)));
        assert!(long.windows(2).all(|w| w[1] - w[0] <= 2 * SECOND));
        // Même audio en morceaux contigus : mêmes fenêtres.
        assert_eq!(starts(&[(0, 7), (7_000, 8), (15_000, 10)]), long);
        // Trou (silence sauté) : nouvelle zone, aucune fenêtre dans le trou.
        let gap = starts(&[(0, 12), (30_000, 6)]);
        assert_eq!(gap, vec![0, 2 * SECOND, 30 * SECOND]);
    }

    #[test]
    fn pieces_give_the_same_turns_as_the_whole_audio() {
        let audio = conversation(&[(0.1, 6), (0.0, 1), (0.2, 6), (0.1, 6), (0.2, 6)]);
        let whole = diarize(&audio, &mut Fake, &Options::default(), &mut |_| {}).unwrap();
        // Blocs de 6 à 15 s, comme ceux d'une réunion.
        let mut analysis = Analysis::new(Options::default().step);
        let mut at = 0;
        for seconds in [6, 9, 7, 3] {
            let end = (at + seconds * SECOND).min(audio.len());
            analysis
                .add(samples_to_ms(at), &audio[at..end], &mut Fake)
                .unwrap();
            at = end;
        }
        analysis.finish(&mut Fake).unwrap();
        assert_eq!(analysis.turns(&Options::default()), whole);
        assert_eq!(analysis.duration_ms(), 25_000);
    }

    #[test]
    fn analysis_survives_a_round_trip_to_bytes() {
        let audio = conversation(&[(0.1, 8), (0.2, 8), (0.3, 8)]);
        let mut analysis = Analysis::new(2 * SECOND);
        analysis.add(0, &audio, &mut Fake).unwrap();
        analysis.finish(&mut Fake).unwrap();
        let bytes = analysis.to_bytes();
        let read = Analysis::from_bytes(&bytes).unwrap();
        assert_eq!(
            read.turns(&Options::default()),
            analysis.turns(&Options::default())
        );
        let forced = Options {
            speakers: Some(2),
            ..Options::default()
        };
        assert_eq!(read.turns(&forced), analysis.turns(&forced));
        // Fichier tronqué, modifié ou étranger : refusé.
        assert!(Analysis::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(Analysis::from_bytes(b"autre chose").is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Analysis::from_bytes(&longer).is_err());
    }

    #[test]
    fn many_embeddings_are_clustered_on_a_sample() {
        // Plus de MAX_CLUSTERED empreintes : deux voix bien séparées.
        let mut analysis = Analysis::new(SECOND);
        for i in 0..MAX_CLUSTERED + 500 {
            analysis.windows.push(Window {
                start: i * SECOND,
                activity: vec![[true, false, false]; 30],
            });
            analysis.owners.push((i, 0));
            let voice = if (i / 50) % 2 == 0 {
                [1.0, 0.1]
            } else {
                [0.1, 1.0]
            };
            analysis.embeddings.push(normalize(voice.to_vec()));
        }
        analysis.end = (MAX_CLUSTERED + 510) * SECOND;
        let speakers: std::collections::BTreeSet<usize> = analysis
            .turns(&Options::default())
            .iter()
            .map(|t| t.speaker)
            .collect();
        assert_eq!(speakers.len(), 2);
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

    fn turn(start_ms: u64, end_ms: u64, speaker: usize) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    #[test]
    fn a_passage_with_one_speaker_stays_whole() {
        let turns = [turn(0, 4_000, 0), turn(4_000, 9_000, 1)];
        let lines = split("Bonjour à tous.", 500, 3_500, &turns);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, Some(0));
        assert_eq!((lines[0].start_ms, lines[0].end_ms), (500, 3_500));
        // Aucun tour dans le passage : le locuteur le plus proche.
        assert_eq!(split("Euh", 9_500, 9_800, &turns)[0].speaker, Some(1));
        assert_eq!(split("Euh", 0, 100, &[])[0].speaker, None);
    }

    #[test]
    fn a_passage_is_shared_by_speaking_time() {
        // A parle 2 s, B 6 s : un quart des mots pour A.
        let turns = [turn(0, 2_000, 0), turn(2_000, 8_000, 1)];
        let text = "un deux trois quatre cinq six sept huit";
        let lines = split(text, 0, 8_000, &turns);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "un deux");
        assert_eq!(lines[0].speaker, Some(0));
        assert_eq!(lines[1].text, "trois quatre cinq six sept huit");
        assert_eq!(lines[1].speaker, Some(1));
        assert_eq!((lines[0].start_ms, lines[1].end_ms), (0, 8_000));
        assert_eq!(lines[1].start_ms, 2_000);
        // Rien n'est perdu ni dédoublé.
        let joined: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(joined.join(" "), text);
    }

    #[test]
    fn overlapping_turns_do_not_duplicate_words() {
        let turns = [
            turn(0, 5_000, 0),
            turn(3_000, 6_000, 1),
            turn(6_000, 9_000, 0),
        ];
        let text = "a b c d e f g h i";
        let lines = split(text, 0, 9_000, &turns);
        let speakers: Vec<Option<usize>> = lines.iter().map(|l| l.speaker).collect();
        assert_eq!(speakers, vec![Some(0), Some(1), Some(0)]);
        let joined: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(joined.join(" "), text);
    }
}
