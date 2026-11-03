# VoiceToText

Dictée vocale universelle, inspirée de Wispr Flow : maintenir **AltGr**, parler, relâcher. Le texte transcrit et nettoyé apparaît dans l'application active.

- Transcription locale (Whisper sur GPU), français.
- Nettoyage du texte par LLM (DeepSeek, désactivable).
- Windows d'abord ; Linux et Android ensuite.

## Installation (Windows)

1. Télécharger l'installeur depuis la page **Releases** du dépôt (`VoiceToText_x.y.z_x64-setup.exe`).
2. Lancer `VoiceToText_x.y.z_x64-setup.exe` (installation pour l'utilisateur courant, sans droits admin).
3. Au premier lancement, le modèle Whisper (874 Mo) se télécharge : progression affichée dans la pastille en bas de l'écran.

## Utilisation

- **Maintenir AltGr**, parler, **relâcher** : le texte est collé dans l'application active.
- AltGr + une autre touche (`@`, `#`, `€`…) fonctionne normalement : la dictée est annulée.
- Icône de notification : état, historique des dictées, réglages, configuration, journaux, redémarrage.

### Nettoyage du texte (DeepSeek)

1. Créer une clé sur [platform.deepseek.com](https://platform.deepseek.com) (compte crédité).
2. Icône de notification → **Réglages…** → coller la clé → **Enregistrer** → **Tester**.

Sans clé, le texte brut de Whisper est inséré.

Configuration : `%APPDATA%\VoiceToText\config.toml` (redémarrer après modification).

```toml
# Noms propres, jargon : orthographe transmise à Whisper et au LLM.
# (clé de premier niveau : à placer avant toute section [..])
dictionary = ["Aquadrox", "Tauri"]

[hotkey]
key = "AltGr"        # ou RightCtrl, CapsLock, ScrollLock, Pause, Insert, Menu, F1…F24
min_hold_ms = 250

[asr]
model = "ggml-large-v3-turbo-q8_0.bin"
language = "fr"
use_gpu = true

[llm]
enabled = true
base_url = "https://api.deepseek.com"
model = "deepseek-flash"
timeout_ms = 3000

[history]
enabled = true       # dictées conservées sur ce PC uniquement
max_entries = 1000

# Snippets : dicter seulement le déclencheur insère le texte tel quel
# (sans LLM). Casse, accents et ponctuation ne comptent pas.
[[snippets]]
trigger = "ma signature"
text = """
Florian
VoiceToText"""

[[snippets]]
trigger = "mon adresse mail"
text = "prenom.nom@example.com"
```

Journaux : `%LOCALAPPDATA%\VoiceToText\logs\voicetotext.log` (purgé automatiquement au-delà de 3 Mo, les 100 dernières lignes sont gardées).

## Documentation

- [Fonctions](docs/FONCTIONS.md)
- [Architecture et choix techniques](docs/ARCHITECTURE.md)
- [Roadmap et avancement](docs/ROADMAP.md)

## Développement

```sh
cargo test --workspace
# Transcrire un WAV avec le pipeline complet (sans micro ni clavier) :
cargo run --release -p vtt-asr --example transcribe_wav -- <modèle.bin> <fichier.wav>
```

Les binaires Windows sont produits par la CI GitHub Actions.
