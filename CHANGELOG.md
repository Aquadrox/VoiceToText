# Nouveautés

Notes de version affichées par l'application avec chaque mise à jour, puis consultables après l'installation (PC : Réglages → Application ; Android : section Mises à jour).

Règles (vérifiées par la CI, `.github/scripts/changelog.py check`) :

- une section `## X.Y.Z — AAAA-MM-JJ` par version publiée, la plus récente en haut ; la version du Cargo.toml racine doit avoir la sienne ;
- une ligne `- ` par changement, en mots d'utilisateur (texte simple : la mise en forme n'est pas affichée) ;
- `[PC]` ou `[Android]` en tête d'un changement propre à une plateforme ; sans préfixe, il est montré sur les deux.

## 0.31.0 — 2026-11-14

- Réunions : un intervenant qui se présente est nommé tout seul. « Bonjour, je m'appelle Marie », « Moi c'est Paul », « Karim à l'appareil » : « Intervenant 2 » devient Marie dans la transcription, la copie, l'export et le compte rendu. Il faut avoir activé « Distinguer les intervenants » ; les noms apparaissent à la fin de l'enregistrement, avec les intervenants.
- Réunions : dans la liste des intervenants, la phrase où chacun s'est présenté est affichée sous son nom, pour vérifier. Corrigez ou effacez le nom s'il est faux : votre choix passe toujours devant. Tout se fait sur l'appareil, sans LLM.
- Réunions déjà enregistrées : pour chercher les présentations, changez le nombre d'intervenants puis revenez à « Selon les voix ».

## 0.30.0 — 2026-11-14

- [Android] Réglages repensés : l'accueil n'est plus une longue page. Les réglages sont rangés en catégories (Dictée, Nettoyage du texte, Dictionnaire et snippets, Bulle de dictée, Historique, Mises à jour, Aide), les plus utilisées en premier, chacune avec son état actuel en un coup d'œil (modèle prêt, fournisseur, clé manquante, nouvelle version…).
- [Android] Recherche dans les réglages : tapez « clé », « traduction » ou « taille » ; le résultat ouvre la bonne page et montre le réglage.
- [Android] En haut de l'accueil, seulement ce qui demande votre attention : nouvelle version, bulle arrêtée par le téléphone, et la mise en route tant qu'il manque quelque chose pour dicter.
- [Android] Les interrupteurs et les choix s'enregistrent dès qu'on les touche, avec une confirmation qui dit ce que cela change. Seul le fournisseur du LLM (adresse, modèle et clé, à tester ensemble) garde un bouton « Enregistrer » ; en quittant la page sans l'avoir fait, l'application demande quoi faire.
- [Android] Libellés plus courts, avec une explication sous chaque réglage. Nettoyage du texte : l'adresse et le modèle passent dans « Adresse et modèle (avancé) », ouvert d'office pour un serveur à vous.

## 0.29.0 — 2026-11-14

- [Android] Clés API par fournisseur, comme sur PC : une clé par fournisseur (DeepSeek, GLM, OpenAI, Mistral, OpenRouter…). Changer de fournisseur ne fait plus perdre la clé du précédent, et la clé d'un fournisseur n'est jamais envoyée à un autre.
- [Android] Nouvelle carte « Clés API » : la liste des clés enregistrées, avec le fournisseur qui les utilise ; « Remplacer » ou « Supprimer » (après confirmation). Les clés ne sont jamais affichées, ni en entier ni en partie.
- [Android] Nettoyage du texte : menu « Fournisseur » qui remplit l'adresse et le modèle, avec une aide pour chaque fournisseur ; il suit aussi l'adresse saisie à la main. Sous le champ de la clé, on voit tout de suite si une clé est déjà enregistrée pour cette adresse.
- [Android] « Tester ces réglages » (carte Nettoyage du texte) : essaie l'adresse, le modèle et la clé saisis avant de les enregistrer, sur une phrase d'exemple ; affiche la réponse, le temps, les jetons et le coût, ou l'erreur avec des pistes pour la corriger (clé refusée, adresse ou modèle inexact…). Les modèles proposés par le serveur s'affichent : un appui pour en choisir un.
- [Android] « Tester » sur chaque clé de la carte Clés API : vérifie qu'une clé enregistrée est toujours acceptée par son fournisseur.
- [Android] En changeant de fournisseur, les réglages propres à l'ancien (délai, tarif, paramètre « réflexion » de DeepSeek et GLM) ne sont plus envoyés au nouveau.

## 0.28.0 — 2026-11-14

- [Android] Réunions : enregistrez une réunion, un cours ou une conférence avec le téléphone posé sur la table, puis obtenez la transcription et un compte rendu par le LLM choisi. Carte « Réunions » sur l'accueil, ou appui long sur l'icône → « Enregistrer une réunion ».
- [Android] Réunions : importez un enregistrement existant (mémo vocal, note vocale, enregistrement d'appel du téléphone, cours, vidéo…), avec le bouton « Importer » ou depuis une autre application : Partager → VoiceToText.
- [Android] Réunions : l'enregistrement continue écran éteint ; grand chrono lisible de loin, « Marquer un moment » pour retrouver un passage important, arrêt depuis l'écran ou la notification.
- [Android] Réunions : la transcription se fait à la fin, sur le téléphone, avec le modèle de la dictée ; une notification prévient quand le compte rendu est prêt. Vous pouvez continuer à dicter pendant ce temps.
- [Android] Réunions : compte rendu et transcription dans deux onglets, à partager ou copier ; réécoute d'une phrase quand l'audio est gardé ; renommer, changer de type de compte rendu (réunion ou cours), refaire le compte rendu, supprimer.
- [Android] Réunions : « Distinguer les intervenants » sépare les voix (« Intervenant 1, 2… ») à la fin, sur le téléphone ; donnez-leur un nom (avec leur temps de parole, leur première phrase et ▶ pour écouter leur voix), imposez leur nombre si deux voix sont confondues. Aussi après coup, depuis le menu ⋯, quand l'audio est gardé. Deux petits modèles (33 Mo) se téléchargent la première fois.
- [Android] Réunions : l'audio est gardé sur le téléphone par défaut (stockage privé, chiffré par Android) ; au choix, effacé après la transcription ou gardé en mémoire seulement.

## 0.27.1 — 2026-11-13

- [PC] Réunions : fini les intervenants en trop. Une même voix n'est plus comptée plusieurs fois (quelques mots, un rire, un changement de ton suffisaient à créer un « Intervenant » de plus, d'autant plus souvent que la réunion était longue).
- [PC] Réunions : « Nombre d'intervenants » imposé plus juste. Les voix principales sont gardées ; avant, quelques secondes atypiques pouvaient prendre la place d'une vraie voix.
- [PC] Réunions déjà enregistrées : pour refaire le calcul, dans le bloc Intervenants, choisissez un nombre puis revenez à « Selon les voix » (ou gardez le bon nombre).

## 0.27.0 — 2026-11-11

- [PC] Le menu principal se replie en une colonne d'icônes (nom de la page au survol) : automatiquement dans les Réglages, qui ont leur propre menu, pour laisser plus de place aux réglages. Le bouton « Réduire le menu » / « Déplier le menu », en bas du menu, change ce choix ; il est retenu d'une fois sur l'autre.

## 0.26.1 — 2026-11-10

- [Android] Bulle arrêtée par le téléphone (elle disparaît et ne revient pas, même après être passé par le clavier VoiceToText) : le clavier et l'application le signalent et ouvrent directement son réglage. Il suffit de la désactiver puis de la réactiver, sans forcer l'arrêt de l'application.
- [Android] Bulle : le modèle de transcription est libéré de la mémoire une minute après l'extinction de l'écran, puis rechargé dès que la bulle réapparaît, pour que le téléphone ait moins de raisons d'arrêter l'application.
- [Android] Bulle : la cause de son dernier arrêt, donnée par Android, reste affichée dans la carte « Bulle de dictée », même après l'avoir relancée.
- [Android] Bulle : conseils complétés pour Xiaomi, Samsung… (verrouiller VoiceToText dans les applications récentes) ; sur Xiaomi, un bouton de la carte « Bulle de dictée » ouvre le réglage « Démarrage automatique », absent des informations de l'application.

## 0.26.0 — 2026-11-10

- [Android] Prompts système modifiables : dans la carte « Nettoyage du texte (LLM) », bouton « Prompts système » pour lire le prompt intégré, en créer une copie à votre goût (nettoyage et Command Mode) et choisir celui qui est utilisé ; les prompts importés du PC y apparaissent.
- [Android] Les champs de texte sur plusieurs lignes (zone d'essai, texte d'un snippet) ont retrouvé leur hauteur.

## 0.25.0 — 2026-11-09

- [Android] Bulle : un double appui passe au clavier VoiceToText (Android 11 et plus) ; sa touche clavier ramène ensuite à votre clavier habituel.

## 0.24.1 — 2026-11-09

- [Android] Clavier : Espace et Entrée ne passent plus sous la barre de navigation d'Android (boutons retour, accueil, applications) ; le clavier s'arrête au-dessus.

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
