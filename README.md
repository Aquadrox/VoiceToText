# VoiceToText

Dictée vocale pour Windows, inspirée de Wispr Flow : **maintenir AltGr, parler, relâcher**. Le texte est transcrit sur votre PC, nettoyé (hésitations, ponctuation, auto-corrections) puis collé dans l'application active : traitement de texte, messagerie, navigateur, e-mail…

- **Transcription locale** : Whisper tourne sur votre carte graphique (ou Parakeet / Vosk sur un PC sans carte graphique), l'audio ne quitte jamais le PC.
- **Nettoyage par LLM** (DeepSeek par défaut, ou GLM, OpenAI, Mistral, OpenRouter, un modèle local Ollama…), désactivable.
- **Mains libres**, **Command Mode** (« traduis ça en anglais »), **snippets**, **dictionnaire**, **styles par application**, **historique**, **statistiques**, **suivi du coût**.
- Français pour l'instant ; Windows d'abord, Linux et Android plus tard.

## Sommaire

1. [Installation](#installation)
   - [Sur Android](#sur-android)
2. [Premiers pas](#premiers-pas)
3. [Dicter](#dicter)
4. [Commandes vocales](#commandes-vocales)
5. [Command Mode](#command-mode)
6. [Coller la version brute](#coller-la-version-brute)
7. [Réessayer une dictée](#réessayer-une-dictée)
8. [Mode traduction](#mode-traduction)
9. [Réunions](#réunions)
10. [Capture commentée](#capture-commentée)
11. [La fenêtre principale](#la-fenêtre-principale)
12. [Nettoyage du texte (LLM)](#nettoyage-du-texte-llm)
13. [Mises à jour](#mises-à-jour)
14. [Confidentialité et sécurité](#confidentialité-et-sécurité)
15. [Fichiers et emplacements](#fichiers-et-emplacements)
16. [Configuration avancée (config.toml)](#configuration-avancée-configtoml)
17. [Dépannage](#dépannage)
18. [Développement](#développement)
19. [Licence](#licence)

## Installation

1. Télécharger `VoiceToText_x.y.z_x64-setup.exe` depuis la page [Releases](https://github.com/Aquadrox/VoiceToText/releases/latest).
2. Le lancer : installation pour l'utilisateur courant, **sans droits administrateur**. Windows SmartScreen peut afficher « Windows a protégé votre ordinateur » (l'installeur n'est pas signé par un éditeur reconnu) : **Informations complémentaires → Exécuter quand même**.
3. Au premier lancement, le modèle Whisper (874 Mo) se télécharge ; la progression s'affiche dans la pastille en bas de l'écran. Les lancements suivants prennent environ une seconde.

L'application est **discrète** : une icône dans la zone de notification (à côté de l'horloge) et une pastille qui n'apparaît que pendant la dictée. La fenêtre principale ne s'ouvre jamais d'elle-même.

Pour la lancer à chaque ouverture de session : **Réglages → Démarrage → Lancer VoiceToText à l'ouverture de session Windows**.

### Sur Android

Un **clavier de dictée** ou une **bulle flottante** (au choix, ou les deux) : on parle, et le texte nettoyé s'insère là où l'on écrit (WhatsApp, Gmail, notes…). La voix est transcrite **sur le téléphone** (Whisper) ; seul le texte part au LLM choisi, comme sur PC.

1. Télécharger depuis [Releases](https://github.com/Aquadrox/VoiceToText/releases/latest) sur le téléphone, l'ouvrir et autoriser l'installation (application hors Play Store). Android 10 ou plus récent, processeur 64 bits. Deux éditions, au choix (même application : on passe de l'une à l'autre en gardant ses réglages) :
   - `VoiceToText_x.y.z_android.apk` : **clavier** seul, s'installe normalement ;
   - `VoiceToText_x.y.z_android-bulle.apk` : clavier **et bulle**. Play Protect bloque l'installation de toute application hors Play Store qui contient un service d'accessibilité (« Appli bloquée pour protéger votre appareil ») : le couper le temps de l'installation (Play Store → icône du profil → Play Protect → ⚙ → décocher « Analyser les applis avec Play Protect »), installer, **puis le réactiver aussitôt** ; ou installer depuis un PC avec `adb install`.
2. Ouvrir **VoiceToText** :
   1. **Autoriser le micro** ;
   2. choisir un **modèle** et le **télécharger** (vérifié à l'arrivée) :
      - **Whisper Small** 8 bits (recommandé, 264 Mo) ; **Base** pour un petit téléphone (82 Mo) ; **Large v3 Turbo** pour le plus précis (574 Mo) ;
      - **Parakeet TDT v3** 8 bits (670 Mo) : rapide et ponctué, comme sur PC ; demande environ 1 Go de mémoire vive ;
      - **Vosk Small** français (42 Mo) : ultra léger et instantané, sans ponctuation (le LLM la rajoute) ;
   3. **Activer le clavier VoiceToText**, puis **Choisir le clavier** ; et/ou activer la **bulle** (ci-dessous).
3. **Nettoyage du texte** : adresse de l'API, modèle et clé (rangée chiffrée par le Keystore Android, jamais affichée), ou **Importer un fichier de réglages** exporté depuis le PC (dictionnaire, snippets, styles, fournisseur ; les clés ne sont jamais exportées).

Au clavier :
- **maintenir 🎤** pour dicter, relâcher pour insérer ; **appui court** : mains libres (nouvel appui pour finir) ; le temps de traitement s'affiche ensuite (« ✓ 1,2 s ») ;
- **🌍** : mode traduction (le texte dicté est traduit dans la langue choisie dans l'application), allumé tant qu'on ne le coupe pas ;
- **maintenir ✨** : consigne appliquée au texte **sélectionné** (« rends ce texte plus formel », « traduis en anglais ») ; sans sélection, le texte demandé est rédigé ;
- 🌐 revient au clavier habituel ; ⌫, espace et entrée pour les retouches.

Avec la **bulle** (édition `android-bulle`, sans changer de clavier) : une bulle 🎤 apparaît quand un clavier est ouvert sur un champ de texte.
- **toucher** : dicter ; toucher encore : le texte s'insère au curseur ;
- **maintenir** : consigne appliquée au texte sélectionné (comme ✨) ;
- **glisser** : déplacer (la position est gardée) ;
- réglages : taille, affichage même clavier fermé, traduction, insertion en collant, historique (coupé par défaut).

Activation, une seule fois : Android range la bulle dans les services d'**accessibilité** (seul moyen de savoir quel champ est actif et d'y écrire ; aucune autorisation « par-dessus les autres applis » n'est nécessaire).
1. Android 13 et plus : **Infos de l'application → ⋮ → « Autoriser les paramètres restreints »** (exigé pour une application installée hors Play Store) ;
2. **Accessibilité → Applications installées → « VoiceToText : bulle de dictée »** → activer ;
3. Xiaomi, Samsung… : autoriser le **démarrage automatique** et mettre la batterie sur **« Pas de restriction »**, sinon le téléphone coupe la bulle.

Les boutons de l'application ouvrent directement ces écrans. Le service regarde seulement si le champ actif accepte du texte ; il n'en lit le contenu qu'au moment de la dictée (pour écrire au curseur ou appliquer une consigne à la sélection) et n'enregistre rien. Dans les champs Android ordinaires, le texte est écrit directement ; ailleurs (pages web, éditeurs riches), il est collé, ce qui **remplace le presse-papier** (Android interdit de le relire pour le remettre).

Champ de **mot de passe** : seule la transcription locale est utilisée, sans LLM ni historique. Champ marqué « ne pas apprendre » (navigation privée, application sensible) : rien dans l'historique. La bulle, elle, ne s'affiche jamais sur un mot de passe, mais ne voit pas la navigation privée : un réglage permet de garder ses dictées hors de l'historique.

Les commandes vocales (« nouvelle ligne », « annule la dernière dictée »), les snippets et les styles par application fonctionnent comme sur PC. Dans l'application : **historique** des dictées (sur le téléphone seulement, désactivable ; toucher pour copier), **dictionnaire et snippets** modifiables.

L'audio reste en mémoire le temps de la dictée, jamais sur le stockage du téléphone.

## Premiers pas

1. Clic sur l'icône → **Réglages…**
2. Section **Clé API** : coller une clé DeepSeek ([platform.deepseek.com](https://platform.deepseek.com), compte crédité ; quelques euros suffisent pour des milliers de dictées) → **Enregistrer** → **Tester**.
3. Ouvrir un éditeur de texte, **maintenir AltGr**, dire « euh bonjour, je voulais te dire que la réunion c'est mardi, non pardon mercredi », **relâcher** → « Bonjour, je voulais te dire que la réunion est mercredi. »

Sans clé, tout fonctionne aussi, avec le texte brut de Whisper (voir [Sans LLM](#sans-llm)).

## Dicter

| Geste | Effet |
|---|---|
| **Maintenir AltGr**, parler, **relâcher** | Le texte est collé dans l'application active |
| Appui très court sur AltGr | Ignoré (pas de déclenchement accidentel) |
| AltGr + une autre touche (`@`, `#`, `€`…) | Frappe normale, la dictée est annulée |
| **Deux appuis courts** sur AltGr | **Mains libres** : la dictée continue sans tenir la touche |
| En mains libres : **AltGr** | Termine la dictée et colle le texte |
| En mains libres : **Échap** | Annule la dictée |
| **AltGr + Ctrl droit** | [Command Mode](#command-mode) |
| **Alt+Maj+V** | [Colle la version brute](#coller-la-version-brute) de la dernière dictée |
| **Alt+Maj+R** | [Réessaie la dernière dictée](#réessayer-une-dictée) et colle le résultat |

Toutes ces touches se changent dans **Réglages → Raccourcis**.

La **pastille** en bas de l'écran indique l'état :

| Couleur | État |
|---|---|
| 🔴 Rouge | Écoute (touche maintenue) |
| 🟢 Vert | Mains libres (« Échap annule ») |
| 🟠 Orange | Command Mode : dictez la consigne |
| 🟣 Violet | Transcription et nettoyage en cours |
| ⚪ Gris | Chargement ou téléchargement du modèle |
| 🟡 Jaune | Erreur (message affiché quelques secondes) |

Pendant l'enregistrement, des **barres** dans la pastille suivent le niveau du micro : si elles ne bougent pas, le micro ne capte rien.

Par défaut (désactivable dans **Réglages → Pendant la dictée**) :

- un **son** marque le début (deux notes montantes), la fin (descendantes) et l'annulation (note grave) ;
- la **musique et les vidéos** en cours (Spotify, YouTube dans le navigateur, lecteurs compatibles avec les touches multimédias) sont **mises en pause** pendant la dictée, puis relancées. Seul ce qui jouait est relancé.

Limites : 5 minutes par dictée maintenue, 10 minutes en mains libres (la dictée est alors traitée automatiquement).

## Commandes vocales

| Dire | Effet |
|---|---|
| « nouvelle ligne », « à la ligne », « retour à la ligne » | Retour à la ligne |
| « point à la ligne » | Point puis retour à la ligne |
| « nouveau paragraphe » | Ligne vide |
| « annule la dernière dictée » (seul) | Annule le dernier collage (Ctrl+Z dans l'application) |

Une commande de mise en page n'est reconnue que **détachée du reste de la phrase**, par une pause : « Bonjour Paul, nouvelle ligne, je voulais te dire… » oui ; « on ouvre une nouvelle ligne de bus » reste du texte. « Annule la dernière dictée » doit être la dictée entière ; Ctrl+Z annule la dernière action de l'application, en général le collage.

Désactivables dans **Réglages → Pendant la dictée**.

## Command Mode

Pour transformer un texte existant avec la voix :

1. Sélectionner le texte.
2. Maintenir **AltGr + Ctrl droit** (dans n'importe quel ordre), dicter la consigne, relâcher.
3. La sélection est remplacée par le résultat.

Exemples : « traduis en anglais », « rends ça plus formel », « résume en trois points », « corrige les fautes ».

**Sans sélection**, le texte demandé est rédigé à l'emplacement du curseur : « écris un mail pour décaler la réunion à jeudi ».

- Nécessite le LLM. En cas d'échec (pas de clé, réseau…), **rien n'est collé** : la sélection reste intacte.
- **Désactivé dans les terminaux** (Windows Terminal, cmd, PowerShell…) : une réponse piégée par le texte sélectionné y serait exécutée au collage.
- Dans VS Code, Ctrl+C sans sélection copie toute la ligne : elle est alors traitée comme la sélection.

## Coller la version brute

Si le nettoyage a trop réécrit une dictée : **Ctrl+Z** (annuler le collage) puis **Alt+Maj+V** colle la transcription Whisper d'origine, sans nettoyage. Aussi depuis le menu de l'icône : **Copier la dernière dictée brute**.

## Réessayer une dictée

Rien n'est perdu quand une dictée échoue (transcription en erreur, réseau coupé, collage refusé…) : l'audio de la **dernière dictée** reste en mémoire jusqu'à la suivante.

- **Alt+Maj+R** : la dictée est retraitée (transcription, nettoyage) et collée **là où se trouve le curseur maintenant**. Pratique aussi pour la coller dans une autre fenêtre.
- Menu de l'icône → **Réessayer la dernière dictée (copie)** : même chose, mais le texte est seulement copié (notification « Ctrl+V pour le coller »).
- Une commande (Command Mode) se réessaie avec Alt+Maj+R, après avoir sélectionné le texte à transformer.
- Si le collage est refusé (application lancée en administrateur), le texte reste dans le presse-papier : **Ctrl+V** pour le coller à la main.

L'audio n'est jamais écrit sur le disque. Une dictée réessayée apparaît une seconde fois dans l'historique et son coût LLM est compté ; les statistiques de l'accueil ne la comptent qu'une fois.

## Mode traduction

Dicter en français, insérer dans une autre langue :

1. **Alt+Maj+T** (ou menu de l'icône → **Mode traduction**) : la pastille confirme « Traduction → anglais ».
2. Dicter normalement : le texte est nettoyé puis traduit par le LLM avant d'être collé. Pendant la dictée, la pastille affiche la langue (**→ EN**).
3. **Alt+Maj+T** à nouveau pour revenir au français.

- Langue d'arrivée : **Réglages → Mode traduction** (anglais, espagnol, allemand, italien, portugais, néerlandais).
- Le mode s'applique aux dictées commencées pendant qu'il est actif, même dans les applications dont le style demande le texte brut. Les snippets et le Command Mode ne sont pas traduits.
- Nécessite le LLM ; sans lui, le texte est collé en français et l'historique l'indique (*non nettoyé*).
- L'historique marque ces dictées *traduit : anglais* ; **Alt+Maj+V** colle toujours la transcription française d'origine.

## Réunions

Enregistrer un appel (Teams, Zoom, Meet, Discord…) et en tirer la transcription et un compte rendu, sans rien envoyer en ligne pour la transcription.

1. **Alt+Maj+M**, menu de l'icône → **Démarrer une réunion**, ou page **Réunions**. La pastille rappelle de **prévenir les participants** (obligatoire en France), puis affiche « Réunion 12:34 » tant que l'enregistrement dure.
2. Deux pistes : votre micro (**Moi**) et le son du PC (**Eux** : toutes les sorties audio, casque compris). Elles sont transcrites au fil de l'eau avec le moteur choisi (Whisper, Parakeet, Vosk), par blocs de 6 à 15 s coupés aux pauses, entre deux dictées : vous pouvez continuer à dicter pendant la réunion.
3. **Alt+Maj+N** colle dans l'application active ce qui a été dit depuis la dernière copie (« Moi : … / Eux : … »).
4. **Alt+Maj+M** à nouveau pour arrêter : les derniers blocs sont transcrits, puis le **compte rendu** est rédigé par le LLM choisi (résumé, décisions, actions « qui : quoi », questions ouvertes). Prompt modifiable dans la page **Prompts** (« Compte rendu de réunion »), coût compté dans la consommation.

**Détection d'appel** : quand Teams, Zoom, Discord, Slack, WhatsApp, Signal, Telegram, Skype ou Webex utilise le micro (2 s), ou un navigateur (20 s, pour ne pas confondre avec un message vocal), une petite fenêtre en bas à droite propose **Enregistrer** / **Pas maintenant**, ou **…** → **Toujours enregistrer** / **Ne plus proposer** pour ce service. Sans réponse, elle se referme au bout de 20 s sans rien enregistrer ; le même appel n'est pas reproposé. Seul le nom du programme qui utilise le micro est lu. Choix par service et désactivation dans les réglages des réunions.

**Écho** (appel sur haut-parleurs, le micro réentend les autres) : les passages du micro qui ne font que suivre le son du PC sont écartés avant transcription, puis les répliques « Moi » qui répètent mot pour mot ce que « Eux » venaient de dire sont masquées. Au casque, rien n'est écarté. Désactivable.

Page **Réunions** : liste, transcription horodatée Moi / Eux (en direct pendant l'enregistrement), compte rendu (dans la langue de la réunion ou traduit : anglais, espagnol…), renommer, **Copier** (Markdown), **Exporter (.md)**, **Supprimer**.

**Audio de la réunion**, au choix (page Réunions → Réglages des réunions) :

| Réglage | Sur le disque | Si VoiceToText s'arrête pendant la réunion |
|---|---|---|
| En mémoire seulement | Rien | La fin non transcrite est perdue |
| Fichier temporaire chiffré (**par défaut**) | Chiffré par Windows (DPAPI, lisible seulement par votre session), supprimé une fois la transcription terminée | Transcription reprise au redémarrage |
| Conservé, chiffré | Gardé ; export en WAV et **Transcrire à nouveau** (après un changement de modèle) | Transcription reprise au redémarrage |

### Conférences et fichiers audio / vidéo

Page **Réunions** → **Conférence ou fichier audio / vidéo…** :

- **Enregistrer une conférence** (cours, exposé, webinaire, podcast) : micro (sur place), son du PC (en ligne), ou les deux ; mêmes choix pour l'audio que les réunions ; « Arrêter l'enregistrement » à la fin. La pastille affiche « Conférence 42:10 ».
- **Transcrire un fichier** : MP3, M4A, MP4, MKV, WebM, WAV, FLAC, OGG… Le fichier est lu sur le PC au fil de l'eau (même plusieurs heures, sans tout charger en mémoire), ni copié ni modifié ; la progression s'affiche dans la liste.
- **Ensuite** : transcription et compte rendu, transcription seule, ou compte rendu seul (la transcription est effacée une fois résumée). Le compte rendu d'une conférence suit un autre plan : résumé, plan horodaté, idées clés, citations, questions et réponses ; prompt modifiable dans la page **Prompts** (« Compte rendu de conférence ou de fichier »).

### Rechercher et poser des questions

- **Rechercher dans toutes les transcriptions** (en haut de la page Réunions) : sans tenir compte des majuscules ni des accents (« reunion » trouve « Réunion ») ; chaque résultat indique l'enregistrement et l'heure, et un clic ouvre la transcription au bon passage, surligné.
- **Réécouter un passage** : quand l'audio est disponible (réunion ou conférence avec l'audio « conservé », ou fichier importé toujours à sa place), l'heure de chaque réplique devient **▶ 12:34** : un clic joue 30 s à partir de ce moment. L'audio conservé est déchiffré à la volée, rien n'est écrit sur le disque.
- **Poser une question** au LLM, sur l'enregistrement affiché ou sur **tous** (« qu'a-t-on décidé pour le budget ? ») : les passages qui contiennent les mots de la question sont envoyés au LLM, qui répond en citant ses sources (titre, heure). Coût compté comme le reste.

Autres réglages : capter ou non le son du PC, écarter l'écho, détection d'appel et choix par service, compte rendu automatique ou à la demande et sa langue, durée de conservation des réunions (7, 30, 90, 365 jours ou toujours).

## Capture commentée

Montrer plutôt qu'écrire : expliquer un bug, une idée ou une consigne en parlant et en dessinant sur l'écran, puis coller le résultat (Slack, Jira, mail, ChatGPT, Claude…).

1. **Alt+Maj+A** (ou menu de l'icône → **Capture commentée**) : la pastille affiche « Capture 00:05 ». Parlez en montrant l'écran.
2. **Tenez Alt+Maj+A** et bougez la souris : vous dessinez à l'écran (sans cliquer). Le trait s'efface au bout d'une seconde, mais il est gardé sur les images.
3. **Alt+Maj+A** à nouveau (appui court) pour arrêter. VoiceToText :
   - transcrit votre narration mot par mot ;
   - choisit les **moments clés** : chaque trait dessiné, chaque clic (cercle rouge), et les mots qui désignent l'écran (« ici », « là », « ça », « regarde »…), au plus 6 ;
   - prend l'écran de ces moments, y dessine vos traits, et les réunit en une **planche numérotée** ;
   - fait rédiger le texte par le LLM selon le modèle choisi (**Explication**, **Rapport de bug**, **Consigne pour une IA**, ou **Narration seule** sans LLM), avec des repères `[image 1]`… ;
   - **colle la planche puis le texte** dans l'application active.

Menu de l'icône → **Coller la dernière capture commentée** pour la coller ailleurs. Réglages → **Capture commentée** :
- **Capturer** : tout l'écran principal, la **fenêtre active** au démarrage, ou une **zone** tracée à la souris au démarrage (Échap : tout l'écran) ;
- clics comme moments clés (oui par défaut) ;
- modèle de texte ; conservation sur le disque (non par défaut : seule la dernière reste en mémoire ; 1, 7 ou 30 jours) ;
- **Faire décrire les images par le LLM** (non par défaut) : les images annotées sont envoyées à un modèle qui lit les images (fournisseur actif ou un profil, par exemple un modèle OpenAI ou Mistral avec vision), pour un texte plus précis (boutons, messages d'erreur visibles). **Les images quittent alors le PC.** Si le modèle refuse les images, le texte est rédigé sans elles.

La pastille et l'encre n'apparaissent jamais sur les captures. Écran principal ; 5 minutes au plus.

## La fenêtre principale

Icône → **Ouvrir VoiceToText** (ou **Historique…**, **Réglages…** pour aller directement à la page).

### Accueil

Mots dictés, temps gagné (comparé à une frappe à 40 mots/min), vitesse de parole, coût LLM du mois, et un graphique des mots dictés par jour sur 30 jours (détail au survol, tableau des valeurs dans « Voir les données »). Un bandeau signale les nouvelles versions.

**Par application** : mots, part des mots, dictées et coût pour chaque programme où vous dictez (Slack, Chrome, VS Code…), ce mois-ci ou depuis le début. Seul le nom du programme est compté (`slack.exe`), jamais le titre de la fenêtre ; comptage à partir de la version 0.10.

### Historique

Les 1 000 dernières dictées : date, durée, application de destination, **coût de chaque dictée** (jetons au survol), texte final, transcription brute (ou consigne pour une commande). Recherche, copie, effacement complet (deux clics). Badges : *commande*, *snippet*, *non nettoyé*, *traduit*, *secours*.

### Dictionnaire

Noms propres, jargon, sigles : leur orthographe est transmise à Whisper et au LLM. **Ajouter** (ou Entrée), **×** pour retirer, **Enregistrer** : appliqué immédiatement, sans redémarrer.

**Suggestions** : sous la liste, VoiceToText propose des mots tirés de l'historique (analysé sur le PC) : noms propres et sigles qui reviennent dans plusieurs dictées, et mots que le LLM a dû corriger parce que Whisper les avait mal transcrits (marqués *corrigé*). **+** ajoute le mot au dictionnaire (enregistré tout de suite), **×** ne le propose plus. Sans historique, pas de suggestions.

### Snippets

Un **déclencheur** et un **texte** : dicter **seulement** le déclencheur (« ma signature ») insère le texte tel quel, sans LLM (instantané et gratuit). Casse, accents et ponctuation ne comptent pas ; un déclencheur prononcé au milieu d'une phrase ne déclenche rien.

### Styles

Consignes de nettoyage selon l'application où vous dictez, d'après la fenêtre active au début de la dictée :

- **Applications** : exécutables (`slack.exe`, `outlook.exe`…) ;
- **Titre de fenêtre contenant** : pour un site dans le navigateur (`Gmail`, `WhatsApp`) ;
- **Consignes** : « ton conversationnel, phrases courtes » ;
- ou **texte brut** (case décochée) : par exemple pour un éditeur de code.

Modèles prêts : **Messagerie**, **E-mail**, **Code**. Le premier style qui correspond s'applique (**↑** pour changer l'ordre).

### Prompts

Les consignes données au LLM, pour le nettoyage des dictées et pour le Command Mode. Le prompt intégré (visible en lecture seule) convient dans la plupart des cas. **Créer une copie modifiable** pour l'adapter (plus concis, sans listes, autre registre, consignes simplifiées pour un petit modèle local), **Tester** sur un exemple avec le fournisseur actif, choisir le **Prompt actif**, **Enregistrer** : appliqué immédiatement. Le dictionnaire et les consignes de style sont ajoutés automatiquement à la fin. Le choix du prompt fait partie du profil LLM (un profil Ollama peut avoir son propre prompt).

### Réglages

- **Raccourcis** : touche de dictée, touche du Command Mode, « coller la version brute », « réessayer », appui minimal, mains libres.
- **Pendant la dictée** : sons, pause de la musique et des vidéos, commandes vocales.
- **Micro et transcription** : micro (celui par défaut si le choisi est débranché), [modèle de transcription](#modèles-de-transcription) (Whisper, Parakeet, Vosk), carte graphique, [libération de la mémoire graphique](#libérer-la-mémoire-graphique).
- **Mises à jour** : vérification automatique.
- **Démarrage** : lancement à l'ouverture de session.
- **Exporter / importer les réglages** : voir ci-dessous.
- **Nettoyage du texte** : profils, fournisseur LLM, modèle, délai, tarif, paramètres avancés, **Tester ces réglages**, **Appliquer**.
- **Clé API** (pour l'adresse saisie), **Clés API enregistrées** (une par fournisseur : remplacer, supprimer ; les clés ne sont jamais affichées, ni en entier ni en partie), **Consommation** (requêtes, jetons, coût du jour, du mois, total).

Tout s'applique sans redémarrer : **Appliquer** prend en compte raccourcis, micro, modèle de transcription, mémoire graphique, options de dictée et mises à jour. Une dictée en cours se termine avec les anciens réglages ; la suivante utilise les nouveaux. Le nettoyage du texte (**Appliquer**), le démarrage avec Windows et la clé API s'appliquent aussi immédiatement.

#### Exporter / importer les réglages

**Exporter…** enregistre dans un fichier `.toml` la configuration, les prompts, les profils LLM, le dictionnaire, les snippets et les styles : pour un autre PC ou une sauvegarde. **Les clés API ne sont jamais exportées** (elles restent dans le Gestionnaire d'identifiants de ce PC) ; par précaution, un paramètre avancé qui ressemble à un secret (`api_key`, `token`, `Authorization`…) est aussi retiré du fichier, et signalé.

**Importer…** vérifie le fichier comme si chaque réglage avait été saisi à la main, garde les réglages actuels dans `config.avant-import.toml`, puis applique tout sans redémarrer. Sur un nouveau PC, saisissez ensuite la clé API.

#### Modèles de transcription

Trois moteurs, tous locaux (l'audio ne quitte jamais le PC) :

| Moteur | Modèle | Téléchargement | Pour qui |
|---|---|---|---|
| Whisper | Large v3 Turbo, 8 bits (**recommandé**) | 874 Mo | Carte graphique récente |
| Whisper | Large v3 Turbo, pleine précision | 1,6 Go | Un peu plus précis |
| Whisper | Large v3 Turbo, 5 bits | 574 Mo | Moins de mémoire graphique |
| Whisper | Large v3, 5 bits | 1,1 Go | Plus précis, plus lent |
| Whisper | Small, 8 bits | 264 Mo | Petite carte graphique |
| **Parakeet** (NVIDIA) | TDT 0.6B v3, 8 bits | 753 Mo | **PC sans carte graphique** : rapide sur le processeur, ponctué |
| **Vosk** | Small français | 57 Mo | Vieux PC, peu de mémoire |

- **Whisper** : le plus précis ; quasi instantané avec une carte graphique, lent sur le processeur seul.
- **Parakeet** : conçu pour le processeur (plusieurs fois plus rapide que le temps réel sur un processeur ordinaire), ponctuation et majuscules comprises, langue détectée automatiquement. Le téléchargement comprend ONNX Runtime (bibliothèque officielle de Microsoft).
- **Vosk** : quelques dizaines de Mo, fonctionne partout, mais moins précis et **sans ponctuation ni majuscules** : gardez le [nettoyage LLM](#nettoyage-du-texte-llm) activé (sans lui, seuls une majuscule initiale et un point final sont ajoutés). Le téléchargement comprend la bibliothèque Vosk.

Avec Parakeet et Vosk, la case « carte graphique » est sans effet, et le dictionnaire ne sert qu'au nettoyage LLM (seul Whisper l'utilise aussi pour transcrire).

Un nouveau modèle est téléchargé dès **Appliquer** (progression dans la pastille), chaque fichier est vérifié (taille et empreinte SHA-256), puis il remplace l'ancien. Les dictées continuent avec l'ancien modèle pendant le téléchargement. Les bibliothèques de Parakeet et Vosk ne sont téléchargées que si vous choisissez ces moteurs : l'installeur ne grossit pas.

#### Libérer la mémoire graphique

Le modèle Whisper occupe en permanence de la mémoire graphique (environ 1 à 1,5 Go avec le modèle recommandé). Pour la rendre aux jeux ou à d'autres logiciels :

- **Libérer la mémoire graphique sans dictée depuis** 5, 15, 30 ou 60 minutes (par défaut : jamais) ;
- **pendant les jeux et vidéos en plein écran** (une minute après la dernière dictée, pour ne pas recharger à chaque message dicté en jeu).

Le modèle est rechargé **dès que vous commencez à dicter**, pendant que vous parlez : seule cette dictée prend une à deux secondes de plus. Le journal indique « modèle déchargé » et le temps de rechargement.

## Nettoyage du texte (LLM)

Le texte transcrit (jamais l'audio) est envoyé au LLM qui retire les hésitations (« euh », « du coup »), applique les auto-corrections (« mardi, non, mercredi » → « mercredi »), corrige ponctuation et accords, met les énumérations en liste. Le texte dicté est traité comme une donnée : une phrase dictée comme « écris-moi un poème » est réécrite, pas exécutée.

### Changer de fournisseur

**Réglages → Nettoyage du texte** :

1. **Fournisseur** : DeepSeek, GLM (Z.ai), OpenAI, Mistral, OpenRouter, **Ollama (ce PC ou réseau local)** ou toute API compatible OpenAI. Le préréglage remplit l'adresse ; renseigner le modèle et son tarif.
2. **Clé API** (section suivante) : saisir la clé du fournisseur. **Chaque adresse garde sa propre clé**.
3. **Tester ces réglages** : envoie une phrase d'exemple avec ce qui est saisi, **sans rien enregistrer**, et affiche un compte rendu : adresse exacte appelée, clé envoyée ou non, durée, réponse ou erreur, jetons et coût, **modèles disponibles sur le serveur** (un clic en choisit un), et des **pistes** en cas d'échec (« ajoutez /v1 », « rien n'écoute sur ce port », « nom de modèle inexact »…).
4. **Appliquer** : actif dès la dictée suivante, sans redémarrer.

**Profils** : « Enregistrer comme profil » garde la configuration sous un nom (« DeepSeek », « Ollama du NAS »…) ; la liste **Profil** permet ensuite de passer de l'un à l'autre (choisir, puis **Appliquer**). Le profil actif est marqué *(actif)*.

**Profil de secours** : si le fournisseur échoue (panne, délai dépassé, clé refusée, réponse vide), le texte est envoyé au profil choisi dans **Profil de secours** avant de se replier sur le texte brut. Exemple : DeepSeek en principal, l'Ollama du réseau local en secours. Le Command Mode en profite aussi. L'historique marque ces dictées *secours : nom du profil* et leur coût suit le tarif du profil de secours. Le délai total peut atteindre la somme des deux délais.

Adresses : l'application ajoute elle-même `/chat/completions` (s'il a été recopié, il est retiré). Pour **Ollama** ou **LM Studio**, l'adresse se termine par `/v1` : `http://localhost:11434/v1` sur ce PC, `http://192.168.1.20:11434/v1` sur une autre machine du réseau (Ollama lancé avec `OLLAMA_HOST=0.0.0.0`).

- La clé n'est envoyée qu'en HTTPS, ou en HTTP vers ce PC : **jamais en clair sur le réseau** (un serveur local du réseau fonctionne sans clé).
- HTTP n'est accepté que vers ce PC ou le réseau local (adresses privées, noms en `.local`).
- GLM-5.3 ne permet pas de désactiver la « réflexion » (plus lent) : le préréglage propose glm-5.2.

### Coût

Chaque réponse indique les jetons consommés ; le coût est estimé avec le tarif saisi (par défaut, le plein tarif de deepseek-flash : estimation haute, les heures creuses coûtent moitié moins). Ordre de grandeur : **environ 1 centime pour 100 dictées**. Totaux dans **Réglages → Consommation** et sur l'accueil, coût de chaque dictée dans l'historique.

### Sans LLM

Nettoyage désactivé (case décochée) ou sans clé : le texte de Whisper est inséré tel quel. Whisper ponctue et met les majuscules, mais garde les hésitations et les reprises. Les snippets fonctionnent ; le Command Mode et les consignes de style, non.

Pour garder le nettoyage **sans rien envoyer sur Internet** : installer [Ollama](https://ollama.com), un modèle (`ollama pull qwen3:8b` par exemple), puis choisir le préréglage **Ollama**. Plus lent qu'un service en ligne, gratuit et privé.

## Mises à jour

VoiceToText vérifie les nouvelles versions une minute après le démarrage puis toutes les 6 heures (désactivable dans **Réglages → Mises à jour**). Quand une version est disponible :

- une notification Windows l'annonce (une fois par version) ;
- le menu de l'icône propose **Installer la version x.y.z…** et l'accueil affiche un bandeau **Mettre à jour**.

Rien n'est installé sans votre clic. Chaque mise à jour est **signée** : la signature (et la version qu'elle couvre) est vérifiée avant l'installation, l'application redémarre ensuite toute seule.

Pour vérifier tout de suite : **Réglages → Mises à jour → Vérifier les mises à jour**. La page indique si VoiceToText est à jour ; sinon, elle affiche la nouvelle version et propose **Télécharger et installer**, avec la progression du téléchargement. Le menu de l'icône propose aussi **Rechercher une mise à jour**.

Les versions antérieures à la 0.5 ne savent pas se mettre à jour : installer la 0.5 une fois à la main.

## Confidentialité et sécurité

| Donnée | Où elle va |
|---|---|
| Audio | **Reste sur le PC** (transcription locale) ; seule la dernière dictée est gardée, en mémoire, pour « Réessayer » |
| Texte transcrit | Envoyé au fournisseur LLM choisi (rien avec Ollama ou le nettoyage désactivé) |
| Texte sélectionné (Command Mode) | Envoyé au fournisseur LLM avec la consigne |
| Clé API | Gestionnaire d'identifiants Windows (`VoiceToText/llm-api-key…`), jamais en clair sur le disque |
| Historique | `history.jsonl` sur le PC, en clair, protégé par votre session Windows ; désactivable, effaçable |
| Réunions | Transcription et compte rendu dans `meetings\` sur le PC (en clair, comme l'historique) ; audio en mémoire, ou chiffré par Windows (DPAPI) et supprimé ou conservé selon le réglage ; jamais d'audio en clair sur le disque (sauf export WAV demandé) |
| Compte rendu de réunion | La transcription est envoyée au fournisseur LLM choisi |
| Fichiers audio / vidéo importés | Lus sur le PC, ni copiés ni modifiés ; seule leur transcription est gardée (et envoyée au LLM pour le compte rendu) |
| Questions sur les enregistrements | Les passages qui correspondent à la question sont envoyés au fournisseur LLM choisi |
| Statistiques et coût | `usage.json` et `usage-apps.json` : compteurs uniquement (par application : le nom du programme), **aucun texte** |
| Export des réglages | Fichier `.toml` où vous le choisissez ; **jamais de clé API** |
| Journal | Durées, tailles, erreurs ; **jamais le texte dicté** |
| Titre de la fenêtre active | Sert à choisir un style ; ni stocké, ni journalisé, ni envoyé |
| Captures d'écran (capture commentée) | Restent sur le PC : en mémoire, ou dans `captures\` si vous choisissez de les garder ; **jamais envoyées**, sauf si vous activez « Faire décrire les images par le LLM » |
| Clics de la souris | Pendant une capture commentée seulement (position et moment), pour marquer les images ; ni stockés à part, ni envoyés |
| Programmes qui utilisent le micro | Nom seulement, lu chaque seconde pour la détection d'appel ; ni stocké, ni envoyé (désactivable) |
| Vérification des mises à jour | Requête vers github.com (votre adresse IP est visible de GitHub) |

## Fichiers et emplacements

| Fichier | Emplacement |
|---|---|
| Configuration | `%APPDATA%\VoiceToText\config.toml` |
| Modèles de transcription (et bibliothèques de Parakeet, Vosk) | `%LOCALAPPDATA%\VoiceToText\models\` |
| Historique | `%LOCALAPPDATA%\VoiceToText\history.jsonl` |
| Réunions (une par dossier) | `%LOCALAPPDATA%\VoiceToText\meetings\` |
| Captures commentées gardées | `%LOCALAPPDATA%\VoiceToText\captures\` |
| Statistiques et coût | `%LOCALAPPDATA%\VoiceToText\usage.json`, `usage-apps.json` |
| Réglages remplacés par un import | `%APPDATA%\VoiceToText\config.avant-import.toml` |
| Journal | `%LOCALAPPDATA%\VoiceToText\logs\voicetotext.log` (purgé au-delà de 3 Mo, les 100 dernières lignes sont gardées) |

Menu de l'icône : **Modifier la configuration** (ouvre config.toml), **Ouvrir les journaux**, **Redémarrer**.

Désinstaller : Paramètres Windows → Applications → VoiceToText. Les fichiers ci-dessus et la clé API (Gestionnaire d'identifiants Windows → Identifiants Windows → `VoiceToText/…`) peuvent ensuite être supprimés à la main.

## Configuration avancée (config.toml)

Tout se règle dans la fenêtre principale ; le fichier reste modifiable à la main (menu **Modifier la configuration**, puis **Redémarrer**). Les clés absentes prennent leur valeur par défaut. Exemple complet :

```toml
# Noms propres, jargon : orthographe transmise à Whisper et au LLM.
# (clé de premier niveau : à placer avant toute section [..])
dictionary = ["Aquadrox", "Tauri"]

[hotkey]
key = "AltGr"        # ou RightCtrl, RightShift, CapsLock, ScrollLock, Pause, Insert, Menu, F1…F24
min_hold_ms = 250    # appui plus court : ignoré
hands_free = true    # double appui → mains libres
paste_raw = "Alt+Shift+V"   # coller la dernière dictée brute ("" = désactivé)
retry = "Alt+Shift+R"       # réessayer la dernière dictée ("" = désactivé)
command_key = "RightCtrl"   # tenue avec le raccourci → Command Mode ("" = désactivé)
translate = "Alt+Shift+T"   # active ou coupe le mode traduction ("" = désactivé)
meeting = "Alt+Shift+M"     # démarre ou arrête une réunion ("" = désactivé)
meeting_copy = "Alt+Shift+N"  # colle les nouvelles répliques de la réunion
annotate = "Alt+Shift+A"    # capture commentée : appui démarre/arrête, maintien dessine

[translation]
language = "anglais"   # langue d'arrivée du mode traduction

[captures]
template = "explication"  # "explication", "bug", "consigne" ou "brut" (sans LLM)
keep_days = 0             # captures gardées sur le disque ; 0 = jamais
area = "ecran"            # "ecran", "fenetre" (active) ou "zone" (tracée)
clicks = true             # clics = moments clés
vision = false            # envoyer les images au LLM pour les décrire
vision_profile = ""       # profil LLM pour les images ; "" = fournisseur actif

[meetings]
audio = "temporaire"   # "memoire", "temporaire" (chiffré, supprimé à la fin) ou "conserve"
system_audio = true    # capter le son du PC (les autres participants)
auto_summary = true    # compte rendu par le LLM à la fin
summary_prompt = ""    # prompt « compte_rendu » (page Prompts) ; "" = intégré
keep_days = 0          # réunions supprimées au-delà ; 0 = jamais
detect = true          # proposer d'enregistrer quand un appel commence
echo_filter = true     # écarter l'écho (haut-parleurs)
summary_language = ""  # langue du compte rendu ; "" = celle de la réunion
# Choix par service (absent = proposer) : "toujours" ou "jamais".
# [[meetings.providers]]
# key = "zoom"
# policy = "toujours"

[dictation]
sounds = true          # sons de début et de fin
pause_media = true     # pause de la musique et des vidéos pendant la dictée
voice_commands = true  # « nouvelle ligne », « annule la dernière dictée »…

[audio]
device = ""          # nom du micro ; vide = micro par défaut de Windows

[asr]
model = "ggml-large-v3-turbo-q8_0.bin"
language = "fr"
use_gpu = true
unload_after_min = 0          # libérer la mémoire graphique après N min sans dicter (0 = jamais)
unload_in_fullscreen = false  # la libérer pendant les jeux et vidéos en plein écran

[llm]
enabled = true
base_url = "https://api.deepseek.com"
model = "deepseek-flash"
timeout_ms = 3000    # au-delà, le texte brut est inséré (30 s minimum pour le Command Mode)
fallback_profile = "Ollama du NAS"   # profil essayé si celui-ci échoue ("" ou absent = aucun)

# Tarif par million de jetons, pour le coût estimé.
[llm.pricing]
input_per_million = 0.30
cached_input_per_million = 0.006
output_per_million = 1.20
currency = "USD"

# Champs propres au fournisseur ajoutés à chaque requête.
[llm.extra_body]
thinking = { type = "disabled" }   # DeepSeek / GLM : pas de « réflexion » (lente)

[history]
enabled = true       # dictées conservées sur ce PC uniquement
max_entries = 1000

[updates]
check = true         # vérifier les nouvelles versions (installation sur clic seulement)

# Snippets : dicter seulement le déclencheur insère le texte tel quel.
[[snippets]]
trigger = "ma signature"
text = """
Florian
VoiceToText"""

# Styles par application : le premier qui correspond s'applique.
[[styles]]
apps = ["slack.exe", "discord.exe", "ms-teams.exe"]
titles = ["WhatsApp"]
instructions = "Messagerie : ton conversationnel, phrases courtes, pas de formule de politesse."

[[styles]]
apps = ["code.exe"]
cleanup = false      # texte brut, sans LLM

# Prompts personnalisés (page Prompts) ; actifs via llm.cleanup_prompt /
# llm.command_prompt (vide = prompt intégré).
[[prompts]]
name = "Concis"
kind = "nettoyage"   # ou "commande"
text = "Corrige la ponctuation et retire les hésitations. Réponds uniquement avec le texte final."

# Profils LLM enregistrés (Réglages → Nettoyage du texte → Profil).
[[llm_profiles]]
name = "Ollama du NAS"
base_url = "http://192.168.1.20:11434/v1"
model = "qwen3:8b"
timeout_ms = 15000
extra_body = {}
```

## Dépannage

| Problème | Piste |
|---|---|
| Rien n'est collé dans une application | Application lancée **en administrateur** : Windows bloque l'insertion depuis une application normale. La relancer sans droits administrateur, ou copier le texte depuis l'**Historique**. |
| La pastille affiche une erreur | Message complet dans le journal (menu **Ouvrir les journaux**). |
| Texte non nettoyé (badge *non nettoyé*) | Clé absente ou refusée (401), solde épuisé (402), délai dépassé : **Réglages → Tester ces réglages** affiche la cause et des pistes. |
| Serveur local (Ollama, LM Studio) injoignable | L'adresse doit se terminer par `/v1` ; sur une autre machine, Ollama doit écouter sur le réseau (`OLLAMA_HOST=0.0.0.0`) et le pare-feu autoriser le port 11434. **Tester ces réglages** liste les modèles si l'adresse est bonne. |
| AltGr ne déclenche rien | Vérifier **Réglages → Raccourcis** ; une autre application peut capter la touche. Essayer F13 ou Ctrl droit. |
| Alt+Maj+V sans effet | Raccourci pris par une autre application (voir le journal) : en choisir un autre dans **Réglages → Raccourcis**. Si plusieurs dispositions de clavier sont installées, Alt+Maj change aussi la langue : choisir un autre raccourci. |
| Mauvais micro | **Réglages → Micro**. |
| Transcription lente | Vérifier que **Utiliser la carte graphique** est coché ; sinon choisir le modèle Small. |
| Mots mal orthographiés (noms propres) | Les ajouter au **Dictionnaire**. |
| Téléchargement du modèle échoué | Vérifier la connexion puis cliquer **Appliquer** dans **Réglages** (le téléchargement recommence). |

## Développement

Le code (Rust, Tauri 2) est organisé en crates : `vtt-core` (logique portable), `vtt-asr` (Whisper, Parakeet, Vosk), `vtt-llm` (nettoyage), `vtt-platform` (Windows : clavier, micro, presse-papier), `apps/desktop` (application PC), et pour Android `vtt-android` (moteur appelé en JNI) et `apps/android` (clavier, bulle et réglages en Kotlin). Les installeurs et l'APK sont produits par la CI GitHub Actions ; une PR qui ne touche que le PC ne recompile pas Android, et inversement.

Android (SDK avec `ndk;29.0.14206865` et `cmake;3.31.6`, cible Rust `aarch64-linux-android`, `cargo-ndk`) :

```sh
apps/android/build-native.sh               # moteur Rust → app/src/main/jniLibs
(cd apps/android && ./gradlew assembleDebug)  # APK de test (VoiceToText (test))
```

Signature des APK publiés : clé générée par le mainteneur, jamais dans le dépôt ; la CI la lit dans les secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, seulement pour les tags.

```sh
cargo test --workspace
# Transcrire un WAV avec le pipeline complet (sans micro ni clavier) :
cargo run --release -p vtt-asr --example transcribe_wav -- <modèle.bin> <fichier.wav>
```


Documentation technique :

- [Fonctions](docs/FONCTIONS.md)
- [Architecture et choix techniques](docs/ARCHITECTURE.md)
- [Roadmap et avancement](docs/ROADMAP.md)

## Licence

[MIT](LICENSE) © 2026 Florian Langlois : utilisation, modification et redistribution libres, y compris dans d'autres projets, à condition de conserver la mention de copyright. Logiciel fourni sans garantie.

Composants tiers : [whisper.cpp](https://github.com/ggml-org/whisper.cpp) et les modèles [Whisper](https://github.com/openai/whisper) (MIT), [Tauri](https://tauri.app) (MIT / Apache 2.0). Téléchargés seulement si vous les choisissez : le modèle [Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) © NVIDIA ([CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), conversion ONNX de [istupakov](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx)), [ONNX Runtime](https://github.com/microsoft/onnxruntime) (MIT, Microsoft), [Vosk](https://alphacephei.com/vosk/) et son modèle français (Apache 2.0, Alpha Cephei) ; [parakeet-rs](https://github.com/altunenes/parakeet-rs) (MIT / Apache 2.0). Le service LLM choisi (DeepSeek, OpenAI…) reste soumis à ses propres conditions d'utilisation.
