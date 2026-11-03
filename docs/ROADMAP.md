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

## v0.2 — Texte propre ✅

- ✅ `vtt-llm` : client OpenAI-compatible, DeepSeek `deepseek-flash` sans mode réflexion, prompt de nettoyage français protégé contre l'exécution des consignes dictées
- ✅ Clé API dans le Gestionnaire d'identifiants Windows, saisie et test depuis « Réglages… »
- ✅ Dictionnaire perso (`dictionary` dans config.toml) → indice pour Whisper et le LLM
- ✅ Validé sur le PC : nettoyage 720–1070 ms par dictée, préchauffage 0,2 s (modèle en cache)
- ✅ Journal daté, purgé au-delà de 3 Mo (garde les 100 dernières lignes)
- ✅ Snippets : dicter seulement le déclencheur insère le texte fixe, sans LLM (config.toml en attendant l'interface v0.4)
- ✅ Historique : 1000 dernières dictées sur le PC, fenêtre « Historique… » (recherche, brut/nettoyé, copier, tout effacer)

## v0.3 — Confort ✅

- ✅ Mode mains libres : double appui court → dictée sans tenir la touche ; nouvel appui pour terminer, Échap pour annuler ; 10 min max ; désactivable (`hotkey.hands_free`)
- ✅ Styles par application (`[[styles]]` dans config.toml) : exécutable ou morceau du titre de la fenêtre active → consignes ajoutées au nettoyage, ou texte brut (`cleanup = false`) ; application affichée dans l'historique
- ✅ Command Mode : AltGr + Ctrl droit (`hotkey.command_key`) → consigne dictée appliquée au texte sélectionné (Ctrl+C simulé, presse-papier restauré), ou rédaction à l'emplacement du curseur sans sélection ; en cas d'échec la sélection reste intacte
- ✅ « Coller la version brute » : raccourci global Alt+Maj+V (configurable, `hotkey.paste_raw`) qui colle la transcription Whisper de la dernière dictée ; « Copier la dernière dictée brute » dans le menu ; gardée en mémoire seulement
- ✅ Suivi du coût LLM : jetons lus dans la réponse (`usage`, cache DeepSeek et OpenAI compris), coût estimé avec un tarif réglable (`[llm.pricing]`), totaux jour / mois / total dans « Réglages… » ; fichier `usage.json` sans aucun texte dicté
- ✅ Fournisseur LLM au choix dans « Réglages… » : préréglages DeepSeek, GLM (Z.ai), OpenAI, Mistral, OpenRouter, Ollama local, ou adresse libre ; modèle, délai, tarif, paramètres propres au fournisseur (`extra_body`) ; une clé par fournisseur ; modèle local sans clé

## v0.4 — Interface principale ⬜

Application façon Wispr Flow, remplace la fenêtre de réglages provisoire :

- ⬜ Tableau de bord : statistiques (mots dictés, vitesse, temps gagné, nombre de dictées)
- ⬜ Historique intégré à l'interface (recoller, statistiques)
- ⬜ Réglages complets : raccourci, micro, modèle, LLM, dictionnaire, snippets, démarrage avec Windows

## v0.5 — Distribution et mises à jour ⬜

- ✅ Hébergement des installeurs : dépôt public depuis le 03/11/2026, les Releases GitHub sont téléchargeables sans compte (plus besoin du VPS)
- ⬜ Vérification au démarrage (puis toutes les quelques heures) : avertissement « nouvelle version disponible » dans la zone de notification
- ⬜ Mise à jour automatique en un clic via le module de mise à jour de Tauri : paquets **signés** (clé privée hors du dépôt, dans les secrets de la CI), signature vérifiée avant installation
- ⬜ Publication automatisée par la CI à chaque tag (installeur + manifeste `latest.json`)
- ⬜ Réglage : mise à jour automatique ou simple avertissement

## Outillage 🚧

Accélérer la CI sans baisser la garde :

- ✅ Sur les PR, ne vérifier que ce qui est touché (`.github/scripts/changes.sh`) : PR de doc → quelques secondes ; crate modifié → clippy et tests de ce crate et de l'application ; `vtt-core`, dépendances ou CI modifiés → tout
- ✅ Les vérifications obligatoires `linux` et `windows` répondent toujours, même pour la doc (étapes sautées, job réussi). Pas de `[skip ci]` sur une branche de PR
- ✅ Au moindre doute (fichier non classé, historique introuvable), tout est vérifié
- ✅ Suite complète + installeur toujours sur `main` et sur les tags : rien n'est publié sans tout repasser
- ✅ Cache de compilation corrigé : rust-cache recevait un chemin absolu (`C:\t`) qu'il collait derrière le dépôt, et ne sauvegardait donc aucun fichier compilé ; chemin relatif (`D:\t`), cache alimenté par `main` et réutilisé par les PR
- ✅ CLI Tauri précompilée via npm au lieu de `cargo install` (10 min de compilation à chaque cache vide)
- ✅ Un nouveau push sur une PR annule la vérification précédente
- ⬜ (priorité basse) Actions encore sur Node.js 20, forcées sur Node.js 24 par GitHub (avertissement en CI, ex. `actions/upload-artifact@v4`) : passer aux versions majeures suivantes des actions concernées

## Plus tard

- Anglais, Linux, Android, nettoyage local.
