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
- ✅ Snippets : dicter seulement le déclencheur insère le texte fixe, sans LLM (config.toml en attendant l'interface v0.4)
- ⬜ Historique

## v0.3 — Confort ⬜

- ⬜ Mode mains libres (double appui)
- ⬜ Styles par application
- ⬜ Command Mode
- ⬜ Raccourci « coller la version brute »
- ⬜ Suivi du coût LLM : jetons consommés lus dans la réponse de l'API (`usage`, y compris le cache DeepSeek), coût calculé avec un tarif par modèle réglable (prix différents selon les fournisseurs), total par jour / mois, affiché dans les réglages puis le tableau de bord v0.4
- ⬜ Fournisseur LLM au choix dans les réglages : URL, modèle (GLM, OpenAI, Mistral…), délai maximal, clé par fournisseur, bouton Tester. Paramètres propres à chaque fournisseur (ex. désactivation de la réflexion) gérés par préréglages. *Déjà possible à la main via `[llm]` dans config.toml.*

## v0.4 — Interface principale ⬜

Application façon Wispr Flow, remplace la fenêtre de réglages provisoire :

- ⬜ Tableau de bord : statistiques (mots dictés, vitesse, temps gagné, nombre de dictées)
- ⬜ Historique consultable (brut / nettoyé, copier, recoller)
- ⬜ Réglages complets : raccourci, micro, modèle, LLM, dictionnaire, snippets, démarrage avec Windows

## v0.5 — Distribution et mises à jour ⬜

- ⬜ Hébergement des installeurs accessible sans compte GitHub : le dépôt est privé, ses Releases ne sont pas téléchargeables par l'application. Options : VPS IONOS (fichiers statiques derrière HTTPS), dépôt public dédié aux seules releases, ou stockage en ligne
- ⬜ Vérification au démarrage (puis toutes les quelques heures) : avertissement « nouvelle version disponible » dans la zone de notification
- ⬜ Mise à jour automatique en un clic via le module de mise à jour de Tauri : paquets **signés** (clé privée hors du dépôt, dans les secrets de la CI), signature vérifiée avant installation
- ⬜ Publication automatisée par la CI à chaque tag (installeur + manifeste `latest.json`)
- ⬜ Réglage : mise à jour automatique ou simple avertissement

## Outillage ✅

Accélérer la CI sans baisser la garde :

- ✅ Sur les PR, ne vérifier que ce qui est touché (`.github/scripts/changes.sh`) : PR de doc → quelques secondes ; crate modifié → clippy et tests de ce crate et de l'application ; `vtt-core`, dépendances ou CI modifiés → tout
- ✅ Les vérifications obligatoires `linux` et `windows` répondent toujours, même pour la doc (étapes sautées, job réussi). Pas de `[skip ci]` sur une branche de PR
- ✅ Au moindre doute (fichier non classé, historique introuvable), tout est vérifié
- ✅ Suite complète + installeur toujours sur `main` et sur les tags : rien n'est publié sans tout repasser
- ✅ Cache de compilation corrigé : l'installeur se recompilait entièrement (~10 min) faute de cache release ; le cache est désormais alimenté par `main` et réutilisé par les PR
- ✅ Un nouveau push sur une PR annule la vérification précédente

## Plus tard

- Anglais, Linux, Android, nettoyage local.
