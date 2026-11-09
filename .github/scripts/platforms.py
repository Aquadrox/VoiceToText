#!/usr/bin/env python3
"""Plateformes touchées entre deux versions publiées.

Usage : platforms.py <ref de départ> <ref d'arrivée>

Affiche un objet JSON : {"windows": bool, "android": bool, "reasons": {...}}.
Le job `release` de la CI s'en sert pour n'annoncer une mise à jour qu'aux
plateformes dont l'application change : une version qui ne touche que
l'application Android n'est pas proposée sur Windows, et inversement.

Règle de sécurité (comme changes.sh) : au moindre doute (fichier non classé,
Cargo.toml ou Cargo.lock modifié au-delà de la version du projet), les deux
plateformes sont considérées comme touchées.
"""

import json
import subprocess
import sys
import tomllib

WINDOWS = "windows"
ANDROID = "android"
BOTH = (WINDOWS, ANDROID)


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def toml_at(ref: str, path: str) -> dict:
    try:
        return tomllib.loads(git("show", f"{ref}:{path}"))
    except subprocess.CalledProcessError:
        return {}


def manifest_changed(old: str, new: str) -> bool:
    """Cargo.toml modifié au-delà de la version du projet."""
    a, b = toml_at(old, "Cargo.toml"), toml_at(new, "Cargo.toml")
    for doc in (a, b):
        doc.get("workspace", {}).get("package", {}).pop("version", None)
    return a != b


def lock_changed(old: str, new: str) -> bool:
    """Cargo.lock modifié au-delà de la version des crates du projet.

    Les crates du projet n'ont pas de `source` dans Cargo.lock ; seule leur
    version est ignorée (une dépendance externe qui change compte toujours).
    """

    def packages(ref: str) -> list:
        result = []
        for pkg in toml_at(ref, "Cargo.lock").get("package", []):
            pkg = dict(pkg)
            if "source" not in pkg:
                pkg.pop("version", None)
            result.append(pkg)
        return sorted(result, key=lambda p: json.dumps(p, sort_keys=True))

    return packages(old) != packages(new)


def classify(path: str, old: str, new: str) -> tuple[str, ...]:
    """Plateformes dont l'application change avec ce fichier."""
    # Documentation et CI : l'application installée reste la même.
    if path.endswith(".md") or path.startswith(("docs/", ".github/")):
        return ()
    # Fichiers de Git et script de publication (lancé sur le PC du mainteneur).
    if path.startswith("LICENSE") or path in (".gitignore", ".gitattributes", "release.sh"):
        return ()
    if path == "Cargo.toml":
        return BOTH if manifest_changed(old, new) else ()
    if path == "Cargo.lock":
        return BOTH if lock_changed(old, new) else ()
    if path.startswith(("apps/android/", "crates/vtt-android/")):
        return (ANDROID,)
    if path.startswith(("apps/desktop/", "crates/vtt-platform/")):
        return (WINDOWS,)
    # Crates partagés (vtt-core, vtt-asr, vtt-llm), crate inconnu, fichier
    # commun (rust-toolchain.toml…) ou non classé : les deux.
    return BOTH


def main() -> None:
    old, new = sys.argv[1], sys.argv[2]
    files = git("diff", "--name-only", f"{old}...{new}").split()
    reasons: dict[str, list[str]] = {WINDOWS: [], ANDROID: []}
    for path in files:
        for platform in classify(path, old, new):
            reasons[platform].append(path)
    print(
        json.dumps(
            {WINDOWS: bool(reasons[WINDOWS]), ANDROID: bool(reasons[ANDROID]), "reasons": reasons},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
