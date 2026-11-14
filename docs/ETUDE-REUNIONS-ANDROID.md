# Étude : le mode Réunions sur Android

Faisabilité du mode Réunions du PC (v0.12, v0.14, v0.20) sur le téléphone, et interface adaptée à un petit écran. Étude du 14/11/2026, sur la base de la version 0.27.1.

## En bref

- **Faisable**, avec une limite imposée par Android : le téléphone ne peut enregistrer que **son propre micro**. Pas de piste « Eux » séparée comme sur le PC, et pas d'enregistrement d'un appel passé *sur le téléphone même* (Teams, WhatsApp, appel classique).
- Le cas d'usage naturel est donc la **réunion en présentiel** (téléphone posé sur la table), le cours, la conférence ; les voix sont séparées par la diarisation (« Intervenant 1, 2… », puis des noms), déjà écrite pour le PC.
- Complément utile : **importer** un enregistrement (mémo vocal, enregistrement d'appel fait par le téléphone lui-même, fichier reçu) via « Partager → VoiceToText ».
- L'essentiel du moteur est déjà partagé : découpage en blocs, transcription (Parakeet, Whisper, Vosk), diarisation, compte rendu LLM, export Markdown. Le travail porte surtout sur l'enregistrement longue durée (service au premier plan) et sur les écrans.

## Ce que le téléphone peut capter

| Situation | Possible ? | Pourquoi |
|---|---|---|
| Réunion en présentiel, cours, conférence | ✅ | Micro seul, comme la dictée, mais longue durée |
| Appel sur un **autre** appareil, téléphone posé à côté du haut-parleur | ✅ (qualité moyenne) | Le micro entend tout le monde ; diarisation pour distinguer |
| Appel Teams / WhatsApp / Meet **sur le téléphone**, piste « Eux » | ❌ | La capture du son des autres applis (`AudioPlaybackCapture`, Android 10+) exclut explicitement le son des appels (`USAGE_VOICE_COMMUNICATION`) |
| Même appel, en se contentant du micro | ❌ (sauf exception à tester) | Pendant un appel, Android donne le micro à l'appli d'appel et fournit du silence aux autres. Un service d'accessibilité au premier plan (édition « bulle ») fait peut-être exception : à essayer, sans garantie |
| Appel téléphonique classique | ❌ en direct | Source `VOICE_CALL` réservée aux applis système depuis Android 9/10. Mais HyperOS (Xiaomi) enregistre lui-même les appels : son fichier peut être **importé** |
| Fichier audio existant (mémo, note vocale WhatsApp, enregistrement d'appel) | ✅ | Décodage par `MediaCodec` d'Android (Opus, AAC, MP3…), sans dépendance |

Conséquence pour l'interface : un seul type d'enregistrement (« Enregistrer ») plus « Importer », pas de choix Réunion / Conférence / sources comme sur le PC. Le prompt de compte rendu (réunion ou conférence) se choisit à l'enregistrement ou après coup.

## Ce qui se réutilise

**Déjà portable et compilé pour Android** (`vtt-core`, `vtt-asr`, `vtt-llm`, tous liés à `vtt-android`) :

- `vtt_core::meeting` : `Chunker` (blocs de 6 à 15 s coupés aux pauses, blocs silencieux ignorés), `Segment`, répliques, horodatage, noms d'intervenants, transcription texte, Markdown, audio par blocs chiffrés (`append_audio` / `read_audio`, chiffrement passé en paramètre).
- `vtt_core::diarize` + `vtt_asr::Diarizer` : analyse au fil de l'eau (`Analysis`), sur le même ONNX Runtime que Parakeet, déjà livré dans l'APK. Seuls les deux modèles (32 Mo) sont à télécharger, comme sur le PC.
- `vtt_llm` : `summarize` (compte rendu, langue au choix), prompts de réunion et de conférence, coût suivi.
- `vtt_core::search` : recherche sans casse ni accents (étape ultérieure).

**À déplacer du PC vers `vtt-core`** (aujourd'hui dans `apps/desktop/src/meetings.rs`, mêlé à Tauri) : le format de `meeting.json` (`MeetingRecord`, `MeetingStatus`, `MeetingKind`, `MeetingSummary`), la liste des réunions et leur purge selon la durée de conservation, l'export Markdown complet. Bénéfice : un seul format, donc une réunion exportée du téléphone se relirait sur le PC. Ce sont des structures et des fonctions pures, testables sous Linux ; `vtt-core` reste sans dépendance OS.

**À écrire pour Android :**

| Morceau | Où | Remarques |
|---|---|---|
| Session de réunion : réception de l'audio, découpe, transcription entre deux dictées, diarisation, fin, compte rendu | `vtt-android` (Rust) | Version simplifiée de la session du PC : une seule piste, pas d'écho à traiter |
| Interface JNI : `meetingStart`, `meetingFeed(FloatArray)`, `meetingStop`, `meetings()`, `meeting(id)`, `summarize`, `rename`, `renameSpeaker`, `delete` | `vtt-android` + `Native.kt` | Même principe que l'existant : échanges en JSON |
| Enregistreur longue durée | `MeetingService.kt` | Service au premier plan de type `microphone` (obligatoire depuis Android 14 avec `targetSdk 36`, permission `FOREGROUND_SERVICE_MICROPHONE`), démarré depuis l'écran ; notification permanente avec chrono et « Arrêter ». Audio transmis au moteur toutes les secondes, rien gardé côté Kotlin |
| Chiffrement de l'audio temporaire | Kotlin (Keystore) → clé passée au moteur | Équivalent de DPAPI ; le stockage privé de l'appli est déjà chiffré par Android, la clé Keystore protège en plus d'une sauvegarde ou d'un accès root |
| Import | `Intent.ACTION_SEND` (audio/\*) + `MediaCodec` | Lu au fil de l'eau, comme sur le PC |
| Écrans | `MeetingsActivity`, `MeetingActivity` | Avec les composants de `Ui.kt`, toujours sans bibliothèque |

## Interface proposée

Principes pour un petit écran :

- **Une action principale par écran**, en bas, à portée du pouce (comme « Enregistrer les réglages » aujourd'hui).
- **Onglets plutôt que colonnes** : là où le PC affiche transcription et compte rendu côte à côte, le téléphone les met dans deux onglets.
- **Actions secondaires dans un menu ⋯** (renommer, intervenants, exporter, supprimer) au lieu d'une rangée de boutons.
- **Partager** (feuille de partage d'Android) remplace « Exporter » et « Copier » : mail, Slack, Drive, Notes… en un geste.
- Écran d'enregistrement **lisible de loin**, posé sur une table : gros chrono, anneau de niveau du micro (le `LevelRing` du clavier), dernières phrases seulement.

Entrée : une carte « Réunions » sur l'écran principal (dernières réunions + « Enregistrer »), et un raccourci sur l'icône de l'appli (appui long → « Enregistrer une réunion »).

### 1. Liste des réunions

```
┌──────────────────────────────────┐
│ ←  Réunions                   🔍 │
├──────────────────────────────────┤
│ Aujourd'hui                      │
│ Point projet Alpha         42:10 │
│ 3 intervenants · compte rendu ✓  │
│──────────────────────────────────│
│ Hier                             │
│ Cours de droit           1:28:03 │
│ Transcription… ▓▓▓▓▓▓░░░░  64 %  │
│──────────────────────────────────│
│ Mémo vocal (importé)        3:12 │
│ Transcription seule              │
│                                  │
├──────────────────────────────────┤
│  [ ● Enregistrer ]  [ Importer ] │
└──────────────────────────────────┘
```

Toucher une ligne ouvre la réunion ; balayer ou appui long → supprimer (confirmation en deux appuis, comme l'historique).

### 2. Enregistrement en cours

```
┌──────────────────────────────────┐
│ ● Enregistrement                 │
│                                  │
│            ╭───────╮             │
│            │ 12:34 │  ← anneau   │
│            ╰───────╯    de niveau│
│                                  │
│ Prévenez les participants que la │
│ réunion est enregistrée.         │
│──────────────────────────────────│
│ 12:02  On part sur le budget de  │
│        mars, donc…               │
│ 12:15  D'accord, je m'occupe du  │
│        devis.                    │
│──────────────────────────────────│
│        [ ■ Arrêter ]             │
└──────────────────────────────────┘
```

- Le rappel de consentement s'efface après une minute.
- L'écran peut s'éteindre : l'enregistrement continue (service au premier plan), la notification affiche `Réunion · 12:34 [Arrêter]`.
- (Remplacé par les maquettes : transcription à la fin, donc pas de texte pendant l’enregistrement ; bouton « Marquer un moment ».)
- Pendant l'enregistrement, le clavier et la bulle affichent « Réunion en cours » au lieu de dicter (le micro est déjà pris).

### 3. Réunion : compte rendu

```
┌──────────────────────────────────┐
│ ←  Point projet Alpha         ⋯  │
│ 14 nov. · 42:10 · 3 intervenants │
│ ┌───────────────┬──────────────┐ │
│ │ Compte rendu  │ Transcription│ │
│ └───────────────┴──────────────┘ │
├──────────────────────────────────┤
│ Résumé                           │
│ Le budget de mars est validé…    │
│                                  │
│ Décisions                        │
│ • Budget de mars validé          │
│                                  │
│ Actions                          │
│ • Paul : devis avant vendredi    │
├──────────────────────────────────┤
│   [ Partager ]   [ Copier ]      │
└──────────────────────────────────┘
```

Sans compte rendu : bouton « Rédiger le compte rendu » (langue et prompt au choix) à la place du texte. Menu ⋯ : Renommer, Intervenants…, Refaire le compte rendu, Exporter en Markdown, Supprimer.

### 4. Réunion : transcription

```
┌──────────────────────────────────┐
│ ←  Point projet Alpha         ⋯  │
│ ┌───────────────┬──────────────┐ │
│ │ Compte rendu  │ Transcription│ │
│ └───────────────┴──────────────┘ │
├──────────────────────────────────┤
│ ● Intervenant 1 · 0:12           │
│ Bonjour à tous, on commence par  │
│ le budget.                       │
│                                  │
│ ● Paul · 0:31                    │
│ Oui, j'ai le chiffre de mars…    │
│                                  │
│ ● Intervenant 1 · 1:05           │
│ Parfait.                         │
└──────────────────────────────────┘
```

Une réplique par bloc, nom en couleur (couleurs du PC). Toucher un nom → le renommer (repris partout) ; toucher le texte → copier.

### 5. Intervenants (fenêtre sur l'écran de la réunion)

```
┌──────────────────────────────────┐
│ Intervenants                     │
│ ● Intervenant 1 · 18 min         │
│   « Bonjour à tous, on commen… » │
│   [ Nom…                       ] │
│ ● Intervenant 2 · 21 min         │
│   « Oui, j'ai le chiffre… »      │
│   [ Paul                       ] │
│                                  │
│ Nombre : [Selon les voix ▾]      │
│              [ Enregistrer ]     │
└──────────────────────────────────┘
```

## Contraintes propres au téléphone

- **Batterie et chaleur** : retenu, transcription à la fin (voir les décisions), le téléphone ne fait qu'enregistrer pendant la réunion. Avec Parakeet sur le Xiaomi 17T Pro, une heure devrait se transcrire en quelques minutes (à mesurer).
- **Téléphone qui tue l'appli** (vu en v0.26.1 sur HyperOS) : un service au premier plan avec notification est beaucoup mieux protégé que la bulle. Avec l'audio « temporaire », reprise après une coupure comme sur le PC ; avec « en mémoire seulement », l'enregistrement est perdu (à dire clairement dans le réglage).
- **Mémoire** : un seul modèle de transcription chargé (règle de v0.18) ; les modèles de diarisation (32 Mo) s'y ajoutent pendant la réunion seulement.
- **Démarrage** : Android interdit de lancer un enregistrement micro depuis l'arrière-plan ; il part toujours d'un geste de l'utilisateur (écran Réunions, raccourci de l'icône, notification). Pas de détection d'appel ni d'enregistrement automatique : de toute façon impossible ici (voir plus haut).
- **Taille de l'APK** : inchangée (ONNX Runtime déjà livré, modèles téléchargés).

## Découpage proposé

Avancement : étapes 1, 2 et 4 livrées en v0.28.0 (voir la roadmap) ; étape 3 (intervenants) à suivre.

1. **Moteur commun** (Rust, testé sous Linux) : format `meeting.json` et liste des réunions déplacés dans `vtt-core` (le PC s'en sert sans changement de comportement) ; session de réunion et JNI dans `vtt-android`. Tests : découpe, transcription simulée, reprise, compte rendu.
2. **Enregistrer et relire** (Android) : service au premier plan, écrans 1 à 4, audio en mémoire ou temporaire chiffré, compte rendu automatique ou à la demande, partage. Captures Robolectric en clair et en sombre.
3. **Intervenants** : téléchargement des modèles de diarisation, analyse pendant la réunion, écran 5, nombre imposé.
4. **Importer** : « Partager → VoiceToText » et bouton Importer, décodage `MediaCodec` au fil de l'eau, avec progression. Essai sur un enregistrement d'appel HyperOS et une note vocale WhatsApp (Opus).
5. **Plus tard** : recherche dans toutes les réunions, questions au LLM, réécoute d'un passage (audio conservé), essai de l'exception « bulle » pendant un appel.

Les étapes 1 et 2 forment une première version utile ; 3 et 4 sont indépendantes l'une de l'autre.

## Décisions du propriétaire (14/11/2026)

1. **Périmètre** : micro seul (réunions en présentiel, cours, import), sans piste « Eux » pour les appels passés sur le téléphone. Accepté.
2. **Audio** : au choix pour chaque réunion (garder sur le téléphone, effacer après la transcription, en mémoire seulement) ; **par défaut, gardé sur le téléphone** (chiffré), ce qui permet la réécoute, la reprise après une coupure et une nouvelle transcription.
3. **Transcription à la fin** de l'enregistrement, pas en direct : pendant la réunion, le téléphone ne fait qu'enregistrer (batterie, chaleur). L'écran d'enregistrement n'affiche donc pas de texte ; après l'arrêt, un écran suit la transcription, les intervenants puis le compte rendu, et une notification prévient quand c'est prêt.
4. **Éditions** : réunions dans les deux APK (clavier et bulle), rien ne demande l'accessibilité.

Ajouté avec les maquettes : **« Marquer un moment »** pendant l'enregistrement (bouton et notification) ; le repère apparaît dans la transcription, pour retrouver un passage important sans texte en direct.

Maquettes visuelles (10 écrans, clair et sombre) : accueil, liste, nouvelle réunion, enregistrement, transcription en cours, compte rendu, transcription, intervenants, notifications. À valider avant de coder.

## À vérifier sur le téléphone

- Enregistrement d'une heure écran éteint, sans être arrêté par HyperOS (service au premier plan).
- Qualité de la transcription et de la diarisation micro posé sur la table, à 3 ou 4 personnes.
- Température et batterie consommée pendant une heure de transcription en direct.
- Pendant un appel WhatsApp sur le téléphone : le micro de VoiceToText reçoit-il du silence (attendu) ? Et depuis l'édition bulle ?
- Format et emplacement des enregistrements d'appel de HyperOS, pour l'import.
