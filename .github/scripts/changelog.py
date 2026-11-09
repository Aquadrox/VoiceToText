#!/usr/bin/env python3
"""Notes de version (CHANGELOG.md) pour la CI.

Usage :
  changelog.py check               vérifie le fichier ; la version du Cargo.toml
                                   racine doit avoir sa section
  changelog.py notes <version>     changements de cette version (release GitHub)
  changelog.py manifest <version>  sections jusqu'à cette version, pour le champ
                                   `changelog` des manifestes de mise à jour

Le texte des manifestes est relu par l'application (vtt-core, module
`changelog`) : elle n'affiche que les versions plus récentes que celle
installée, et seulement les changements de sa plateforme. Format :

  ## 0.22.0 — 2026-11-08

  - Changement visible sur le PC et sur le téléphone.
  - [PC] Changement propre à l'application Windows.
  - [Android] Changement propre au téléphone.
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHANGELOG = ROOT / "CHANGELOG.md"

HEADING = re.compile(r"^## +\[?v?(\d+)\.(\d+)\.(\d+)\]?(?=\s|$)")
DATE = re.compile(r"\b(\d{4})-(\d{2})-(\d{2})\b")
ITEM = re.compile(r"^\s*[-*] +(.*)$")
TAG = re.compile(r"^\[([^\]]*)\]")
TAGS = {"pc", "windows", "android"}

# Taille du champ `changelog` : le manifeste Android est refusé au-delà de
# 64 Kio ; les sections les plus anciennes sont retirées en premier.
MAX_SECTIONS = 20
MAX_BYTES = 24 * 1024


def version(text: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)", text.strip())
    if not match:
        sys.exit(f"version illisible : {text}")
    return tuple(int(part) for part in match.groups())


def sections(text: str) -> tuple[list[dict], list[str]]:
    """Sections `## X.Y.Z` (texte compris) et problèmes de format."""
    found, problems, current = [], [], None
    for number, line in enumerate(text.splitlines(), 1):
        if line.startswith("## "):
            match = HEADING.match(line)
            if not match:
                problems.append(f"ligne {number} : titre sans version X.Y.Z : {line}")
                current = None
                continue
            current = {
                "version": tuple(int(part) for part in match.groups()),
                "line": number,
                "lines": [line],
                "items": [],
            }
            found.append(current)
            if not DATE.search(line):
                problems.append(f"ligne {number} : date AAAA-MM-JJ manquante : {line}")
            continue
        if current is None:
            continue
        current["lines"].append(line)
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        item = ITEM.match(line)
        if item:
            current["items"].append(item.group(1))
            tag = TAG.match(item.group(1))
            if tag and tag.group(1).strip().lower() not in TAGS:
                problems.append(
                    f"ligne {number} : préfixe [{tag.group(1)}] inconnu ([PC] ou [Android])"
                )
        elif not current["items"]:
            problems.append(f"ligne {number} : texte hors liste (« - » attendu) : {line}")
    for section in found:
        if not any(item.strip() for item in section["items"]):
            problems.append(f"ligne {section['line']} : aucun changement listé")
        while section["lines"] and not section["lines"][-1].strip():
            section["lines"].pop()
    for previous, following in zip(found, found[1:]):
        if following["version"] >= previous["version"]:
            problems.append(
                f"ligne {following['line']} : versions de la plus récente à la plus ancienne"
            )
    return found, problems


def label(v: tuple[int, int, int]) -> str:
    return ".".join(map(str, v))


def find(found: list[dict], wanted: tuple[int, int, int]) -> dict:
    for section in found:
        if section["version"] == wanted:
            return section
    sys.exit(f"CHANGELOG.md : aucune section « ## {label(wanted)} — AAAA-MM-JJ »")


def texts(kept: list[dict]) -> list[str]:
    return ["\n".join(s["lines"]).strip() for s in kept]


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    command, args = sys.argv[1], sys.argv[2:]
    found, problems = sections(CHANGELOG.read_text(encoding="utf-8"))
    if command == "check":
        cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        current = version(cargo["workspace"]["package"]["version"])
        if not any(s["version"] == current for s in found):
            problems.append(
                f"version {label(current)} (Cargo.toml) sans section dans CHANGELOG.md : "
                f"ajouter « ## {label(current)} — AAAA-MM-JJ » et ses changements en tête"
            )
        elif found[0]["version"] != current:
            problems.append(f"la section {label(current)} doit être la première")
        for problem in problems:
            print(f"::error file=CHANGELOG.md::{problem}")
        if problems:
            sys.exit(1)
        print(f"CHANGELOG.md : {len(found)} versions, {label(current)} en tête")
    elif command == "notes" and len(args) == 1:
        section = find(found, version(args[0]))
        print("\n".join(section["lines"][1:]).strip())
    elif command == "manifest" and len(args) == 1:
        wanted = version(args[0])
        find(found, wanted)
        kept = [s for s in found if s["version"] <= wanted][:MAX_SECTIONS]
        while len(kept) > 1 and len("\n\n".join(texts(kept)).encode()) > MAX_BYTES:
            kept.pop()
        print("\n\n".join(texts(kept)))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
