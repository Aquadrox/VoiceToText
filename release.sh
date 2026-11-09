#!/usr/bin/env bash
# Publie une version de VoiceToText : pose le tag vX.Y.Z sur le dernier commit
# de `main` (sur GitHub) et le pousse. La CI compile, signe et publie alors la
# release (installeur PC, APK, manifestes de mise à jour).
#
# Usage, depuis n'importe quel dossier du dépôt (sous Windows : Git Bash) :
#   ./release.sh            version lue dans le Cargo.toml de main
#   ./release.sh 0.23.0     idem, en vérifiant que c'est bien celle attendue
#
# Rien n'est publié sans confirmation. Le dossier local n'est pas modifié
# (pas de checkout) : seul le tag est créé, sur le commit de origin/main.
set -euo pipefail

remote=origin
cd "$(dirname "$0")"

fail() {
  echo "Erreur : $*" >&2
  exit 1
}

case "${1:-}" in
  -h | --help)
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac
wanted="${1:-}"
wanted="${wanted#v}"

echo "Récupération de main et des tags depuis GitHub…"
git fetch --quiet --tags "$remote" main || fail "impossible de joindre GitHub ($remote)"
commit=$(git rev-parse --verify "$remote/main^{commit}")

# Version du code publié : celle du Cargo.toml de main (la CI refuse un tag
# qui ne lui correspond pas).
version=$(git show "$commit:Cargo.toml" | grep -m1 '^version = ' | cut -d'"' -f2)
[ -n "$version" ] || fail "version introuvable dans le Cargo.toml de main"
if [ -n "$wanted" ] && [ "$wanted" != "$version" ]; then
  fail "main est en version $version, pas $wanted. La PR qui monte la version est-elle mergée ?"
fi
tag="v$version"

# Déjà publiée : il faut une nouvelle version (montée dans une PR).
if git ls-remote --exit-code --tags "$remote" "refs/tags/$tag" >/dev/null; then
  fail "$tag est déjà publiée. Rien de nouveau à publier : la prochaine PR doit monter la version."
fi

# Version plus ancienne que la dernière publiée : main n'est pas à jour ?
latest=$(git tag --list 'v[0-9]*' --sort=-v:refname | { grep -vxF "$tag" || true; } | sed -n 1p)
if [ -n "$latest" ] && [ "$(printf '%s\n%s\n' "$latest" "$tag" | sort -V | tail -n1)" != "$tag" ]; then
  fail "$tag est plus ancienne que la dernière version publiée ($latest)"
fi

# Tag local laissé par une tentative précédente (push échoué) : repris s'il
# désigne le même commit.
if existing=$(git rev-parse -q --verify "refs/tags/$tag^{commit}"); then
  [ "$existing" = "$commit" ] ||
    fail "le tag local $tag désigne un autre commit ; le supprimer avec : git tag -d $tag"
fi

# Notes de version : la CI les exige et les affiche dans l'application.
notes=$(git show "$commit:CHANGELOG.md" |
  awk -v head="## $version — " 'index($0, head) == 1 { on = 1; next } on && /^## / { exit } on')
[ -n "$(printf '%s' "$notes" | tr -d '[:space:]')" ] ||
  fail "CHANGELOG.md de main n'a pas de section « ## $version — AAAA-MM-JJ »"

url=$(git remote get-url "$remote" | sed -E 's#^git@github\.com:#https://github.com/#; s#\.git$##')

echo
echo "Version   : $tag"
echo "Commit    : $(git log -1 --format='%h %s' "$commit")"
[ -n "$latest" ] && echo "Précédente : $latest ($(git rev-list --count "$latest..$commit") commit(s) depuis)"
echo "Nouveautés :"
printf '%s\n' "$notes" | sed '/^[[:space:]]*$/d; s/^/  /'
echo
read -r -p "Publier $tag ? [o/N] " answer || answer=""
case "$answer" in
  [oOyY]*) ;;
  *)
    echo "Annulé : rien n'a été publié."
    exit 0
    ;;
esac

[ -n "${existing:-}" ] || git tag "$tag" "$commit"
git push "$remote" "refs/tags/$tag" || fail "envoi du tag refusé ; relancer ./release.sh (le tag local sera repris)"

echo
echo "Tag $tag envoyé. La CI vérifie tout, puis compile, signe et publie (environ 15 min)."
echo "Suivi   : $url/actions"
echo "Release : $url/releases/tag/$tag"
