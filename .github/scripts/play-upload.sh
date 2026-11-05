#!/usr/bin/env bash
# Envoie l'AAB sur Google Play (API Android Publisher v3), sur la piste et
# avec le statut choisis par les variables du dépôt (PLAY_TRACK, PLAY_STATUS).
#
# La clé du compte de service est lue dans PLAY_SERVICE_ACCOUNT_JSON (secret
# GitHub) : jamais écrite sur le disque, jamais affichée. Seuls openssl, curl
# et jq servent : aucune action tierce ne voit la clé.
set -euo pipefail

aab="$1"
tag="$2"
package="${PLAY_PACKAGE:-io.github.aquadrox.voicetotext}"
# « alpha » : la piste de test fermé créée par défaut dans la Play Console.
track="${PLAY_TRACK:-alpha}"
# « draft » tant que l'application n'a jamais été publiée, puis « completed ».
status="${PLAY_STATUS:-completed}"
: "${PLAY_SERVICE_ACCOUNT_JSON:?secret PLAY_SERVICE_ACCOUNT_JSON absent}"

b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

# Jeton OAuth du compte de service : JWT signé RS256, valable 10 minutes.
email=$(printf "%s" "$PLAY_SERVICE_ACCOUNT_JSON" | jq -r .client_email)
now=$(date +%s)
header=$(printf '{"alg":"RS256","typ":"JWT"}' | b64url)
claims=$(jq -cn --arg iss "$email" --argjson iat "$now" '{
  iss: $iss,
  scope: "https://www.googleapis.com/auth/androidpublisher",
  aud: "https://oauth2.googleapis.com/token",
  iat: $iat,
  exp: ($iat + 600)
}' | b64url)
signature=$(printf '%s.%s' "$header" "$claims" |
  openssl dgst -sha256 -sign <(printf "%s" "$PLAY_SERVICE_ACCOUNT_JSON" | jq -r .private_key) | b64url)
token=$(curl -sS --fail-with-body https://oauth2.googleapis.com/token \
  -d grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer \
  -d "assertion=$header.$claims.$signature" | jq -r .access_token)
echo "::add-mask::$token"
auth=(-H "Authorization: Bearer $token")

api="https://androidpublisher.googleapis.com/androidpublisher/v3/applications/$package"
upload="https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications/$package"

# Une « édition » regroupe les changements, appliqués d'un coup au commit.
edit=$(curl -sS --fail-with-body "${auth[@]}" -X POST "$api/edits" | jq -r .id)
code=$(curl -sS --fail-with-body "${auth[@]}" \
  -H "Content-Type: application/octet-stream" --data-binary @"$aab" \
  "$upload/edits/$edit/bundles?uploadType=media" | jq -r .versionCode)
echo "AAB envoyé : version $code"

notes="Nouveautés : https://github.com/${GITHUB_REPOSITORY:-Aquadrox/VoiceToText}/releases/tag/$tag"
release=$(jq -cn --arg track "$track" --arg status "$status" --arg code "$code" \
  --arg name "${tag#v}" --arg notes "$notes" '{
  track: $track,
  releases: [{
    name: $name,
    versionCodes: [$code],
    status: $status,
    releaseNotes: [{language: "fr-FR", text: $notes}]
  }]
}')
curl -sS --fail-with-body "${auth[@]}" -X PUT -H "Content-Type: application/json" \
  -d "$release" "$api/edits/$edit/tracks/$track" >/dev/null
curl -sS --fail-with-body "${auth[@]}" -X POST "$api/edits/$edit:commit" >/dev/null
echo "Version $code publiée sur la piste « $track » ($status)"
