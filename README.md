# VoiceToText

Dictée vocale universelle, inspirée de Wispr Flow : maintenir **AltGr**, parler, relâcher. Le texte transcrit et nettoyé apparaît dans l'application active.

- Transcription locale (Whisper sur GPU), français.
- Nettoyage du texte par LLM (DeepSeek, désactivable).
- Windows d'abord ; Linux et Android ensuite.

## Documentation

- [Fonctions](docs/FONCTIONS.md)
- [Architecture et choix techniques](docs/ARCHITECTURE.md)
- [Roadmap et avancement](docs/ROADMAP.md)

## Développement

```sh
cargo test --workspace
```

Les binaires Windows sont produits par la CI GitHub Actions.
