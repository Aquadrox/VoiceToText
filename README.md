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
4. [Command Mode](#command-mode)
5. [Coller la version brute](#coller-la-version-brute)
6. [La fenêtre principale](#la-fenêtre-principale)
7. [Nettoyage du texte (LLM)](#nettoyage-du-texte-llm)
8. [Mises à jour](#mises-à-jour)
9. [Confidentialité et sécurité](#confidentialité-et-sécurité)
10. [Fichiers et emplacements](#fichiers-et-emplacements)
11. [Configuration avancée (config.toml)](#configuration-avancée-configtoml)
12. [Dépannage](#dépannage)
13. [Développement](#développement)
14. [Licence](#licence)

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

Limites : 5 minutes par dictée maintenue, 10 minutes en mains libres (la dictée est alors traitée automatiquement).

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

## La fenêtre principale

Icône → **Ouvrir VoiceToText** (ou **Historique…**, **Réglages…** pour aller directement à la page).

### Accueil

Mots dictés, temps gagné (comparé à une frappe à 40 mots/min), vitesse de parole, coût LLM du mois, et un graphique des mots dictés par jour sur 30 jours (détail au survol, tableau des valeurs dans « Voir les données »). Un bandeau signale les nouvelles versions.

### Historique

Les 1 000 dernières dictées : date, durée, application de destination, **coût de chaque dictée** (jetons au survol), texte final, transcription brute (ou consigne pour une commande). Recherche, copie, effacement complet (deux clics). Badges : *commande*, *snippet*, *non nettoyé*.

### Dictionnaire

Noms propres, jargon, sigles : leur orthographe est transmise à Whisper et au LLM. **Ajouter** (ou Entrée), **×** pour retirer, **Enregistrer** : appliqué immédiatement, sans redémarrer.

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

- **Raccourcis** : touche de dictée, touche du Command Mode, « coller la version brute », appui minimal, mains libres.
- **Micro et transcription** : micro (celui par défaut si le choisi est débranché), modèle Whisper, carte graphique.
- **Mises à jour** : vérification automatique.
- **Démarrage** : lancement à l'ouverture de session.
- **Nettoyage du texte** : profils, fournisseur LLM, modèle, délai, tarif, paramètres avancés, **Tester ces réglages**, **Appliquer**.
- **Clé API** (pour l'adresse saisie), **Consommation** (requêtes, jetons, coût du jour, du mois, total).

Raccourcis, micro, modèle Whisper et mises à jour s'appliquent avec **Enregistrer et redémarrer** (quelques secondes). Le nettoyage du texte (**Appliquer**), le démarrage avec Windows et la clé API s'appliquent immédiatement, sans redémarrer.

Modèles Whisper proposés :

| Modèle | Taille | Pour qui |
|---|---|---|
| Large v3 Turbo, 8 bits (**recommandé**) | 874 Mo | Carte graphique récente |
| Large v3 Turbo, pleine précision | 1,6 Go | Un peu plus précis |
| Large v3 Turbo, 5 bits | 574 Mo | Moins de mémoire graphique |
| Large v3, 5 bits | 1,1 Go | Plus précis, plus lent |
| Small, 8 bits | 264 Mo | PC sans carte graphique |

Un nouveau modèle est téléchargé au redémarrage ; son empreinte SHA-256 est vérifiée.

## Nettoyage du texte (LLM)

Le texte transcrit (jamais l'audio) est envoyé au LLM qui retire les hésitations (« euh », « du coup »), applique les auto-corrections (« mardi, non, mercredi » → « mercredi »), corrige ponctuation et accords, met les énumérations en liste. Le texte dicté est traité comme une donnée : une phrase dictée comme « écris-moi un poème » est réécrite, pas exécutée.

### Changer de fournisseur

**Réglages → Nettoyage du texte** :

1. **Fournisseur** : DeepSeek, GLM (Z.ai), OpenAI, Mistral, OpenRouter, **Ollama (ce PC ou réseau local)** ou toute API compatible OpenAI. Le préréglage remplit l'adresse ; renseigner le modèle et son tarif.
2. **Clé API** (section suivante) : saisir la clé du fournisseur. **Chaque adresse garde sa propre clé**.
3. **Tester ces réglages** : envoie une phrase d'exemple avec ce qui est saisi, **sans rien enregistrer**, et affiche un compte rendu : adresse exacte appelée, clé envoyée ou non, durée, réponse ou erreur, jetons et coût, **modèles disponibles sur le serveur** (un clic en choisit un), et des **pistes** en cas d'échec (« ajoutez /v1 », « rien n'écoute sur ce port », « nom de modèle inexact »…).
4. **Appliquer** : actif dès la dictée suivante, sans redémarrer.

**Profils** : « Enregistrer comme profil » garde la configuration sous un nom (« DeepSeek », « Ollama du NAS »…) ; la liste **Profil** permet ensuite de passer de l'un à l'autre (choisir, puis **Appliquer**). Le profil actif est marqué *(actif)*.

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

Rien n'est installé sans votre clic. Chaque mise à jour est **signée** : la signature (et la version qu'elle couvre) est vérifiée avant l'installation, l'application redémarre ensuite toute seule. Menu de l'icône → **Rechercher une mise à jour** pour vérifier tout de suite.

Les versions antérieures à la 0.5 ne savent pas se mettre à jour : installer la 0.5 une fois à la main.

## Confidentialité et sécurité

| Donnée | Où elle va |
|---|---|
| Audio | **Reste sur le PC** (transcription locale) |
| Texte transcrit | Envoyé au fournisseur LLM choisi (rien avec Ollama ou le nettoyage désactivé) |
| Texte sélectionné (Command Mode) | Envoyé au fournisseur LLM avec la consigne |
| Clé API | Gestionnaire d'identifiants Windows (`VoiceToText/llm-api-key…`), jamais en clair sur le disque |
| Historique | `history.jsonl` sur le PC, en clair, protégé par votre session Windows ; désactivable, effaçable |
| Statistiques et coût | `usage.json` : compteurs uniquement, **aucun texte** |
| Journal | Durées, tailles, erreurs ; **jamais le texte dicté** |
| Titre de la fenêtre active | Sert à choisir un style ; ni stocké, ni journalisé, ni envoyé |
| Vérification des mises à jour | Requête vers github.com (votre adresse IP est visible de GitHub) |

## Fichiers et emplacements

| Fichier | Emplacement |
|---|---|
| Configuration | `%APPDATA%\VoiceToText\config.toml` |
| Modèles Whisper | `%LOCALAPPDATA%\VoiceToText\models\` |
| Historique | `%LOCALAPPDATA%\VoiceToText\history.jsonl` |
| Statistiques et coût | `%LOCALAPPDATA%\VoiceToText\usage.json` |
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
command_key = "RightCtrl"   # tenue avec le raccourci → Command Mode ("" = désactivé)

[audio]
device = ""          # nom du micro ; vide = micro par défaut de Windows

[asr]
model = "ggml-large-v3-turbo-q8_0.bin"
language = "fr"
use_gpu = true

[llm]
enabled = true
base_url = "https://api.deepseek.com"
model = "deepseek-flash"
timeout_ms = 3000    # au-delà, le texte brut est inséré (30 s minimum pour le Command Mode)

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
| Téléchargement du modèle échoué | Vérifier la connexion puis **Redémarrer** depuis le menu de l'icône (le téléchargement recommence). |

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
