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

    fn blit(&mut self, other: &Image, x: u32, y: u32) {
        for row in 0..other.height.min(self.height.saturating_sub(y)) {
            let width = other.width.min(self.width.saturating_sub(x)) as usize * 4;
            let src = (row * other.width * 4) as usize;
            let dst = (((y + row) * self.width + x) * 4) as usize;
            self.rgba[dst..dst + width].copy_from_slice(&other.rgba[src..src + width]);
        }
    }
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

/// Moments à illustrer : fin de chaque trait (priorité), mots qui désignent
/// l'écran, et la fin de la capture s'il n'y a rien d'autre. Triés, espacés
/// d'au moins 1,5 s, au plus [`MAX_MOMENTS`].
pub fn key_moments(words: &[Word], strokes: &[Stroke], duration_ms: u64) -> Vec<u64> {
    let mut chosen: Vec<u64> = Vec::new();
    let add = |at: u64, chosen: &mut Vec<u64>| {
        if chosen.len() < MAX_MOMENTS && chosen.iter().all(|c| c.abs_diff(at) >= MIN_GAP_MS) {
            chosen.push(at);
        }
    };
    for stroke in strokes {
        add(stroke.end_ms, &mut chosen);
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
        assert_eq!(key_moments(&words, &strokes, 7_000), [1_500, 5_700]);
        assert_eq!(key_moments(&words[..2], &[], 3_000), [3_000]);
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
        let single = contact_sheet(&[Image::new(800, 600, [1, 2, 3, 255])]).unwrap();
        assert_eq!(single.width, 800 + 24);
    }
}
