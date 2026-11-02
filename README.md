# VoiceToText

Dictée vocale universelle, inspirée de Wispr Flow : maintenir **AltGr**, parler, relâcher. Le texte transcrit et nettoyé apparaît dans l'application active.

- Transcription locale (Whisper sur GPU), français.
- Nettoyage du texte par LLM (DeepSeek, désactivable).
- Windows d'abord ; Linux et Android ensuite.

## Installation (Windows)

1. Télécharger l'installeur : onglet **Actions** du dépôt → dernier run réussi → artefact `VoiceToText-installeur-windows` (ou page **Releases**).
2. Lancer `VoiceToText_x.y.z_x64-setup.exe` (installation pour l'utilisateur courant, sans droits admin).
3. Au premier lancement, le modèle Whisper (874 Mo) se télécharge : progression affichée dans la pastille en bas de l'écran.

## Utilisation

- **Maintenir AltGr**, parler, **relâcher** : le texte est collé dans l'application active.
- AltGr + une autre touche (`@`, `#`, `€`…) fonctionne normalement : la dictée est annulée.
- Icône de notification : état, configuration, journaux, redémarrage.

Configuration : `%APPDATA%\VoiceToText\config.toml` (redémarrer après modification).

```toml
[hotkey]
key = "AltGr"        # ou RightCtrl, CapsLock, ScrollLock, Pause, Insert, Menu, F1…F24
min_hold_ms = 250

[asr]
model = "ggml-large-v3-turbo-q8_0.bin"
language = "fr"
use_gpu = true
```

Journaux : `%LOCALAPPDATA%\VoiceToText\logs\voicetotext.log`.

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
