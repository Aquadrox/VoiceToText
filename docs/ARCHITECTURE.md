# Architecture

## Pipeline

```
[Hook clavier] → [Capture micro] → [ASR Whisper] → [Nettoyage LLM] → [Snippets] → [Insertion]
  AltGr down       cpal, 16 kHz mono   local GPU       DeepSeek API                   presse-papier + Ctrl+V
  AltGr up ────────────────────────────────▶ déclenche le traitement
```

Budget de latence visé (relâchement → texte affiché) : **< 1,5 s**, dont ~0,4 s ASR et ~1 s LLM. À mesurer en v0.2 ; si le LLM est trop lent, le nettoyage peut être désactivé ou réservé aux dictées longues.

Si le LLM échoue ou dépasse son délai, le texte brut de Whisper est inséré : une dictée n'est jamais perdue.

## Découpage

```
VoiceToText/
├── crates/
│   ├── vtt-core/      pipeline, traits, configuration — portable, sans dépendance OS
│   ├── vtt-asr/       Whisper via whisper-rs (backend Vulkan)            [v0.1]
│   ├── vtt-llm/       client OpenAI-compatible (DeepSeek), prompts       [v0.2]
│   └── vtt-platform/  hook clavier, capture audio, insertion, appli active [v0.1]
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
| LLM | DeepSeek V4 Flash, API OpenAI-compatible | `base_url` et `model` configurables : changement de fournisseur sans code |
| Clé API | Gestionnaire d'identifiants Windows (crate `keyring`) | Jamais en clair dans le fichier de config |
| Hook clavier | `WH_KEYBOARD_LL` (crate `windows`) | Seule façon de voir l'appui **et** le relâchement d'AltGr ; `RegisterHotKey` ne voit pas le relâchement |
| Insertion | Presse-papier + Ctrl+V, presse-papier restauré ensuite | Instantané même pour un long texte ; `SendInput` Unicode en repli |
| Audio | `cpal` (WASAPI), rééchantillonnage 16 kHz via `rubato` | Format attendu par Whisper |
| Configuration | TOML dans `%APPDATA%\VoiceToText\config.toml` | Lisible et éditable à la main |
| Modèles | Téléchargés au premier lancement dans `%LOCALAPPDATA%\VoiceToText\models` | Installeur léger |

## Confidentialité

- L'audio ne quitte jamais le PC (ASR local).
- Le **texte** transcrit est envoyé à l'API DeepSeek pour le nettoyage. Désactivable dans les réglages (`llm.enabled = false`).

## Points d'attention Windows

- **AltGr** est émis par Windows comme `Ctrl gauche` (faux, injecté) + `Alt droit`. Le hook doit ignorer ce faux Ctrl.
- **Applis lancées en administrateur** : Windows (UIPI) bloque l'insertion depuis une appli non élevée. Limite connue, pas de contournement prévu.
- Avant le Ctrl+V, AltGr doit être relâché, sinon la frappe devient AltGr+Ctrl+V.

## Construction

- Développement sur le serveur Debian (pas de GPU) : `vtt-core` et tout ce qui est portable est testé sous Linux.
- Les binaires Windows sont construits par **GitHub Actions** (`windows-latest`) : installeur NSIS téléchargeable depuis les artefacts ou les releases.
