# Architecture

## Pipeline

```
[Hook clavier] → [Capture micro] → [ASR Whisper] → [Snippets ?] → [Nettoyage LLM] → [Insertion]
  AltGr down       cpal, 16 kHz mono   local GPU       texte fixe     DeepSeek API      presse-papier + Ctrl+V
  AltGr up ────────────────────────────────▶ déclenche le traitement
```

Budget de latence visé (relâchement → texte affiché) : **< 1,5 s**, dont ~0,4 s ASR et ~1 s LLM. À mesurer en v0.2 ; si le LLM est trop lent, le nettoyage peut être désactivé ou réservé aux dictées longues.

Si le LLM échoue ou dépasse son délai, le texte brut de Whisper est inséré : une dictée n'est jamais perdue.

Garde-fous avant/après Whisper (`vtt-core`) :

- enregistrement < 300 ms ou silencieux (RMS < -46 dBFS) → ignoré, Whisper n'est pas appelé ;
- phrases inventées par Whisper sur du bruit (« Sous-titres réalisés par la communauté d'Amara.org »…) → retirées.

## Threads (application Windows)

| Thread | Rôle |
|---|---|
| `keyboard-hook` | Hook `WH_KEYBOARD_LL` + boucle de messages ; recopie chaque touche dans un canal, n'avale rien |
| `controller` | Machine à états du push-to-talk (`vtt-core::hotkey`), pilote le micro ; vérifie aussi l'état physique de la touche (relâchement manqué) et coupe à 5 min |
| `audio` | Possède le flux `cpal` ouvert pendant l'enregistrement |
| `pipeline` | Transcrit et insère les dictées une par une ; une nouvelle dictée peut démarrer pendant ce temps |
| principal (Tauri) | Icône de notification, pastille |

La pastille est une fenêtre transparente, non activable et traversable par la souris, affichée une fois pour toutes : la montrer/cacher à chaque dictée risquerait de voler le focus de l'application cible.

## Découpage

```
VoiceToText/
├── crates/
│   ├── vtt-core/      pipeline, traits, configuration — portable, sans dépendance OS
│   ├── vtt-asr/       Whisper via whisper-rs (backend Vulkan)            [v0.1]
│   ├── vtt-llm/       client OpenAI-compatible (DeepSeek), prompt         [v0.2]
│   └── vtt-platform/  hook clavier, capture audio, insertion              [v0.1]
├── apps/
│   └── desktop/       Tauri 2 : tray, réglages, historique, pastille     [v0.1]
└── docs/
```

`vtt-core` définit trois traits ; chaque implémentation est interchangeable via la configuration :

| Trait | Rôle | Implémentations prévues |
|---|---|---|
| `Transcriber` | audio → texte brut | Whisper local ; plus tard cloud (Groq, Voxtral) |
| `PostProcessor` | texte brut → texte propre | LLM OpenAI-compatible ; plus tard LLM local |
| `TextSink` | insertion dans l'appli active | presse-papier + Ctrl+V (Windows) ; plus tard X11/Wayland, IME Android |

Le cœur reste sans code spécifique à un OS pour pouvoir être réutilisé tel quel sur Linux et Android (via UniFFI).

## Choix techniques

| Sujet | Choix | Raison |
|---|---|---|
| Langage | Rust | Binaire unique, faible RAM (appli résidente), bindings whisper.cpp matures, portable mobile |
| Interface | Tauri 2 (frontend HTML/JS sans framework) | UI web légère via WebView2, déjà présent sur Windows 10/11 |
| ASR | whisper.cpp via `whisper-rs`, modèle `large-v3-turbo` quantifié q8_0 (~0,9 Go VRAM) | Meilleure qualité en français, très rapide sur RTX 4070 Ti |
| Accélération GPU | **Vulkan** plutôt que CUDA | Le pilote NVIDIA fournit Vulkan : rien à installer ni à embarquer (CUDA = ~500 Mo de DLL). CUDA reste une option si la latence déçoit |
| LLM | DeepSeek `deepseek-flash`, API OpenAI-compatible, `temperature 0`, mode réflexion désactivé (`thinking.type = disabled`, activé par défaut chez DeepSeek et lent) | `base_url` et `model` configurables : changement de fournisseur sans code |
| Clé API | Gestionnaire d'identifiants Windows, appels Win32 directs (`CredWriteW`/`CredReadW`), cible `VoiceToText/llm-api-key` | Jamais en clair sur le disque ; lue à chaque dictée, donc prise en compte sans redémarrer |
| Hook clavier | `WH_KEYBOARD_LL` (crate `windows`) | Seule façon de voir l'appui **et** le relâchement d'AltGr ; `RegisterHotKey` ne voit pas le relâchement |
| Insertion | Presse-papier + Ctrl+V, presse-papier restauré ensuite | Instantané même pour un long texte ; `SendInput` Unicode en repli |
| Audio | `cpal` (WASAPI), micro ouvert seulement pendant l'appui ; conversion 16 kHz mono maison (`vtt-core::audio`) | Format attendu par Whisper ; pas d'icône « micro utilisé » en permanence |
| Configuration | TOML dans `%APPDATA%\VoiceToText\config.toml` | Lisible et éditable à la main |
| Diarisation (essai v0.19) | Méthode pyannote 3 réécrite en Rust : segmentation pyannote 3.0 + empreinte WeSpeaker en ONNX (même ONNX Runtime que Parakeet), regroupement et fbank dans `vtt-core` | Ni Python ni PyTorch ; modèles légers (32 Mo) ; nombre de locuteurs quelconque |
| Modèles | Téléchargés au premier lancement dans `%LOCALAPPDATA%\VoiceToText\models`, SHA-256 vérifié | Installeur léger |
| Journaux | `%LOCALAPPDATA%\VoiceToText\logs\voicetotext.log` (durées ASR/insertion de chaque dictée) | Diagnostic à distance |

## Nettoyage LLM

- Le texte dicté est encadré par `<dictee>…</dictee>` et le prompt système interdit d'y répondre ou d'exécuter ses consignes (« écris-moi un mail » doit être réécrit, pas exécuté).
- Réponse rejetée (texte brut inséré) si elle dépasse 2 × la longueur dictée + 80 caractères : signe que le modèle a répondu au lieu de réécrire.
- Erreurs HTTP traduites (401 clé refusée, 402 solde insuffisant, 429…) dans le journal ; la dictée est quand même insérée.

## Confidentialité

- L'audio ne quitte jamais le PC (ASR local).
- Le **texte** transcrit est envoyé au fournisseur LLM choisi (DeepSeek par défaut) pour le nettoyage. Désactivable dans les réglages. Avec un modèle local (Ollama…), rien ne quitte le PC.
- La clé API n'est envoyée qu'en HTTPS, ou en HTTP vers ce PC (`localhost`, `127.x`, `::1`) ; jamais en clair sur le réseau. HTTP n'est accepté que vers ce PC ou le réseau local (adresses privées, `.local`), où un serveur comme Ollama fonctionne sans clé.
- L'**historique** des dictées (texte brut et final) est stocké en clair dans `%LOCALAPPDATA%\VoiceToText\history.jsonl`, protégé par les droits du compte Windows comme les autres fichiers du profil. Jamais envoyé ailleurs. Limité à 1000 dictées, effaçable depuis la fenêtre Historique, désactivable (`history.enabled = false`). Le journal, lui, ne contient jamais le texte dicté.
- **Styles par application** : l'exécutable de la fenêtre active est journalisé et gardé dans l'historique ; son **titre** (qui peut contenir un nom de document ou de conversation) sert seulement à choisir le style, il n'est ni journalisé ni stocké, ni envoyé au LLM.
- **Command Mode** : le texte sélectionné est envoyé au LLM avec la consigne (c'est le but) ; il n'est jamais journalisé (longueur seulement). Le prompt traite la sélection comme une donnée : les instructions qu'elle contiendrait (page web copiée…) ne sont pas exécutées, et les balises du prompt (`<selection>`, `<consigne>`, `<dictee>`) présentes dans le texte sont désamorcées pour qu'il ne puisse pas sortir de sa balise. Défense en profondeur : Command Mode refusé dans les terminaux, où une réponse piégée serait exécutée au collage.
- **Mises à jour** : l'application interroge `github.com/…/releases/latest/download/latest.json` (adresse IP visible de GitHub) ; l'installeur n'est exécuté qu'après vérification de sa signature minisign avec la clé publique embarquée, et de la version couverte par la signature. La clé privée n'existe que chez le propriétaire et dans les secrets GitHub Actions, lus seulement par les builds de tags.
- La **consommation** du LLM (`usage.json`) ne contient que des compteurs de jetons et des montants par jour, aucun texte.
- Dans les fenêtres, le texte dicté est affiché avec `textContent` uniquement : une dictée contenant du HTML ne peut pas s'exécuter.

## Points d'attention Windows

- **AltGr** est émis par Windows comme `Ctrl gauche` (faux, injecté) + `Alt droit`. Le hook doit ignorer ce faux Ctrl.
- **Applis lancées en administrateur** : Windows (UIPI) bloque l'insertion depuis une appli non élevée. Limite connue, pas de contournement prévu.
- Avant le Ctrl+V, AltGr doit être relâché, sinon la frappe devient AltGr+Ctrl+V. C'est le cas : l'insertion a lieu après le relâchement.
- Un appui seul sur Alt active la barre de menus au relâchement ; une frappe neutre (touche virtuelle `0xE8`) est envoyée pendant l'appui pour l'éviter.
- Le presse-papier est restauré 600 ms après le collage, **texte uniquement** : une image copiée avant la dictée est perdue.

## Construction

- Développement sur le serveur Debian (pas de GPU) : `vtt-core` et tout ce qui est portable est testé sous Linux.
- Les binaires Windows sont construits par **GitHub Actions** (`windows-latest`) : installeur NSIS téléchargeable depuis les artefacts ou les releases.
