#!/usr/bin/env bash
# Décide ce que la CI doit vérifier d'après les fichiers modifiés.
#
# Sorties (GITHUB_OUTPUT) :
#   code=true|false     application PC (jobs linux et windows) à vérifier
#   packages=...        arguments cargo pour clippy/test (`--workspace` ou `-p x -p y`)
#   android=true|false  application Android (job android) à vérifier
#
# Une PR qui ne touche que l'application Android ne relance pas les
# vérifications du PC, et inversement ; les crates partagés (vtt-core,
# vtt-llm, vtt-asr) relancent les deux.
#
# Règle de sécurité : au moindre doute (pas une PR, historique introuvable,
# fichier non classé), tout est vérifié. Sur `main` et les tags, toujours tout.
set -euo pipefail

out="${GITHUB_OUTPUT:-/dev/stdout}"

everything() {
  echo "code=true" >>"$out"
  echo "packages=--workspace" >>"$out"
  echo "android=true" >>"$out"
  echo "Vérification complète : $1"
  exit 0
}

[ "${GITHUB_EVENT_NAME:-}" = "pull_request" ] || everything "événement ${GITHUB_EVENT_NAME:-inconnu}"
[ -n "${BASE_SHA:-}" ] && [ -n "${HEAD_SHA:-}" ] || everything "commits de la PR inconnus"
files=$(git diff --name-only "$BASE_SHA...$HEAD_SHA") || everything "historique introuvable"
[ -n "$files" ] || everything "aucun fichier modifié détecté"

# Chaque PR monte la version (Cargo.toml, et Cargo.lock pour les crates du
# projet) : si c'est le seul changement de ces fichiers, il ne demande pas
# de tout revérifier.
project_version() {
  git show "$1:Cargo.toml" | grep -m1 '^version = ' | cut -d'"' -f2 | sed 's/\./\\./g'
}
only_version_bump() {
  local changed old new
  old=$(project_version "$BASE_SHA") && new=$(project_version "$HEAD_SHA") || return 1
  [ -n "$old" ] && [ -n "$new" ] || return 1
  changed=$(git diff --unified=0 "$BASE_SHA...$HEAD_SHA" -- "$1" | grep -E '^[+-]' | grep -vE '^(\+\+\+|---) ') || return 1
  # Seules lignes acceptées : la version du projet, ancienne ou nouvelle.
  [ -n "$changed" ] && ! grep -qvE "^[+-]version = \"($old|$new)\"$" <<<"$changed"
}

code=false
android=false
# Tout le PC à vérifier (crate utilisé partout, exemple du README).
workspace=false
declare -A packages=()
while IFS= read -r file; do
  case "$file" in
    # Le README contient un exemple de configuration testé par vtt-core.
    README.md)
      code=true
      workspace=true
      ;;
    *.md | docs/* | LICENSE* | .gitignore) ;;
    Cargo.toml | Cargo.lock)
      only_version_bump "$file" || everything "$file modifié au-delà de la version"
      ;;
    apps/android/* | crates/vtt-android/*) android=true ;;
    crates/vtt-core/*)
      code=true
      workspace=true
      android=true
      ;;
    # Crates partagés par le PC et le téléphone.
    crates/vtt-llm/* | crates/vtt-asr/*)
      code=true
      android=true
      crate="${file#crates/}"
      packages["${crate%%/*}"]=1
      packages[voicetotext]=1
      ;;
    crates/*/*)
      code=true
      crate="${file#crates/}"
      packages["${crate%%/*}"]=1
      packages[voicetotext]=1
      ;;
    apps/desktop/*)
      code=true
      packages[voicetotext]=1
      ;;
    *) everything "fichier commun modifié : $file" ;;
  esac
done <<<"$files"

if [ "$workspace" = true ]; then
  args="--workspace"
else
  args=""
  for p in "${!packages[@]}"; do args+=" -p $p"; done
  args="${args# }"
fi
echo "code=$code" >>"$out"
echo "packages=$args" >>"$out"
echo "android=$android" >>"$out"
echo "Fichiers modifiés :"
echo "$files" | sed 's/^/  /'
echo "PC : $code (${args:-rien}) ; Android : $android"
