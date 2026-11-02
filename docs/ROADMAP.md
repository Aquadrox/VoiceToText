# Roadmap et avancement

Légende : ✅ fait · 🚧 en cours · ⬜ à faire

## v0.0 — Fondations ✅

- ✅ Analyse des fonctions (`FONCTIONS.md`)
- ✅ Architecture et choix techniques (`ARCHITECTURE.md`)
- ✅ Workspace Cargo, crate `vtt-core` : configuration TOML, traits du pipeline, repli si le LLM échoue
- ✅ CI GitHub Actions : tests Linux + Windows

## v0.1 — Dictée qui marche (Windows) ⬜

- ⬜ `vtt-platform` : hook clavier AltGr (avec gestion des combinaisons AZERTY)
- ⬜ `vtt-platform` : capture micro 16 kHz mono
- ⬜ `vtt-asr` : Whisper large-v3-turbo, Vulkan, français
- ⬜ `vtt-platform` : insertion presse-papier + Ctrl+V avec restauration
- ⬜ `apps/desktop` : Tauri, icône de notification, pastille d'enregistrement
- ⬜ Téléchargement du modèle au premier lancement
- ⬜ Installeur Windows construit par la CI

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
