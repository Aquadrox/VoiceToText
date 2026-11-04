# VoiceToText

Dictée vocale pour Windows, inspirée de Wispr Flow : **maintenir AltGr, parler, relâcher**. Le texte est transcrit sur votre PC, nettoyé (hésitations, ponctuation, auto-corrections) puis collé dans l'application active : traitement de texte, messagerie, navigateur, e-mail…

- **Transcription locale** : Whisper tourne sur votre carte graphique, l'audio ne quitte jamais le PC.
- **Nettoyage par LLM** (DeepSeek par défaut, ou GLM, OpenAI, Mistral, OpenRouter, un modèle local Ollama…), désactivable.
- **Mains libres**, **Command Mode** (« traduis ça en anglais »), **snippets**, **dictionnaire**, **styles par application**, **historique**, **statistiques**, **suivi du coût**.
- Français pour l'instant ; Windows d'abord, Linux et Android plus tard.

## Sommaire

1. [Installation](#installation)
2. [Premiers pas](#premiers-pas)
3. [Dicter](#dicter)
4. [Commandes vocales](#commandes-vocales)
5. [Command Mode](#command-mode)
6. [Coller la version brute](#coller-la-version-brute)
7. [Réessayer une dictée](#réessayer-une-dictée)
8. [Mode traduction](#mode-traduction)
9. [La fenêtre principale](#la-fenêtre-principale)
10. [Nettoyage du texte (LLM)](#nettoyage-du-texte-llm)
11. [Mises à jour](#mises-à-jour)
12. [Confidentialité et sécurité](#confidentialité-et-sécurité)
13. [Fichiers et emplacements](#fichiers-et-emplacements)
14. [Configuration avancée (config.toml)](#configuration-avancée-configtoml)
15. [Dépannage](#dépannage)
16. [Développement](#développement)
17. [Licence](#licence)

## Installation

1. Télécharger `VoiceToText_x.y.z_x64-setup.exe` depuis la page [Releases](https://github.com/Aquadrox/VoiceToText/releases/latest).
2. Le lancer : installation pour l'utilisateur courant, **sans droits administrateur**. Windows SmartScreen peut afficher « Windows a protégé votre ordinateur » (l'installeur n'est pas signé par un éditeur reconnu) : **Informations complémentaires → Exécuter quand même**.
3. Au premier lancement, le modèle Whisper (874 Mo) se télécharge ; la progression s'affiche dans la pastille en bas de l'écran. Les lancements suivants prennent environ une seconde.

L'application est **discrète** : une icône dans la zone de notification (à côté de l'horloge) et une pastille qui n'apparaît que pendant la dictée. La fenêtre principale ne s'ouvre jamais d'elle-même.

Pour la lancer à chaque ouverture de session : **Réglages → Démarrage → Lancer VoiceToText à l'ouverture de session Windows**.

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
- **Micro et transcription** : micro (celui par défaut si le choisi est débranché), modèle Whisper, carte graphique, [libération de la mémoire graphique](#libérer-la-mémoire-graphique).
- **Mises à jour** : vérification automatique.
- **Démarrage** : lancement à l'ouverture de session.
- **Exporter / importer les réglages** : voir ci-dessous.
- **Nettoyage du texte** : profils, fournisseur LLM, modèle, délai, tarif, paramètres avancés, **Tester ces réglages**, **Appliquer**.
- **Clé API** (pour l'adresse saisie), **Clés API enregistrées** (une par fournisseur : remplacer, supprimer ; les clés ne sont jamais affichées, ni en entier ni en partie), **Consommation** (requêtes, jetons, coût du jour, du mois, total).

Tout s'applique sans redémarrer : **Appliquer** prend en compte raccourcis, micro, modèle Whisper, mémoire graphique, options de dictée et mises à jour. Une dictée en cours se termine avec les anciens réglages ; la suivante utilise les nouveaux. Le nettoyage du texte (**Appliquer**), le démarrage avec Windows et la clé API s'appliquent aussi immédiatement.

#### Exporter / importer les réglages

**Exporter…** enregistre dans un fichier `.toml` la configuration, les prompts, les profils LLM, le dictionnaire, les snippets et les styles : pour un autre PC ou une sauvegarde. **Les clés API ne sont jamais exportées** (elles restent dans le Gestionnaire d'identifiants de ce PC) ; par précaution, un paramètre avancé qui ressemble à un secret (`api_key`, `token`, `Authorization`…) est aussi retiré du fichier, et signalé.

**Importer…** vérifie le fichier comme si chaque réglage avait été saisi à la main, garde les réglages actuels dans `config.avant-import.toml`, puis applique tout sans redémarrer. Sur un nouveau PC, saisissez ensuite la clé API.

Modèles Whisper proposés :

| Modèle | Taille | Pour qui |
|---|---|---|
| Large v3 Turbo, 8 bits (**recommandé**) | 874 Mo | Carte graphique récente |
| Large v3 Turbo, pleine précision | 1,6 Go | Un peu plus précis |
| Large v3 Turbo, 5 bits | 574 Mo | Moins de mémoire graphique |
| Large v3, 5 bits | 1,1 Go | Plus précis, plus lent |
| Small, 8 bits | 264 Mo | PC sans carte graphique |

Un nouveau modèle est téléchargé dès **Appliquer** (progression dans la pastille), son empreinte SHA-256 est vérifiée, puis il remplace l'ancien. Les dictées continuent avec l'ancien modèle pendant le téléchargement.

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
| Statistiques et coût | `usage.json` et `usage-apps.json` : compteurs uniquement (par application : le nom du programme), **aucun texte** |
| Export des réglages | Fichier `.toml` où vous le choisissez ; **jamais de clé API** |
| Journal | Durées, tailles, erreurs ; **jamais le texte dicté** |
| Titre de la fenêtre active | Sert à choisir un style ; ni stocké, ni journalisé, ni envoyé |
| Vérification des mises à jour | Requête vers github.com (votre adresse IP est visible de GitHub) |

## Fichiers et emplacements

| Fichier | Emplacement |
|---|---|
| Configuration | `%APPDATA%\VoiceToText\config.toml` |
| Modèles Whisper | `%LOCALAPPDATA%\VoiceToText\models\` |
| Historique | `%LOCALAPPDATA%\VoiceToText\history.jsonl` |
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

[translation]
language = "anglais"   # langue d'arrivée du mode traduction

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

Le code (Rust, Tauri 2) est organisé en crates : `vtt-core` (logique portable), `vtt-asr` (Whisper), `vtt-llm` (nettoyage), `vtt-platform` (Windows : clavier, micro, presse-papier) et `apps/desktop` (application). Les installeurs sont produits par la CI GitHub Actions.

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

Composants tiers : [whisper.cpp](https://github.com/ggml-org/whisper.cpp) et les modèles [Whisper](https://github.com/openai/whisper) (MIT), [Tauri](https://tauri.app) (MIT / Apache 2.0). Le service LLM choisi (DeepSeek, OpenAI…) reste soumis à ses propres conditions d'utilisation.
