"""Visuels de la fiche Google Play, dessinés d'après l'icône de l'application
(micro blanc sur fond indigo, res/drawable/ic_launcher_foreground.xml).

    python3 apps/android/play/visuels.py

Produit icone-512.png (icône de la fiche) et banniere-1024x500.png (image
de présentation) à côté de ce script.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
INDIGO = (79, 70, 229)
INDIGO_DARK = (55, 48, 163)
WHITE = (255, 255, 255)
SCALE = 4  # dessin agrandi puis réduit : bords lisses


def microphone(draw: ImageDraw.ImageDraw, cx: float, cy: float, unit: float) -> None:
    """Micro de l'icône (coordonnées du vecteur 108×108, centré sur 54,54)."""

    def p(x: float, y: float) -> tuple[float, float]:
        return cx + (x - 54) * unit, cy + (y - 54) * unit

    # Capsule : x 45→63, y 30→64, rayon 9.
    draw.rounded_rectangle([p(45, 30), p(63, 64)], radius=9 * unit, fill=WHITE)
    width = round(4 * unit)
    # Arc : demi-cercle de rayon 16 centré sur (54, 52), vers le bas ; PIL
    # trace l'épaisseur vers l'intérieur du cadre, élargi d'autant.
    half = 2
    draw.arc([p(38 - half, 36 - half), p(70 + half, 68 + half)], start=0, end=180, fill=WHITE, width=width)
    draw.line([p(54, 68), p(54, 78)], fill=WHITE, width=width)
    draw.line([p(45, 78), p(63, 78)], fill=WHITE, width=width)
    for x, y in [(38, 52), (70, 52), (45, 78), (63, 78)]:
        r = width / 2
        X, Y = p(x, y)
        draw.ellipse([X - r, Y - r, X + r, Y + r], fill=WHITE)


def icon() -> None:
    size = 512 * SCALE
    image = Image.new("RGB", (size, size), INDIGO)
    # Zone sûre d'une icône adaptative : 66 sur 108 au centre.
    microphone(ImageDraw.Draw(image), size / 2, size / 2, size / 108 * 1.25)
    image.resize((512, 512), Image.LANCZOS).save(HERE / "icone-512.png")


def font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont:
    name = "DejaVuSans-Bold.ttf" if bold else "DejaVuSans.ttf"
    return ImageFont.truetype(f"/usr/share/fonts/truetype/dejavu/{name}", size)


def banner() -> None:
    w, h = 1024 * SCALE, 500 * SCALE
    image = Image.new("RGB", (w, h), INDIGO)
    draw = ImageDraw.Draw(image)
    # Dégradé discret vers le bas.
    for y in range(h):
        t = y / h
        color = tuple(round(a + (b - a) * t) for a, b in zip(INDIGO, INDIGO_DARK))
        draw.line([(0, y), (w, y)], fill=color)
    # Bulle de dictée à gauche.
    cx, cy, r = 250 * SCALE, 250 * SCALE, 150 * SCALE
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(99, 91, 255))
    microphone(draw, cx, cy, r / 54 * 1.15)
    x = 460 * SCALE
    draw.text((x, 160 * SCALE), "VoiceToText", font=font(68 * SCALE, bold=True), fill=WHITE)
    lines = ["Parlez, le texte s'écrit.", "Transcription sur le téléphone."]
    for i, line in enumerate(lines):
        draw.text((x, (262 + i * 46) * SCALE), line, font=font(32 * SCALE), fill=(224, 231, 255))
    image.resize((1024, 500), Image.LANCZOS).save(HERE / "banniere-1024x500.png")


if __name__ == "__main__":
    icon()
    banner()
