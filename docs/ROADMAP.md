# Roadmap et avancement

Légende : ✅ fait · 🚧 en cours · ⬜ à faire

## v0.0 — Fondations ✅

- ✅ Analyse des fonctions (`FONCTIONS.md`)
- ✅ Architecture et choix techniques (`ARCHITECTURE.md`)
- ✅ Workspace Cargo, crate `vtt-core` : configuration TOML, traits du pipeline, repli si le LLM échoue
- ✅ CI GitHub Actions : tests Linux + Windows

## v0.1 — Dictée qui marche (Windows) 🚧 code livré, validation sur PC en cours

- ✅ `vtt-core` : push-to-talk AltGr (combinaisons AZERTY, faux Ctrl d'AltGr), conversion audio, silence, hallucinations
- ✅ `vtt-platform` : hook clavier, capture micro 16 kHz mono, insertion presse-papier + Ctrl+V avec restauration
- ✅ `vtt-asr` : Whisper large-v3-turbo q8_0, Vulkan, français — validé sur le serveur (CPU) avec une voix de synthèse
- ✅ `apps/desktop` : Tauri, icône de notification, pastille d'enregistrement, journaux
- ✅ Téléchargement du modèle au premier lancement (SHA-256 vérifié)
- ✅ Installeur Windows construit par la CI
- ⬜ Validation sur le PC Windows (latence GPU, AltGr, insertion dans les applis courantes)

## v0.2 — Texte propre ⬜

- ⬜ `vtt-llm` : client DeepSeek, prompt de nettoyage français
- ⬜ Clé API dans le gestionnaire d'identifiants Windows
- ⬜ Dictionnaire perso, snippets
- ⬜ Historique
- ⬜ Écran de réglages (dont choix du raccourci)
- ⬜ Mesure de latence bout en bout

## v0.3 — Confort ⬜

- ⬜ Mode mains libres (double appui)
- ⬜ Styles par application
- ⬜ Command Mode
- ⬜ Raccourci « coller la version brute »

## Plus tard

- Anglais, Linux, Android, nettoyage local.
