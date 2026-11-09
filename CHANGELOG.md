# Nouveautés

Notes de version affichées par l'application avec chaque mise à jour, puis consultables après l'installation (PC : Réglages → Application ; Android : section Mises à jour).

Règles (vérifiées par la CI, `.github/scripts/changelog.py check`) :

- une section `## X.Y.Z — AAAA-MM-JJ` par version publiée, la plus récente en haut ; la version du Cargo.toml racine doit avoir la sienne ;
- une ligne `- ` par changement, en mots d'utilisateur (texte simple : la mise en forme n'est pas affichée) ;
- `[PC]` ou `[Android]` en tête d'un changement propre à une plateforme ; sans préfixe, il est montré sur les deux.

## 0.24.0 — 2026-11-09

- [Android] Nouvelle apparence aux couleurs du logo : en-tête avec le logo, réglages rangés en cartes, mode sombre soigné.
- [Android] Carte « Mise en route » : chaque étape (micro, modèle, clavier, bulle) est cochée une fois faite, avec le bouton qui ouvre le bon écran d'Android.
- [Android] « Enregistrer les réglages » reste visible en bas de l'écran ; les explications longues sont repliées sous « En savoir plus ».
- [Android] Clavier : vraies icônes à la place des émojis, touches arrondies, et un disque autour du micro qui suit le niveau de la voix.
- [Android] Bulle : icônes dessinées (micro, consigne, attente, terminé).
- [Android] Historique en liste, dictionnaire en pastilles, logo dans les notifications.

## 0.23.0 — 2026-11-09

- Nouveau logo : des barres de voix qui se terminent par un curseur de texte (icône de l'application).
- [PC] Nouvelle apparence : couleurs tirées du logo (encre, papier, vermillon), icônes dans les menus à la place des émojis, accueil et historique plus aérés, mode sombre retouché.
- [PC] Accueil : la barre d'aujourd'hui ressort dans le graphique ; la part de chaque application se lit sur une barre complète.
- [PC] Pastille : vermillon pendant l'écoute et pendant une réunion, une conférence ou une capture ; bleu pendant la transcription.
- [Android] Clavier et bulle aux couleurs du logo : micro vermillon sur le clavier, bulle d'encre au repos et vermillon à l'écoute.

## 0.22.0 — 2026-11-08

- [PC] Chaque mise à jour proposée affiche ses nouveautés (accueil et Réglages → Application) ; après l'installation, l'accueil montre ce qui a changé.
- [PC] Les nouveautés de chaque version restent consultables dans Réglages → Application, sous le bouton de mise à jour.
- [Android] Chaque mise à jour proposée affiche ses nouveautés (bandeau et section Mises à jour) ; après l'installation, l'application montre ce qui a changé.
- [Android] Les nouveautés de chaque version restent consultables dans la section Mises à jour.

## 0.21.0 — 2026-11-08

- [Android] Mises à jour sur clic, comme sur PC : vérification à l'ouverture puis toutes les 6 h, notification, téléchargement vérifié (taille, SHA-256, signature) puis installation par Android.

## 0.20.0 — 2026-11-07

- [PC] Réunions et conférences : les voix de la piste « Eux » sont distinguées (« Intervenant 1 », « Intervenant 2 »…), avec des noms à donner, repris dans le compte rendu. Réglage désactivé par défaut (Réglages → Réunions).
- [PC] « Distinguer les intervenants » après coup, pour un enregistrement fait sans le réglage.

## 0.19.0 — 2026-11-07

- [PC] Réglages découpés en sections, appliqués section par section, sans redémarrer.
- [PC] Zone de test en bas des réglages : ce que le moteur a entendu, le texte inséré, les temps et le coût de chaque dictée.

## 0.18.2 — 2026-11-07

- [Android] La bulle ne s'arrête plus sur une erreur du moteur ou du micro ; si elle s'arrête quand même, l'application le signale avec la raison donnée par Android.

## 0.18.1 — 2026-11-05

- [Android] Parakeet conseillé et proposé par défaut : le plus rapide et le plus fiable aux essais.

## 0.18.0 — 2026-11-05

- [Android] Parakeet et Vosk sur le téléphone, en plus des trois modèles Whisper ; réglages du PC qui les utilisent importés tels quels.

## 0.17.1 — 2026-11-05

- [Android] Bulle de dictée flottante : dicter sans changer de clavier, dans n'importe quelle application.
- [Android] Deux éditions : « clavier » (s'installe partout) et « bulle ».

## 0.16.0 — 2026-11-05

- [Android] Historique des dictées, dictionnaire et snippets modifiables sur le téléphone.
- [Android] Mode traduction (🌍) et Command Mode (maintenir ✨).
- [Android] Mots de passe et navigation privée : rien dans l'historique, rien envoyé au LLM.

## 0.15.0 — 2026-11-04

- [Android] Première version Android : clavier de dictée, transcription Whisper sur le téléphone, nettoyage par LLM.
- [Android] Clé API chiffrée par le téléphone ; import des réglages exportés depuis le PC.

## 0.14.1 — 2026-11-04

- [PC] Réécouter un passage d'une réunion ou d'une conférence (▶ sur chaque réplique).
- [PC] Prompt du compte rendu de conférence modifiable dans la page Prompts.

## 0.14.0 — 2026-11-04

- [PC] Conférences : enregistrement longue durée, ou import d'un fichier audio ou vidéo.
- [PC] Recherche dans toutes les réunions et conférences, et questions au LLM avec les sources citées.

## 0.13.1 — 2026-11-04

- [PC] Capture commentée : fenêtre active ou zone tracée à la souris, clics cerclés sur les images.
- [PC] Option (désactivée par défaut) : description des images par un LLM.

## 0.13.0 — 2026-11-04

- [PC] Capture commentée (Alt+Maj+A) : parler et dessiner sur l'écran, puis coller un texte et des images annotées.

## 0.12.2 — 2026-11-04

- [PC] Détection d'appel : enregistrement proposé quand Teams, Zoom, Discord… utilisent le micro.
- [PC] Écho du son du PC retiré de la piste du micro.
- [PC] Compte rendu dans la langue choisie.

## 0.12.1 — 2026-11-04

- [PC] Boutons radio des réglages des réunions alignés.

## 0.12.0 — 2026-11-04

- [PC] Réunions : enregistrement d'un appel (micro et son du PC), transcription en direct, compte rendu par le LLM.

## 0.11.0 — 2026-11-04

- [PC] Choix du moteur de transcription : Parakeet (PC sans carte graphique) et Vosk (vieux PC), en plus de Whisper.

## 0.10.0 — 2026-11-04

- [PC] Export et import des réglages (jamais les clés API).
- [PC] Liste des clés API enregistrées, à remplacer ou supprimer.
- [PC] Statistiques par application sur l'accueil.

## 0.9.1 — 2026-11-04

- [PC] Suggestions pour le dictionnaire tirées de l'historique.
- [PC] Mode traduction (Alt+Maj+T) : dicter dans une langue, insérer dans une autre.

## 0.9.0 — 2026-11-04

- [PC] Tous les réglages appliqués sans redémarrer (bouton Appliquer).
- [PC] « Vérifier les mises à jour » dans les réglages, installation avec progression.

## 0.8.0 — 2026-11-04

- [PC] Sons de début, de fin et d'annulation de la dictée.
- [PC] Musique et vidéos mises en pause pendant la dictée.
- [PC] Niveau du micro dans la pastille.
- [PC] Commandes vocales : « nouvelle ligne », « nouveau paragraphe », « annule la dernière dictée ».

## 0.7.2 — 2026-11-04

- [PC] « Réessayer » la dernière dictée (Alt+Maj+R) ; collage refusé : texte laissé dans le presse-papier.
- [PC] Mémoire graphique libérée après un temps sans dicter ou pendant un jeu en plein écran.
- [PC] Profil LLM de secours quand le fournisseur principal échoue.

## 0.6.1 — 2026-11-03

- [PC] Réglages LLM appliqués sans redémarrer ; « Tester ces réglages » avec diagnostic des erreurs.
- [PC] Profils LLM enregistrés ; serveurs du réseau local acceptés.
- [PC] Prompts système modifiables (page Prompts), testables sur un exemple.

## 0.5.1 — 2026-11-03

- [PC] Aucun changement visible : première mise à jour automatique.

## 0.5.0 — 2026-11-03

- [PC] Mises à jour signées : vérification régulière, installation sur un clic.
- [PC] Manuel d'utilisation complet dans le README.

## 0.4.2 — 2026-11-03

- [PC] Pages Dictionnaire, Snippets et Styles, appliquées sans recharger le modèle.
- [PC] Réglages complets : raccourcis, micro, modèle Whisper, carte graphique, démarrage avec Windows.

## 0.4.0 — 2026-11-03

- [PC] Fenêtre principale : accueil avec statistiques (mots dictés, temps gagné, coût) et historique avec le coût de chaque dictée.

## 0.3.1 — 2026-11-03

- [PC] Command Mode (AltGr + Ctrl droit) : une consigne dictée transforme le texte sélectionné.

## 0.3.0 — 2026-11-03

- [PC] Mode mains libres : double appui court pour dicter sans tenir la touche.
- [PC] Styles par application : consignes de nettoyage selon le programme actif.
- [PC] « Coller la version brute » (Alt+Maj+V).
- [PC] Fournisseur LLM au choix (DeepSeek, GLM, OpenAI, Mistral, OpenRouter, Ollama…) et coût suivi.

## 0.2.1 — 2026-11-03

- [PC] Snippets : dicter un déclencheur insère un texte fixe.
- [PC] Historique des 1000 dernières dictées.

## 0.2.0 — 2026-11-03

- [PC] Nettoyage du texte par un LLM : ponctuation, hésitations retirées ; consignes dictées jamais exécutées.
- [PC] Clé API rangée dans le Gestionnaire d'identifiants Windows.
- [PC] Dictionnaire personnel pour les noms et termes à reconnaître.

## 0.1.0 — 2026-11-03

- [PC] Première version : dictée en maintenant AltGr, transcription Whisper sur la carte graphique, texte collé là où vous écrivez.
