# Roadmap et avancement

Légende : ✅ fait · 🚧 en cours · ⬜ à faire

## v0.0 — Fondations ✅

- ✅ Analyse des fonctions (`FONCTIONS.md`)
- ✅ Architecture et choix techniques (`ARCHITECTURE.md`)
- ✅ Workspace Cargo, crate `vtt-core` : configuration TOML, traits du pipeline, repli si le LLM échoue
- ✅ CI GitHub Actions : tests Linux + Windows

## v0.1 — Dictée qui marche (Windows) ✅

- ✅ `vtt-core` : push-to-talk AltGr (combinaisons AZERTY, faux Ctrl d'AltGr), conversion audio, silence, hallucinations
- ✅ `vtt-platform` : hook clavier, capture micro 16 kHz mono, insertion presse-papier + Ctrl+V avec restauration
- ✅ `vtt-asr` : Whisper large-v3-turbo q8_0, Vulkan, français
- ✅ `apps/desktop` : Tauri, icône de notification, pastille d'enregistrement, journaux
- ✅ Téléchargement du modèle au premier lancement (SHA-256 vérifié)
- ✅ Installeur Windows construit par la CI
- ✅ Validé sur le PC (RTX 4070 Ti) : ASR 420–530 ms pour 1,4–2,5 s de dictée ; préchauffage 10 s au démarrage

## v0.2 — Texte propre 🚧

- ✅ `vtt-llm` : client OpenAI-compatible, DeepSeek `deepseek-flash` sans mode réflexion, prompt de nettoyage français protégé contre l'exécution des consignes dictées
- ✅ Clé API dans le Gestionnaire d'identifiants Windows, saisie et test depuis « Réglages… »
- ✅ Dictionnaire perso (`dictionary` dans config.toml) → indice pour Whisper et le LLM
- ✅ Validé sur le PC : nettoyage 720–1070 ms par dictée, préchauffage 0,2 s (modèle en cache)
- ✅ Journal daté, purgé au-delà de 3 Mo (garde les 100 dernières lignes)
- ⬜ Snippets
- ⬜ Historique

## v0.3 — Confort ⬜

- ⬜ Mode mains libres (double appui)
- ⬜ Styles par application
- ⬜ Command Mode
- ⬜ Raccourci « coller la version brute »

## v0.4 — Interface principale ⬜

Application façon Wispr Flow, remplace la fenêtre de réglages provisoire :

- ⬜ Tableau de bord : statistiques (mots dictés, vitesse, temps gagné, nombre de dictées)
- ⬜ Historique consultable (brut / nettoyé, copier, recoller)
- ⬜ Réglages complets : raccourci, micro, modèle, LLM, dictionnaire, snippets, démarrage avec Windows

## Plus tard

- Anglais, Linux, Android, nettoyage local.
