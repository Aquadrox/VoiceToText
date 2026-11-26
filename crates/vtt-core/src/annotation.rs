//! Capture commentée : choisir les moments clés (mots « ici », « là »,
//! traits dessinés), y placer des repères dans le texte, dessiner les traits
//! sur les captures d'écran et les réunir en une planche.
//!
//! Images en RGBA 8 bits ; l'encodage (PNG) et la capture d'écran vivent
//! dans d'autres crates.

use serde::{Deserialize, Serialize};

use crate::pipeline::Word;

/// Image RGBA (4 octets par pixel, ligne par ligne).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, color: [u8; 4]) -> Self {
        Self {
            width,
            height,
            rgba: color.repeat((width * height) as usize),
        }
    }

    fn put(&mut self, x: i64, y: i64, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.rgba[i..i + 4].copy_from_slice(&color);
    }

    fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, color);
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, radius: f32, color: [u8; 4]) {
        let r = radius.ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                if (dx * dx + dy * dy) as f32 <= radius * radius {
                    self.put(cx as i64 + dx, cy as i64 + dy, color);
                }
            }
        }
    }

    /// Copie réduite (moyenne des pixels), au plus `max_width` de large.
    pub fn scaled_to(&self, max_width: u32) -> Image {
        if self.width <= max_width {
            return self.clone();
        }
        let width = max_width;
        let height =
            (u64::from(self.height) * u64::from(width) / u64::from(self.width)).max(1) as u32;
        let mut out = Image::new(width, height, [0, 0, 0, 255]);
        let fx = self.width as f32 / width as f32;
        let fy = self.height as f32 / height as f32;
        for y in 0..height {
            for x in 0..width {
                let (x0, y0) = ((x as f32 * fx) as u32, (y as f32 * fy) as u32);
                let x1 = (((x + 1) as f32 * fx) as u32).clamp(x0 + 1, self.width);
                let y1 = (((y + 1) as f32 * fy) as u32).clamp(y0 + 1, self.height);
                let mut sum = [0u32; 4];
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        for (s, c) in sum.iter_mut().zip(self.get(sx, sy)) {
                            *s += u32::from(c);
                        }
                    }
                }
                let n = (x1 - x0) * (y1 - y0);
                out.put(i64::from(x), i64::from(y), sum.map(|s| (s / n) as u8));
            }
        }
        out
    }

    /// Partie de l'image (bornée à l'image).
    pub fn crop(&self, region: &Region) -> Image {
        let x = region.x.min(self.width.saturating_sub(1));
        let y = region.y.min(self.height.saturating_sub(1));
        let width = region.width.clamp(1, self.width - x);
        let height = region.height.clamp(1, self.height - y);
        let mut out = Image::new(width, height, [0, 0, 0, 255]);
        for row in 0..height {
            let src = (((y + row) * self.width + x) * 4) as usize;
            let dst = (row * width * 4) as usize;
            out.rgba[dst..dst + width as usize * 4]
                .copy_from_slice(&self.rgba[src..src + width as usize * 4]);
        }
        out
    }

    fn blit(&mut self, other: &Image, x: u32, y: u32) {
        for row in 0..other.height.min(self.height.saturating_sub(y)) {
            let width = other.width.min(self.width.saturating_sub(x)) as usize * 4;
            let src = (row * other.width * 4) as usize;
            let dst = (((y + row) * self.width + x) * 4) as usize;
            self.rgba[dst..dst + width].copy_from_slice(&other.rgba[src..src + width]);
        }
    }
}

/// Zone capturée, en pixels de l'écran (fenêtre active ou zone choisie).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// Point de l'écran ramené dans la zone.
    pub fn local(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (x - self.x as f32, y - self.y as f32)
    }

    /// Rectangle de masquage en coordonnées écran ramené dans la zone ;
    /// `None` s'il ne recoupe pas la zone.
    pub fn local_rect(&self, r: &Redaction) -> Option<Redaction> {
        let rx0 = self.x;
        let ry0 = self.y;
        let rx1 = self.x.saturating_add(self.width);
        let ry1 = self.y.saturating_add(self.height);
        let x0 = r.x.max(rx0);
        let y0 = r.y.max(ry0);
        let x1 = r.x.saturating_add(r.width).min(rx1);
        let y1 = r.y.saturating_add(r.height).min(ry1);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Redaction {
            x: x0 - rx0,
            y: y0 - ry0,
            width: x1 - x0,
            height: y1 - y0,
            style: r.style,
        })
    }
}

/// Style de masquage d'une zone sensible (clés API, mails, données perso).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactStyle {
    /// Flou fort irréversible (sous-échantillonnage + 3 passes de flou).
    #[default]
    #[serde(alias = "flou")]
    Blur,
    /// Mosaïque grossière (blocs moyens).
    #[serde(alias = "pixel")]
    Pixel,
    /// Bandeau opaque couleur encre (zéro information résiduelle).
    #[serde(alias = "plein")]
    Solid,
}

impl RedactStyle {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "pixel" | "pixelliser" => Self::Pixel,
            "plein" | "solid" | "noir" => Self::Solid,
            _ => Self::Blur,
        }
    }
}

/// Zone sensible à masquer sur une capture, en pixels de l'image ou de l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redaction {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub style: RedactStyle,
}

/// Couleur du bandeau opaque (encre de l'identité VoiceToText).
const REDACT_SOLID: [u8; 4] = [24, 29, 39, 255];

/// Applique les zones de masquage sur `image` avant annotation, enregistrement,
/// collage ou envoi au LLM vision. Bornée strictement à chaque rectangle.
pub fn apply_redactions(image: &mut Image, redactions: &[Redaction]) {
    if image.width == 0 || image.height == 0 {
        return;
    }
    for r in redactions {
        let x0 = r.x.min(image.width);
        let y0 = r.y.min(image.height);
        let x1 = r.x.saturating_add(r.width).min(image.width);
        let y1 = r.y.saturating_add(r.height).min(image.height);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        match r.style {
            RedactStyle::Solid => {
                image.fill_rect(
                    i64::from(x0),
                    i64::from(y0),
                    i64::from(x1 - x0),
                    i64::from(y1 - y0),
                    REDACT_SOLID,
                );
            }
            RedactStyle::Pixel => pixelate_rect(image, x0, y0, x1, y1),
            RedactStyle::Blur => blur_rect(image, x0, y0, x1, y1),
        }
    }
}

fn pixelate_rect(image: &mut Image, x0: u32, y0: u32, x1: u32, y1: u32) {
    let w = x1 - x0;
    let h = y1 - y0;
    let block = (w.min(h) / 4).clamp(10, 24);
    let mut by = y0;
    while by < y1 {
        let by1 = (by + block).min(y1);
        let mut bx = x0;
        while bx < x1 {
            let bx1 = (bx + block).min(x1);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for y in by..by1 {
                for x in bx..bx1 {
                    let px = image.get(x, y);
                    for c in 0..4 {
                        sum[c] += u64::from(px[c]);
                    }
                    count += 1;
                }
            }
            let avg = sum.map(|s| (s / count.max(1)) as u8);
            for y in by..by1 {
                for x in bx..bx1 {
                    image.put(i64::from(x), i64::from(y), avg);
                }
            }
            bx = bx1;
        }
        by = by1;
    }
}

/// Flou fort irréversible : d'abord une moyenne par gros blocs (détruit les
/// glyphes fins), puis 3 passes de flou séparable (approximation gaussienne)
/// strictement confinées au rectangle `[x0..x1, y0..y1]`.
fn blur_rect(image: &mut Image, x0: u32, y0: u32, x1: u32, y1: u32) {
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    // Pré-moyennage grossier pour empêcher toute reconstruction de caractères.
    let cell = (w.min(h) / 6).clamp(6, 14) as u32;
    let mut by = y0;
    while by < y1 {
        let by1 = (by + cell).min(y1);
        let mut bx = x0;
        while bx < x1 {
            let bx1 = (bx + cell).min(x1);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for y in by..by1 {
                for x in bx..bx1 {
                    let px = image.get(x, y);
                    for c in 0..4 {
                        sum[c] += u64::from(px[c]);
                    }
                    count += 1;
                }
            }
            let avg = sum.map(|s| (s / count.max(1)) as u8);
            for y in by..by1 {
                for x in bx..bx1 {
                    image.put(i64::from(x), i64::from(y), avg);
                }
            }
            bx = bx1;
        }
        by = by1;
    }

    let radius = (w.min(h) / 4).clamp(6, 18);
    let mut buf = vec![[0u8; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            buf[y * w + x] = image.get(x0 + x as u32, y0 + y as u32);
        }
    }
    let mut tmp = vec![[0u8; 4]; w * h];

    for _ in 0..3 {
        // Passe horizontale.
        for y in 0..h {
            let row = y * w;
            let mut sum = [0u32; 4];
            let first = buf[row];
            let last = buf[row + w - 1];
            for dx in -(radius as isize)..=(radius as isize) {
                let px = if dx < 0 {
                    first
                } else if (dx as usize) >= w {
                    last
                } else {
                    buf[row + dx as usize]
                };
                for c in 0..4 {
                    sum[c] += u32::from(px[c]);
                }
            }
            let span = (2 * radius + 1) as u32;
            for x in 0..w {
                tmp[row + x] = sum.map(|s| (s / span) as u8);
                let out_x = x as isize - radius as isize;
                let in_x = x + radius + 1;
                let out_px = if out_x < 0 {
                    first
                } else {
                    buf[row + out_x as usize]
                };
                let in_px = if in_x >= w { last } else { buf[row + in_x] };
                for c in 0..4 {
                    sum[c] = sum[c] + u32::from(in_px[c]) - u32::from(out_px[c]);
                }
            }
        }
        // Passe verticale.
        for x in 0..w {
            let mut sum = [0u32; 4];
            let first = tmp[x];
            let last = tmp[(h - 1) * w + x];
            for dy in -(radius as isize)..=(radius as isize) {
                let px = if dy < 0 {
                    first
                } else if (dy as usize) >= h {
                    last
                } else {
                    tmp[dy as usize * w + x]
                };
                for c in 0..4 {
                    sum[c] += u32::from(px[c]);
                }
            }
            let span = (2 * radius + 1) as u32;
            for y in 0..h {
                buf[y * w + x] = sum.map(|s| (s / span) as u8);
                let out_y = y as isize - radius as isize;
                let in_y = y + radius + 1;
                let out_px = if out_y < 0 {
                    first
                } else {
                    tmp[out_y as usize * w + x]
                };
                let in_px = if in_y >= h { last } else { tmp[in_y * w + x] };
                for c in 0..4 {
                    sum[c] = sum[c] + u32::from(in_px[c]) - u32::from(out_px[c]);
                }
            }
        }
    }

    for y in 0..h {
        for x in 0..w {
            image.put(
                i64::from(x0 + x as u32),
                i64::from(y0 + y as u32),
                buf[y * w + x],
            );
        }
    }
}

/// Clic pendant la capture, en pixels de l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    pub at_ms: u64,
}

/// Trait dessiné pendant la capture, en pixels de l'écran capturé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<(f32, f32)>,
    /// Fin du trait, depuis le début de la capture.
    pub end_ms: u64,
}

const INK: [u8; 4] = [230, 30, 60, 255];
const INK_HALO: [u8; 4] = [255, 255, 255, 255];

/// Dessine les traits sur l'image (rouge, liseré blanc pour rester visible
/// sur tous les fonds), assez épais pour rester lisibles sur la planche
/// réduite.
pub fn draw_strokes(image: &mut Image, strokes: &[&Stroke]) {
    let thickness = (image.width as f32 / 300.0).clamp(3.0, 10.0);
    for (radius, color) in [(thickness + 1.5, INK_HALO), (thickness, INK)] {
        for stroke in strokes {
            for pair in stroke.points.windows(2) {
                let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                let steps = ((x1 - x0).abs().max((y1 - y0).abs()) / (radius / 2.0))
                    .ceil()
                    .max(1.0);
                for i in 0..=steps as u32 {
                    let t = i as f32 / steps;
                    image.disc(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, radius, color);
                }
            }
            if let [(x, y)] = stroke.points[..] {
                image.disc(x, y, radius, color);
            }
        }
    }
}

/// Cercle autour d'un clic (rouge, liseré blanc).
pub fn draw_click(image: &mut Image, (x, y): (f32, f32)) {
    let radius = (image.width as f32 / 70.0).clamp(14.0, 40.0);
    let thickness = (radius / 6.0).max(3.0);
    for (width, color) in [(thickness + 3.0, INK_HALO), (thickness, INK)] {
        let steps = (radius * 8.0) as u32;
        for i in 0..steps {
            let angle = i as f32 / steps as f32 * std::f32::consts::TAU;
            image.disc(
                x + radius * angle.cos(),
                y + radius * angle.sin(),
                width / 2.0,
                color,
            );
        }
    }
}

/// Mots qui désignent quelque chose à l'écran.
const POINTING: &[&str] = &[
    "ici", "là", "ça", "ceci", "cela", "celui-ci", "celle-ci", "regarde", "regardez", "voilà",
    "voici", "this", "here", "there", "look",
];
/// Deux moments plus proches sont fusionnés.
const MIN_GAP_MS: u64 = 1_500;
/// Au plus ce nombre d'images.
pub const MAX_MOMENTS: usize = 6;

fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_lowercase()
}

/// Moments à illustrer : fin de chaque trait (priorité), clics, mots qui
/// désignent l'écran, et la fin de la capture s'il n'y a rien d'autre.
/// Triés, espacés d'au moins 1,5 s, au plus [`MAX_MOMENTS`].
pub fn key_moments(
    words: &[Word],
    strokes: &[Stroke],
    clicks: &[Click],
    duration_ms: u64,
) -> Vec<u64> {
    let mut chosen: Vec<u64> = Vec::new();
    let add = |at: u64, chosen: &mut Vec<u64>| {
        if chosen.len() < MAX_MOMENTS && chosen.iter().all(|c| c.abs_diff(at) >= MIN_GAP_MS) {
            chosen.push(at);
        }
    };
    for stroke in strokes {
        add(stroke.end_ms, &mut chosen);
    }
    for click in clicks {
        add(click.at_ms, &mut chosen);
    }
    for word in words {
        if POINTING.contains(&normalize(&word.text).as_str()) {
            add(word.end_ms, &mut chosen);
        }
    }
    if chosen.is_empty() {
        chosen.push(duration_ms);
    }
    chosen.sort_unstable();
    chosen
}

/// Écran le plus récent au moment `at_ms` (captures horodatées, triées).
pub fn frame_at(frames: &[u64], at_ms: u64) -> Option<usize> {
    frames
        .iter()
        .rposition(|&t| t <= at_ms)
        .or(if frames.is_empty() { None } else { Some(0) })
}

/// Le mot n'est que de la ponctuation (Parakeet la donne à part).
fn is_punctuation(word: &str) -> bool {
    word.chars().all(|c| !c.is_alphanumeric())
}

/// Texte de la narration avec un repère `[image n]` à chaque moment clé.
pub fn transcript_with_markers(words: &[Word], moments: &[u64]) -> String {
    let mut text = String::new();
    let mut next = 0;
    for word in words {
        if !text.is_empty() && !is_punctuation(&word.text) {
            text.push(' ');
        }
        text.push_str(&word.text);
        while next < moments.len() && moments[next] <= word.end_ms {
            next += 1;
            text.push_str(&format!(" [image {next}]"));
        }
    }
    while next < moments.len() {
        next += 1;
        text.push_str(&format!(" [image {next}]"));
    }
    text.trim().to_owned()
}

/// Chiffres 3×5 pour numéroter les images sans police.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Pastille numérotée dans le coin haut gauche.
fn badge(image: &mut Image, number: usize) {
    let digits: Vec<usize> = number
        .to_string()
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as usize))
        .collect();
    let pixel = 8;
    let width = (digits.len() * 4 + 1) as i64 * pixel;
    let height = 7 * pixel;
    image.fill_rect(8, 8, width + 2 * pixel, height, INK);
    for (n, digit) in digits.iter().enumerate() {
        for (row, bits) in DIGITS[*digit].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    image.fill_rect(
                        8 + (2 + n as i64 * 4 + col) * pixel,
                        8 + (1 + row as i64) * pixel,
                        pixel,
                        pixel,
                        [255, 255, 255, 255],
                    );
                }
            }
        }
    }
}

/// Planche : les images numérotées, réduites, sur deux colonnes (une seule
/// image : telle quelle, réduite).
pub fn contact_sheet(images: &[Image]) -> Option<Image> {
    const CELL_WIDTH: u32 = 960;
    const GAP: u32 = 12;
    let cells: Vec<Image> = images
        .iter()
        .enumerate()
        .map(|(i, image)| {
            let mut cell = image.scaled_to(if images.len() == 1 { 1600 } else { CELL_WIDTH });
            badge(&mut cell, i + 1);
            cell
        })
        .collect();
    let columns = if cells.len() == 1 { 1 } else { 2 };
    let cell_width = cells.iter().map(|c| c.width).max()?;
    let cell_height = cells.iter().map(|c| c.height).max()?;
    let rows = cells.len().div_ceil(columns) as u32;
    let columns = columns as u32;
    let mut sheet = Image::new(
        columns * cell_width + (columns + 1) * GAP,
        rows * cell_height + (rows + 1) * GAP,
        [245, 245, 248, 255],
    );
    for (i, cell) in cells.iter().enumerate() {
        let (col, row) = (i as u32 % columns, i as u32 / columns);
        sheet.blit(
            cell,
            GAP + col * (cell_width + GAP),
            GAP + row * (cell_height + GAP),
        );
    }
    Some(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start_ms: u64) -> Word {
        Word {
            text: text.into(),
            start_ms,
            end_ms: start_ms + 300,
        }
    }

    #[test]
    fn moments_follow_strokes_then_pointing_words() {
        let words = vec![
            word("Le", 0),
            word("bouton", 400),
            word("ici,", 800),
            word("ne", 1_200),
            word("marche", 1_600),
            word("pas.", 2_000),
            word("Et", 5_000),
            word("là", 5_400),
            word("non", 5_800),
            word("plus.", 6_200),
        ];
        let strokes = vec![Stroke {
            points: vec![(10.0, 10.0), (50.0, 10.0)],
            end_ms: 1_500,
        }];
        // Trait à 1,5 s ; « ici » (1,1 s) trop proche ; « là » à 5,7 s.
        assert_eq!(key_moments(&words, &strokes, &[], 7_000), [1_500, 5_700]);
        assert_eq!(key_moments(&words[..2], &[], &[], 3_000), [3_000]);
        let click = Click {
            x: 0.0,
            y: 0.0,
            at_ms: 3_200,
        };
        assert_eq!(
            key_moments(&words, &strokes, &[click], 7_000),
            [1_500, 3_200, 5_700]
        );
        assert_eq!(
            transcript_with_markers(&words, &[1_500, 5_700]),
            "Le bouton ici, ne [image 1] marche pas. Et là [image 2] non plus."
        );
    }

    #[test]
    fn punctuation_tokens_stick_to_the_previous_word() {
        let words = vec![
            word("Bonjour", 0),
            word(",", 300),
            word("ici", 400),
            word(".", 700),
        ];
        assert_eq!(transcript_with_markers(&words, &[]), "Bonjour, ici.");
        assert_eq!(transcript_with_markers(&[], &[10]), "[image 1]");
    }

    #[test]
    fn the_latest_frame_before_each_moment_is_used() {
        let frames = [0, 1_000, 4_000];
        assert_eq!(frame_at(&frames, 3_999), Some(1));
        assert_eq!(frame_at(&frames, 4_000), Some(2));
        assert_eq!(frame_at(&[500], 100), Some(0));
        assert_eq!(frame_at(&[], 100), None);
    }

    #[test]
    fn strokes_are_drawn_and_sheets_are_assembled() {
        let mut image = Image::new(400, 200, [0, 0, 0, 255]);
        let stroke = Stroke {
            points: vec![(20.0, 100.0), (380.0, 100.0)],
            end_ms: 0,
        };
        draw_strokes(&mut image, &[&stroke]);
        assert_eq!(image.get(200, 100), INK);
        assert_eq!(image.get(200, 20), [0, 0, 0, 255]);

        let big = Image::new(1920, 1080, [10, 20, 30, 255]);
        let sheet = contact_sheet(&[big.clone(), big.clone(), big]).unwrap();
        assert_eq!(sheet.width, 2 * 960 + 3 * 12);
        assert_eq!(sheet.height, 2 * 540 + 3 * 12);
        // Pastille « 1 » dans le coin de la première image.
        assert_eq!(sheet.get(12 + 9, 12 + 9), INK);
        assert_eq!(contact_sheet(&[]), None);

        // Zone : fenêtre de 100 × 50 à (300, 100) ; clic dessiné dedans.
        let mut screen = Image::new(800, 600, [0, 0, 0, 255]);
        screen.put(300, 100, [9, 9, 9, 255]);
        let region = Region {
            x: 300,
            y: 100,
            width: 100,
            height: 50,
        };
        let mut window = screen.crop(&region);
        assert_eq!((window.width, window.height), (100, 50));
        assert_eq!(window.get(0, 0), [9, 9, 9, 255]);
        assert_eq!(region.local((350.0, 125.0)), (50.0, 25.0));
        draw_click(&mut window, (50.0, 25.0));
        assert_eq!(window.get(50, 25), [0, 0, 0, 255], "centre du cercle vide");
        assert_eq!(window.get(50 + 14, 25), INK);
        // Zone qui dépasse l'écran : bornée.
        let clipped = screen.crop(&Region {
            x: 780,
            y: 590,
            width: 100,
            height: 100,
        });
        assert_eq!((clipped.width, clipped.height), (20, 10));
        let single = contact_sheet(&[Image::new(800, 600, [1, 2, 3, 255])]).unwrap();
        assert_eq!(single.width, 800 + 24);
    }

    #[test]
    fn redactions_blur_pixel_solid_preserve_outside_and_destroy_inside() {
        // Damier noir/blanc très contrasté (simule du texte fin / clé API).
        let mut base = Image::new(80, 60, [255, 255, 255, 255]);
        for y in 0..60u32 {
            for x in 0..80u32 {
                if (x + y) % 2 == 0 {
                    base.put(i64::from(x), i64::from(y), [0, 0, 0, 255]);
                }
            }
        }

        // 1. Masque plein : couleur encre exacte dedans, damier intact dehors.
        let mut solid = base.clone();
        apply_redactions(
            &mut solid,
            &[Redaction {
                x: 10,
                y: 10,
                width: 40,
                height: 30,
                style: RedactStyle::Solid,
            }],
        );
        assert_eq!(solid.get(25, 25), REDACT_SOLID);
        assert_eq!(solid.get(9, 10), base.get(9, 10));
        assert_eq!(solid.get(50, 25), base.get(50, 25));

        // 2. Pixellisation : chaque bloc moyen tourne autour de 127 (plus de noir pur ni blanc pur).
        let mut pixel = base.clone();
        apply_redactions(
            &mut pixel,
            &[Redaction {
                x: 10,
                y: 10,
                width: 40,
                height: 30,
                style: RedactStyle::Pixel,
            }],
        );
        let p = pixel.get(25, 25)[0];
        assert!((110..=145).contains(&p), "moyenne attendue ~127, reçu {p}");
        assert_eq!(pixel.get(5, 5), base.get(5, 5));

        // 3. Flou fort : les contrastes fins disparaissent complètement à l'intérieur.
        let mut blur = base.clone();
        apply_redactions(
            &mut blur,
            &[Redaction {
                x: 10,
                y: 10,
                width: 40,
                height: 30,
                style: RedactStyle::Blur,
            }],
        );
        let b = blur.get(25, 25)[0];
        assert!((110..=145).contains(&b), "flou attendu ~127, reçu {b}");
        assert_eq!(blur.get(0, 0), base.get(0, 0));
        assert_eq!(blur.get(79, 59), base.get(79, 59));

        // 4. Rectangles hors cadre / vides ignorés sans panique.
        apply_redactions(
            &mut blur,
            &[
                Redaction {
                    x: 200,
                    y: 200,
                    width: 20,
                    height: 20,
                    style: RedactStyle::Blur,
                },
                Redaction {
                    x: 10,
                    y: 10,
                    width: 0,
                    height: 20,
                    style: RedactStyle::Solid,
                },
            ],
        );

        // 5. Region::local_rect : translation et découpage propre.
        let region = Region {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        assert_eq!(
            region.local_rect(&Redaction {
                x: 80,
                y: 60,
                width: 50,
                height: 40,
                style: RedactStyle::Pixel,
            }),
            Some(Redaction {
                x: 0,
                y: 10,
                width: 30,
                height: 40,
                style: RedactStyle::Pixel,
            })
        );
        assert_eq!(
            region.local_rect(&Redaction {
                x: 0,
                y: 0,
                width: 50,
                height: 50,
                style: RedactStyle::Blur,
            }),
            None
        );
    }
}
