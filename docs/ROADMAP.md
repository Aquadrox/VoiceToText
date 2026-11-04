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

## v0.4 — Interface principale ✅

Application façon Wispr Flow, remplace la fenêtre de réglages provisoire :

- ✅ Fenêtre principale (menu latéral), ouverte seulement depuis l'icône : l'application reste discrète
- ✅ Accueil : mots dictés, temps gagné (frappe à 40 mots/min), vitesse de parole, coût du mois, mots par jour sur 30 jours ; compteurs par jour sans texte, conservés si l'historique est effacé
- ✅ Historique intégré, avec le coût de chaque dictée (jetons au survol)
- ✅ Pages Dictionnaire, Snippets et Styles (modèles Messagerie / E-mail / Code, ordre de priorité) : enregistrées dans config.toml et appliquées à chaud, sans recharger le modèle
- ✅ Réglages complets : raccourcis (dictée, Command Mode, version brute, appui minimal, mains libres), micro, modèle Whisper (5 modèles vérifiés par SHA-256, téléchargés au redémarrage), carte graphique, démarrage avec Windows (clé Run de l'utilisateur)

## v0.5 — Distribution et mises à jour ✅

- ✅ Hébergement des installeurs : dépôt public depuis le 03/11/2026, les Releases GitHub sont téléchargeables sans compte (plus besoin du VPS)
- ✅ Vérification 1 min après le démarrage puis toutes les 6 h (`updates.check`) : notification Windows (une fois par version), entrée « Installer la version… » dans le menu de l'icône, bandeau sur l'accueil
- ✅ Installation sur clic uniquement, via le module de mise à jour de Tauri : paquets **signés** (clé privée générée par le propriétaire, uniquement dans les secrets GitHub ; clé publique dans tauri.conf.json), signature et version signée vérifiées avant installation (protection contre le retour forcé à une ancienne version)
- ✅ CI : installeur signé et manifeste `latest.json` publiés à chaque tag ; les PR ne touchent jamais aux secrets
- ✅ Manuel d'utilisation complet dans le README
- ✅ Validé sur le PC : mise à jour automatique (passage 0.5.0 → 0.5.1)

## v0.6 — Personnalisation du LLM ✅

- ✅ Réglages LLM appliqués à chaud, sans redémarrage
- ✅ « Tester ces réglages » avant d'enregistrer : adresse appelée, clé envoyée ou non, réponse ou erreur, jetons et coût, modèles du serveur (`/models`, un clic pour choisir), pistes de correction (`/v1` manquant, port fermé, modèle inexact, clé refusée…)
- ✅ Adresse normalisée (`/` final et `/chat/completions` recopié retirés)
- ✅ Serveurs du réseau local (adresses privées, `.local`) en HTTP ; la clé n'est jamais envoyée en clair hors de ce PC
- ✅ Profils LLM enregistrés, pour passer de l'un à l'autre
- ✅ Clé API rattachée à l'adresse saisie dans le formulaire
- ✅ Prompts système éditables (page Prompts) : nettoyage et Command Mode, prompt intégré en référence, copies modifiables, test sur exemple, prompt actif par profil LLM, appliqués à chaud

## v0.7 — Fiabilité ✅

- ✅ Rien n'est perdu : l'audio de la dernière dictée reste en mémoire ; « Réessayer » par raccourci (Alt+Maj+R, collé au curseur) ou depuis le menu de l'icône (copié) ; collage refusé → texte laissé dans le presse-papier
- ✅ Libérer la mémoire graphique : Whisper déchargé après 5 à 60 min sans dicter (`asr.unload_after_min`) ou pendant un jeu / une vidéo en plein écran (`asr.unload_in_fullscreen`, après une minute sans dicter), rechargé dès le début de la dictée suivante, pendant que l'utilisateur parle
- ✅ Profil LLM de secours (`llm.fallback_profile`) : si le fournisseur principal échoue ou répond vide, un second profil est essayé (nettoyage et Command Mode) avant le repli sur le texte brut ; coût au tarif du secours, dictée marquée dans l'historique

## v0.8 — Confort de dictée ✅

- ✅ Sons de début, de fin et d'annulation (générés par l'application, `dictation.sounds`)
- ✅ Musique et vidéos en pause pendant la dictée puis relancées, via les contrôles multimédias de Windows ; seul ce qui jouait est relancé (`dictation.pause_media`)
- ✅ Niveau du micro dans la pastille (barres à la place du point pendant l'enregistrement)
- ✅ Commandes vocales (`dictation.voice_commands`) : « nouvelle ligne », « à la ligne », « point à la ligne », « nouveau paragraphe » (reconnues seulement détachées de la phrase), « annule la dernière dictée » (Ctrl+Z)

## v0.9 — Apprentissage ✅

- ✅ Tous les réglages appliqués sans redémarrer (bouton **Appliquer**) : raccourcis, micro, modèle Whisper (téléchargé puis chargé à chaud), mémoire graphique, sons, pause de la musique, commandes vocales, mises à jour
- ✅ Bouton **Vérifier les mises à jour** dans les réglages : version disponible, téléchargement et installation en un clic, avec progression
- ✅ Suggestions pour le dictionnaire tirées de l'historique (mots corrigés par le LLM, noms propres et sigles récurrents), ajout ou refus en un clic
- ✅ Mode traduction : Alt+Maj+T ou menu de l'icône, langue d'arrivée au choix, langue affichée dans la pastille

## v0.10 — Pratique ✅

- ✅ Export / import des réglages (configuration, prompts, profils, dictionnaire, snippets, styles ; jamais les clés API, ni un paramètre avancé qui ressemble à un secret). Import vérifié comme une saisie, anciens réglages gardés dans `config.avant-import.toml`, appliqué sans redémarrer
- ✅ Liste des clés API enregistrées (une par fournisseur) pour les remplacer ou les supprimer ; jamais affichées en clair, ni en entier ni en partie (seulement le fournisseur et « clé enregistrée »). Seuls les noms des identifiants sont lus pour la liste
- ✅ Statistiques par application sur l'accueil (mots, part, dictées, coût ; ce mois ou depuis le début ; nom du programme seulement)

## Outillage ✅

Accélérer la CI sans baisser la garde :

- ✅ Sur les PR, ne vérifier que ce qui est touché (`.github/scripts/changes.sh`) : PR de doc → quelques secondes ; crate modifié → clippy et tests de ce crate et de l'application ; `vtt-core`, dépendances ou CI modifiés → tout
- ✅ Les vérifications obligatoires `linux` et `windows` répondent toujours, même pour la doc (étapes sautées, job réussi). Pas de `[skip ci]` sur une branche de PR
- ✅ Au moindre doute (fichier non classé, historique introuvable), tout est vérifié
- ✅ Suite complète + installeur toujours sur `main` et sur les tags : rien n'est publié sans tout repasser
- ✅ Cache de compilation corrigé : rust-cache recevait un chemin absolu (`C:\t`) qu'il collait derrière le dépôt, et ne sauvegardait donc aucun fichier compilé ; chemin relatif (`D:\t`), cache alimenté par `main` et réutilisé par les PR
- ✅ CLI Tauri précompilée via npm au lieu de `cargo install` (10 min de compilation à chaque cache vide)
- ✅ Un nouveau push sur une PR annule la vérification précédente
- ✅ Actions GitHub sur Node.js 24 : checkout v7, upload-artifact v7, download-artifact v8 (contrôle d'empreinte des artefacts désormais bloquant)

## Plus tard

- Installeur signé (certificat de signature de code) : plus d'avertissement SmartScreen. Reporté tant que l'application n'a pas d'utilisateurs : les certificats sont payants, et SignPath Foundation (gratuit) exige un logiciel déjà diffusé et utilisé. Avertissement non bloquant en attendant (« Informations complémentaires » → « Exécuter quand même »).
- Anglais, Linux, Android, nettoyage local.
