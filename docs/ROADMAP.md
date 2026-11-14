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

## v0.11 — Moteurs légers ✅

- ✅ Choix du moteur de transcription dans Réglages → Micro et transcription, appliqué sans redémarrer
- ✅ Parakeet TDT 0.6B v3 (NVIDIA, int8, ONNX Runtime) : PC sans carte graphique, rapide et ponctué
- ✅ Vosk Small français : très léger, pour les vieux PC
- ✅ Bibliothèques natives (ONNX Runtime, libvosk) téléchargées avec le modèle et vérifiées (SHA-256) : installeur inchangé
- ⬜ Vosk en continu (transcription pendant la dictée, texte prêt au relâchement), si les essais le demandent

## v0.12 — Réunions ✅

Enregistrer un appel (Teams, Zoom, Meet, Discord…) et en tirer la transcription et un compte rendu, sur Windows. Rien d'automatique par défaut, chaque comportement est un réglage.

- ✅ Deux pistes séparées : le micro (« Moi ») et le son du PC (« Eux », capture de toutes les sorties audio actives, casque compris), alignées sur l'horloge (silences du son du PC comblés)
- ✅ Démarrage à la main : menu de l'icône, raccourci (Alt+Maj+M), page Réunions
- ✅ Détection d'appel (v0.12.2) : programme qui utilise le micro (Teams, Zoom, Discord, Slack, WhatsApp, Signal, Telegram, Skype, Webex : 2 s ; navigateurs : 20 s) → petite fenêtre « Appel détecté : Enregistrer / Pas maintenant » ; par service : Proposer, Toujours enregistrer, Ne rien proposer ; jamais d'enregistrement sans réponse
- ✅ Rappel de consentement dans la pastille au démarrage : en France, les participants doivent être prévenus de l'enregistrement
- ✅ Pastille « Réunion 12:34 » pendant l'enregistrement ; « Coller les nouvelles répliques » (Alt+Maj+N : ce qui a été dit depuis la dernière copie)
- ✅ Transcription en direct avec le moteur choisi (Whisper, Parakeet, Vosk), par blocs de 6 à 15 s coupés aux pauses, seulement quand aucune dictée n'est en cours ; blocs silencieux ignorés
- ✅ Suppression de l'écho (v0.12.2) : passages du micro qui suivent le son du PC écartés (enveloppes corrélées, retard 0 à 600 ms), puis répliques « Moi » répétant « Eux » masquées ; rien au casque
- ✅ Audio de la réunion, au choix de l'utilisateur :
  - en mémoire seulement (rien sur le disque ; perdu si l'application plante)
  - fichier temporaire chiffré par Windows (DPAPI), supprimé après la transcription ; reprise automatique après une coupure
  - conservé (chiffré) : export WAV, nouvelle transcription
- ✅ Compte rendu par le LLM choisi (n'importe quel fournisseur) : résumé, décisions, actions (qui fait quoi), questions ouvertes ; prompt modifiable dans la page Prompts ; coût suivi ; automatique ou à la demande
- ✅ Compte rendu dans la langue choisie (v0.12.2), par défaut et pour chaque réunion
- ✅ Correctif v0.12.1 : boutons radio des réglages des réunions alignés
- ✅ Page Réunions : liste, transcription Moi / Eux horodatée (en direct), compte rendu, renommer, copier, exporter en Markdown, supprimer ; durée de conservation réglable

## v0.13 — Capture commentée ✅

Montrer plutôt qu'écrire : parler et dessiner sur l'écran, et obtenir un texte et des images prêts à coller (Slack, Jira, mail, ChatGPT, Claude…).

- ✅ Une touche (Alt+Maj+A) : appui pour démarrer / arrêter, maintien pour dessiner à l'écran avec la souris (traits qui s'effacent à l'écran mais restent sur les captures)
- ✅ Captures d'écran quand l'écran change (pas de vidéo : léger), images retenues aux moments clés : traits dessinés, mots « ici », « là », « ça », « regarde »… ; transcription mot par mot horodatée (Whisper, Parakeet ; Vosk : répartie)
- ✅ Résultat : planche d'images numérotées et annotées + texte mis en forme par le LLM selon un modèle au choix (explication ; rapport de bug : étapes, attendu, obtenu ; consigne pour une IA ; narration seule), collés dans l'application active ; « Coller la dernière capture commentée »
- ✅ Pastille et encre exclues des captures d'écran
- ✅ Captures gardées ou non sur le disque (non par défaut ; 1, 7 ou 30 jours)
- ✅ Option (désactivée par défaut, v0.13.1) : description des images par un LLM qui les lit (fournisseur actif ou profil dédié) ; avertissement : les images quittent le PC ; repli sur le texte seul si le modèle les refuse
- ✅ Fenêtre active ou zone tracée à la souris plutôt que l'écran entier ; clics enregistrés comme moments clés, cerclés sur l'image (v0.13.1)

## v0.14 — Conférences et recherche ✅

Transcrire de longs enregistrements, puis retrouver ce qui a été dit.

- ✅ Enregistrer une conférence (sur place : micro ; en ligne : son du PC ; ou les deux), longue durée, mêmes choix pour l'audio qu'en v0.12
- ✅ Importer un fichier audio ou vidéo existant (MP3, M4A, MP4, MKV, WebM, WAV, FLAC, OGG…) et le transcrire, lu au fil de l'eau (des heures sans tout charger en mémoire), avec progression
- ✅ Au choix : transcription seule, compte rendu seul, ou les deux ; compte rendu de conférence par le LLM choisi (résumé, plan horodaté, idées clés, citations, questions et réponses)
- ✅ Recherche dans toutes les réunions et conférences : sans casse ni accents, résultats horodatés qui ouvrent la transcription au bon passage, surligné
- ✅ Questions au LLM sur un enregistrement ou sur tous (« qu'a-t-on décidé sur le budget ? »), avec les sources citées (titre, heure)
- ✅ Réécouter un passage (v0.14.1) : audio conservé (déchiffré à la volée) ou fichier importé d'origine ; ▶ sur chaque réplique, y compris celle trouvée par la recherche
- ✅ Prompt de compte rendu de conférence modifiable dans la page Prompts (v0.14.1)

## v0.15 — Android : clavier de dictée ✅

- ✅ Application Android (Kotlin, sans dépendance externe) : clavier de dictée (InputMethodService) — maintenir 🎤 pour dicter, appui court pour mains libres, 🌐 / ⌫ / espace / entrée
- ✅ Moteur Rust partagé avec le PC (crate `vtt-android`, JNI) : Whisper sur le téléphone (Small, Base, Large v3 Turbo ; téléchargement vérifié), snippets, commandes vocales, styles par application, nettoyage LLM
- ✅ Clé API chiffrée par le Keystore Android, jamais affichée ; audio en mémoire seulement
- ✅ Import du fichier de réglages exporté depuis le PC (sans les clés)
- ✅ CI : APK de test sur les PR (identifiant `.test`, installable à côté de la version publiée), APK signé dans chaque release ; une PR Android seule ne relance pas le PC, et inversement
- ✅ Essais sur un vrai téléphone (Xiaomi 17T Pro) ; vitesse corrigée (whisper.cpp compilé pour ARMv8.2 + dotprod, encodeur limité à la durée de la dictée, 4 threads)

## v0.16 — Android : confort ✅

- ✅ Historique des dictées sur le téléphone (désactivable, toucher pour copier, effacement en deux appuis)
- ✅ Dictionnaire et snippets modifiables sur le téléphone
- ✅ Mode traduction (🌍 au clavier, langue au choix)
- ✅ Command Mode (maintenir ✨ : consigne appliquée au texte sélectionné)
- ✅ Moteur Parakeet sur le téléphone (ONNX Runtime Android) : fait en v0.18

## v0.17 — Android : bulle de dictée ✅

Dicter sans changer de clavier, comme Wispr Flow : une bulle flottante sur le champ actif.

- ✅ Bulle 🎤 affichée quand un clavier est ouvert sur un champ de texte (ou dès qu'un champ est actif, au choix) ; jamais sur un mot de passe ; historique de la bulle coupable à part (la navigation privée est invisible d'un service d'accessibilité) ; masquée quand le clavier VoiceToText est déjà ouvert
- ✅ Toucher : dicter / insérer ; maintenir : consigne sur la sélection ; glisser : déplacer (position gardée) ; taille au choix
- ✅ Écriture au curseur dans les champs Android ordinaires, collage ailleurs (pages web, éditeurs riches) ou partout au choix
- ✅ Moteur partagé avec le clavier (modèle chargé une seule fois) ; traduction, commandes vocales, snippets, historique identiques
- ✅ Guide d'activation dans l'application (paramètres restreints, accessibilité, batterie Xiaomi)
- ✅ Essai sur le Xiaomi 17T Pro
- ✅ Deux APK par release : « clavier » (s'installe partout) et « bulle » (Play Protect bloque hors Play Store toute appli avec service d'accessibilité : à couper le temps de l'installation)

## v0.18 — Android : modèles du PC ✅

- ✅ Parakeet TDT v3 (ONNX Runtime) et Vosk Small français sur le téléphone, en plus des trois Whisper
- ✅ Bibliothèques natives officielles (ONNX Runtime 1.31.1, Vosk 0.3.75) tirées de leurs paquets Maven, empreintes vérifiées, livrées dans l'APK (Android n'exécute que le code livré avec l'application)
- ✅ Réglages du PC avec Parakeet ou Vosk importés tels quels
- ✅ Un seul modèle en mémoire à la fois (l'ancien est libéré avant de charger le nouveau)
- ✅ Essai sur le Xiaomi 17T Pro : Parakeet le plus rapide et le plus fiable → modèle conseillé et proposé par défaut (v0.18.1)
- ✅ Bulle qui disparaissait (service « Ne fonctionne pas » dans l'accessibilité) : une erreur du moteur (panique Rust, exception Kotlin) ou du micro tuait tout le processus, et Android ne relançait pas le service. Erreurs désormais rattrapées et affichées ; bulle remise si Android retire sa fenêtre ; l'application signale une bulle activée mais arrêtée, avec la raison du dernier arrêt donnée par Android (v0.18.2)
- Suite en v0.26.1 : la bulle s'arrête encore (téléphone qui tue l'application) ; modèle libéré écran éteint, relance guidée

## v0.19 — Réglages clairs ✅

La page Réglages, devenue un long formulaire, est découpée en sections, avec une zone pour essayer chaque changement.

- ✅ Menu des réglages par section : Raccourcis, Reconnaissance locale, Dictée et traduction, Nettoyage par LLM, Clés API et coût, Réunions (réglages déplacés depuis la page Réunions), Capture commentée, Application ; dernière section ouverte retenue ; ancre `#reglages/section`
- ✅ Appliquer section par section, sans redémarrer : champ modifié marqué (barre orange, « modifié »), « N modifications non appliquées » dans l'en-tête, point dans le menu ; **Appliquer** (ou Ctrl+S) n'envoie que la section affichée, les autres gardent leurs saisies en attente ; **Annuler** remet les réglages appliqués
- ✅ Options en interrupteurs avec titre et explication, champs regroupés par thème
- ✅ Zone de test en bas des réglages : dicter dans le champ ; pour chaque dictée (ici ou ailleurs) : ce que le moteur local a entendu, le texte inséré, temps de transcription et modèle, temps du nettoyage, jetons, coût, secours, erreur ; état en direct (écoute, transcription, téléchargement du modèle) et résumé des réglages actifs. Événement `dictation` envoyé à la fenêtre principale seulement, rien d'enregistré
- À vérifier sur le PC : rendu Windows (WebView2), dictée dans le champ de test, changement de modèle suivi dans la zone de test

## v0.20 — Qui parle ? (diarisation) ✅

Distinguer les participants dans la piste « Eux » des réunions et dans les conférences et fichiers importés : « Eux » devient « Intervenant 1 », « Intervenant 2 »…, puis des noms donnés par l'utilisateur. Idée reprise de [pytranscribe](https://github.com/XavierBeheydt/pytranscribe) de Xavier Beheydt (MIT ; WhisperX + pyannote, en Python), réécrite en Rust, sans Python ni PyTorch : crédité dans le README (section Licence), dans l'application (Réglages → Application → Crédits) et dans le code.

- ✅ Essai (PR #47, prototype testé sous Linux) :
  - méthode pyannote 3 : segmentation `segmentation-3.0` (jusqu'à 3 voix par fenêtre de 10 s, chevauchements compris), une empreinte vocale par voix et par fenêtre, regroupement hiérarchique (lien moyen, distance cosinus), reconstruction des tours de parole ; nombre de locuteurs déduit ou imposé
  - modèles ONNX via le même ONNX Runtime que Parakeet, donc aucune nouvelle dépendance : segmentation pyannote 3.0 (MIT, 6 Mo) et empreinte WeSpeaker ResNet34-LM (CC BY 4.0, 26 Mo), au format sherpa-onnx sur Hugging Face, sans compte ni jeton
  - algorithme et fbank (filtres mel façon Kaldi) dans `vtt-core` (`diarize`, `fbank`), testés avec de faux modèles ; fbank identiques à `kaldi-native-fbank` (écart < 10⁻⁴) ; modèles dans `vtt-asr` (`Diarizer`) ; essai : `cargo run --release -p vtt-asr --example diarize_wav`
  - mesuré sur 4 enregistrements de test (2 et 4 voix, anglais et chinois) : frontières identiques à sherpa-onnx (à 0,05 s près), bons locuteurs avec le seuil 0,55, même résultat avec un second modèle (3D-Speaker CAM++) ; 10 min à 4 voix : numéros stables du début à la fin
  - empreintes calculées comme à l'entraînement de WeSpeaker (fenêtre de Hamming, moyenne retirée) : les voix sont bien mieux séparées qu'avec les réglages de sherpa-onnx (écart entre « même voix » et « voix différentes » de 0,63 contre 0,39)
- ✅ Analyse au fil de l'eau (`Analysis`) : l'audio arrive par blocs (réunion, fichier lu peu à peu) et n'est pas gardé, seules restent les voix par fenêtre et leurs empreintes ; mêmes tours de parole que l'analyse d'un seul tenant (testé) ; fenêtres analysées par lots de 8 (les deux modèles alternant fenêtre par fenêtre, leurs threads se gênaient : 1,6 fois plus lent) ; au-delà de 3 000 empreintes (≈ 1 h 30), regroupement sur un échantillon, les autres rejoignent le groupe le plus proche
- ✅ Modèles téléchargés à l'activation du réglage (ou au démarrage s'il est activé sans eux), révisions Hugging Face figées, taille et SHA-256 vérifiés ; ONNX Runtime partagé avec Parakeet ; avancement dans Réglages → Réunions, « Réessayer » en cas d'échec
- ✅ Réglage `meetings.diarize` (désactivé par défaut : téléchargement et analyse des voix sur accord) dans Réglages → Réunions → Intervenants
- ✅ Pendant la réunion, les blocs de la piste « Eux » (conférence : la source captée ; fichier : tout) partent à un thread d'analyse (la moitié des cœurs, la transcription continue) ; à la fin, tours de parole puis compte rendu, qui reprend donc les intervenants ; reprise après une coupure (audio relu bloc par bloc, sans tout charger)
- ✅ Répliques « Eux » réparties entre intervenants à l'affichage, d'après les tours de parole (une réplique peut être coupée : mots étalés sur le temps de parole de chacun) ; transcription d'origine intacte, donc réversible
- ✅ Page Réunions : « Intervenant 1, 2… » en couleur ; bloc Intervenants : nom à donner (temps de parole et première phrase pour le reconnaître), repris dans la transcription, la copie, l'export Markdown, les questions au LLM et le compte rendu ; nombre d'intervenants imposé, recalculé aussitôt ; « Ne plus distinguer » (empreintes effacées)
- ✅ « Distinguer les intervenants » après coup pour un enregistrement fait sans le réglage (audio conservé ou fichier importé toujours présent)
- ✅ Empreintes vocales chiffrées par Windows (`voix.bin`, DPAPI) dans le dossier de la réunion, pour changer le nombre d'intervenants sans l'audio ; jamais écrites pour une réunion « en mémoire seulement » ; effacées avec la réunion
- ✅ Crédits dans Réglages → Application : pytranscribe, pyannote.audio, WeSpeaker (CC BY 4.0), sherpa-onnx, et les autres composants
- Limites : seuil calé sur peu d'exemples (recalé en v0.27.1, sur des enregistrements annotés) ; pas d'intervenants en direct (à la fin de la réunion seulement)
- À vérifier sur le PC : vraies réunions Teams / Meet en français (seuil), vitesse pendant un appel avec Whisper sur la carte graphique, rendu Windows du bloc Intervenants

## v0.21 — Android : mises à jour ✅

Comme sur PC (v0.5) : vérifier, prévenir, installer sur un clic.

- ✅ Manifeste `latest-android.json` publié par la CI à chaque tag, à part de `latest.json` (Tauri exige une signature minisign pour chaque entrée) : une entrée par édition (`android-clavier`, `android-bulle`) avec adresse, taille et SHA-256
- ✅ Vérification à l'ouverture de l'application (si la dernière date de plus d'une heure) puis toutes les 6 h en arrière-plan (JobScheduler, réseau requis, gardée après un redémarrage, replanifiée par le clavier et la bulle) ; désactivable ; notification une fois par version, avec « Installer »
- ✅ Section Mises à jour : version installée et édition, dernière vérification, « Vérifier maintenant », « Télécharger et installer la x.y.z » avec progression, « Voir les nouveautés » ; bandeau en haut de l'application ; notification « mis à jour : version x.y.z » une fois installée (Android ferme l'application sans la rouvrir) et message au lancement suivant
- ✅ Installation sur clic par l'installateur d'Android (PackageInstaller) : autorisation « applis inconnues » demandée la première fois, l'installation reprend au retour ; confirmation d'Android ouverte directement, ou par notification si l'application n'est plus à l'écran ; Android 12 et plus : sans confirmation quand Android l'accepte ; échec expliqué (annulée, bloquée par Play Protect, signature, stockage…)
- ✅ Sécurité : adresses limitées aux Releases du dépôt, HTTPS seulement ; taille et SHA-256 vérifiés ; paquet relu avant l'installation (même identifiant, version annoncée, même édition, même clé de signature) ; Android refuse de toute façon une autre signature ou une version plus ancienne ; pas de seconde clé à gérer (celle des APK, dans les secrets GitHub)
- ✅ La version de test des PR (`.test`, autre signature) ne se met pas à jour
- À vérifier sur le téléphone : passage 0.21.0 → 0.21.1 (clavier et bulle), confirmation ou non d'Android/HyperOS, Play Protect sur l'édition bulle, clavier et bulle toujours activés après la mise à jour

## v0.22 — Notes de version ✅

Chaque mise à jour arrive avec la description de ses changements, consultable aussi après l'installation.

- ✅ `CHANGELOG.md` à la racine : une section `## X.Y.Z — AAAA-MM-JJ` par version, une ligne par changement en mots d'utilisateur, préfixe `[PC]` ou `[Android]` pour une seule plateforme ; versions 0.1.0 à 0.21.0 reconstituées
- ✅ Lecture commune dans `vtt-core` (`changelog`) : tri par plateforme, versions entre l'installée et la proposée (versions sautées comprises), dates en toutes lettres, texte venu du réseau lu sans jamais échouer ; testée sur le vrai fichier
- ✅ CI : `changelog.py check` sur chaque PR (la version du Cargo.toml doit avoir sa section, préfixes et ordre vérifiés) ; à chaque tag, notes en tête de la release GitHub et champ `changelog` dans `latest.json` et `latest-android.json` (`notes` garde l'adresse de la release, lue par la 0.21 Android)
- ✅ PC : nouveautés dans le bandeau de l'accueil et sous **Vérifier maintenant** ; après la mise à jour, notification puis bandeau « mis à jour » sur l'accueil jusqu'à **Fermer** (`version.json`, gardé d'un lancement à l'autre) ; nouveautés de chaque version installée dans Réglages → Application, sous le bouton de mise à jour
- ✅ Android : nouveautés dans le bandeau et la section Mises à jour (lecture par le moteur Rust, `Native.releaseNotes`) ; après la mise à jour, message en haut de l'application avec ce qui a changé ; « Nouveautés de chaque version » dans la section Mises à jour, aussi sur la version de test
- À vérifier : passage 0.22.0 → 0.22.1 sur le PC et le téléphone (notes affichées avant, puis après)

## v0.23 — Identité visuelle ✅

Sortir de l'apparence « générée par défaut » (indigo, émojis, cartes partout) :

- ✅ Logo : barres de voix terminées par un curseur de texte (encre, papier, vermillon) ; sources `apps/desktop/icons/icon.svg` et `icon-small.svg` (version calée sur 16 px pour la zone de notification) ; PNG et `.ico` (16 à 256 px) générés par `node apps/desktop/icons/render-icons.js` ; icône adaptative Android dans la zone sûre
- ✅ Palette tirée du logo : `--accent` (encre) pour les actions, `--signal` (vermillon) pour ce qui est actif ou enregistre ; contrastes vérifiés (texte ≥ 4,5:1) en clair et en sombre
- ✅ Icônes Lucide (licence ISC, `ui/icons-LICENSE.txt`) dans le menu principal et celui des réglages, à la place des émojis ; page ouverte marquée par son icône vermillon
- ✅ Accueil : chiffres sur une seule bande, barre d'aujourd'hui en vermillon, part des mots sur une piste complète ; historique en liste à séparateurs ; titres de groupe sans majuscules forcées ; focus visible au clavier ; police Segoe UI Variable (Windows 11)
- ✅ Pastille : vermillon (écoute, réunion, conférence, capture), vert, orange, bleu (transcription), gris, jaune ; carte « Appel détecté » aux mêmes couleurs
- ✅ Android : icône, clavier (fond d'encre, micro vermillon), bulle (encre au repos avec liseré clair, vermillon à l'écoute)
- À vérifier sur le PC : rendu avec Segoe UI Variable, icône dans la zone de notification et la barre des tâches ; sur le téléphone : icône du lanceur, clavier et bulle

## v0.24 — Android : nouvelle apparence ✅

Même identité que le PC (v0.23), toujours sans bibliothèque :

- ✅ Thème aux couleurs du logo (`res/values` et `values-night`) : encre, papier, vermillon ; barres d'Android assorties ; écrans sans barre d'action, marges des barres gérées (Android 15 dessine l'application dessous)
- ✅ Composants communs (`Ui.kt`, remplace `Screens.kt`) : en-tête (logo ou retour), cartes titrées avec icône, boutons principal et secondaire avec effet d'appui, champs arrondis (bord vermillon au focus), étapes cochées, « En savoir plus », pastilles qui passent à la ligne (`FlowLayout`)
- ✅ Écran principal en cartes : Mise en route (micro, modèle, clavier, bulle, cochés une fois faits, boutons masqués quand c'est fait), modèle, comment dicter, bulle, LLM, traduction, historique et dictionnaire, réglages du PC, mises à jour, essai ; « Enregistrer les réglages » fixé en bas
- ✅ Icônes Lucide converties en VectorDrawable (`res/drawable/ic_*.xml`, licence dans `assets/icons-LICENSE.txt`), vérifiées au pixel contre l'original ; logo en en-tête, logo blanc pour les notifications
- ✅ Clavier : icônes à la place des émojis, touches arrondies avec ondulation, mode traduction allumé en touche claire, touche consigne vermillon pendant l'écoute, disque de niveau autour du micro (`LevelRing`)
- ✅ Bulle : icônes (micro, consigne, attente, coche) au lieu des émojis
- ✅ Historique dans une carte (date et application, puis texte ; toucher pour copier), dictionnaire en pastilles, snippets en liste
- ✅ Vérifié sur le serveur : les deux éditions compilent ; écrans principal, historique, dictionnaire et clavier capturés en clair et en sombre avec Robolectric (vrai code, moteur Rust compilé pour Linux) ; corrigés grâce aux captures : bouton « Ajouter » rogné, police à chasse fixe du champ de clé API, nom du modèle trop long dans « Mise en route »
- À vérifier sur le téléphone (APK de test de la PR) : barres d'Android, clavier et bulle en situation, notifications
- ✅ Clavier sous la barre de navigation (Android 15 et plus étendent aussi la fenêtre du clavier dessous) : Espace et Entrée cachés par les boutons retour, accueil, applications ; marge du bas calculée comme Android pour sa propre barre, barre transparente sur le fond d'encre (v0.24.1). À vérifier sur le téléphone, navigation à trois boutons et par gestes

## v0.25 — Android : de la bulle au clavier ✅

- ✅ Double appui sur la bulle : passage au clavier VoiceToText (`SoftKeyboardController.switchToInputMethod`, Android 11 et plus, clavier activé) ; la bulle s'efface tant qu'il est ouvert. Aucune attente ajoutée au simple appui : la dictée part dès le premier et le second appui l'abandonne
- ✅ Validé sur le téléphone : double appui sur la bulle (clavier VoiceToText activé ou non), retour par la touche clavier

## v0.26 — Android : prompts système ✅

- ✅ Écran « Prompts système » (bouton dans la carte Nettoyage du texte) : prompt intégré en lecture seule, copies modifiables (nom, texte), prompt utilisé au choix, pour le nettoyage et le Command Mode ; mêmes `[[prompts]]` et `llm.cleanup_prompt` / `command_prompt` que la page Prompts du PC, donc les prompts importés du PC y apparaissent ; vérifiés comme sur PC (`clean_prompts`), prompts de compte rendu gardés tels quels, profil LLM pointant vers un prompt supprimé ramené au prompt intégré
- ✅ Champs multi-lignes : `minLines` posé après `minHeight` (Android ne garde que le dernier minimum)
- À vérifier sur le téléphone : édition d'un long prompt au clavier, dictée nettoyée avec la copie

## v0.26.1 — Android : bulle arrêtée par le téléphone ✅

Sur le Xiaomi (HyperOS), la bulle disparaît ; le clavier VoiceToText refonctionne, mais la bulle ne revient qu'après un arrêt forcé et une réactivation. Cause : le téléphone tue l'application et refuse de la relancer ; le service d'accessibilité reste coché mais mort, et Android ne reconnecte jamais seul un service dans cet état, même quand le processus redémarre pour le clavier. Aucune API ne permet à l'application de le reconnecter : seul l'utilisateur le peut (désactiver puis réactiver).

- ✅ Détection de la bulle « cochée mais morte » (`BubbleStatus`) depuis l'application (carte en haut de l'écran) et depuis le clavier (ligne à toucher) ; ouverture directe de la page de la bulle dans l'accessibilité (`ACCESSIBILITY_DETAILS_SETTINGS`, sinon la liste). Quatre secondes de grâce après le lancement du processus, le temps qu'Android reconnecte le service
- ✅ Cause de l'arrêt : premier arrêt du processus depuis la dernière connexion de la bulle (`getHistoricalProcessExitReasons`), gardée dans les réglages et affichée dans la carte Bulle même après un arrêt forcé ou une relance (avant : seul le dernier arrêt, donc souvent l'arrêt forcé lui-même)
- ✅ Modèle libéré une minute après l'extinction de l'écran (`Native.release`), rechargé quand la bulle réapparaît ou à la dictée suivante : une application de plusieurs centaines de Mo est la première visée par le ménage écran verrouillé
- ✅ Appuis sur la bulle et insertion du texte protégés des exceptions (elles tuaient le processus)
- ✅ Conseils constructeurs complétés (verrouillage dans les applications récentes) ; Xiaomi : bouton vers la liste « Démarrage automatique » de l'application Sécurité (`AutoStartManagementActivity`), le réglage n'étant pas dans les informations de l'application sur HyperOS. À vérifier sur le téléphone : le bouton ouvre bien cet écran
- À vérifier sur le téléphone : ligne « Bulle arrêtée » du clavier et carte de l'application quand la bulle est morte ; relance par simple désactivation/réactivation ; cause affichée (à rapporter : elle dira si c'est le ménage de HyperOS, la mémoire ou une mise à jour) ; première dictée après l'écran éteint
- Si cela ne suffit pas : relance automatique possible avec la permission `WRITE_SECURE_SETTINGS`, donnée une fois depuis un PC (`adb`), ou service au premier plan avec notification permanente

## v0.27 — PC : menu principal repliable ✅

Dans les Réglages, deux menus côte à côte (menu principal + sections) prenaient plus de 400 px sur une fenêtre de 980 px.

- ✅ Menu principal replié en colonne d'icônes (63 px au lieu de 190) : libellé en bulle au survol et au focus clavier, toujours lu par les lecteurs d'écran ; point « modifications non appliquées » gardé sur l'icône Réglages ; icônes immobiles d'un état à l'autre (la souris reste sur l'icône cliquée)
- ✅ Replié d'office sur les pages qui ont leur propre menu (`data-sub-nav` : Réglages), déplié ailleurs ; bouton « Réduire le menu » / « Déplier le menu » en bas du menu pour inverser ce choix, retenu par genre de page (`localStorage`, comme la dernière section des réglages) ; repli animé, sauf à l'ouverture et si Windows demande moins d'animations
- ✅ Vérifié sur le serveur (captures Playwright, interface seule) : clair et sombre, fenêtre par défaut (980 × 720) et minimale (720 × 480)
- À vérifier sur le PC : rendu dans WebView2, bulles au survol

## v0.27.1 — PC : intervenants en trop ✅

Retour du propriétaire après une vraie réunion : 2 participants, 5 intervenants trouvés. Reproduit sur le serveur, mesuré, corrigé dans le regroupement des voix (`vtt_core::diarize`).

- ✅ Mesure sur des enregistrements annotés (qui parle quand), analysés comme pendant une réunion (blocs de 6 à 15 s), puis analyses rejouées avec le code réel pour chaque réglage : appels téléphoniques français à 2 voix (corpus Simsamu, 8 kHz, 1 à 7 min) et réunions anglaises à 4 voix (corpus AMI, micros-casques mélangés, 13 à 39 min). Outils de mesure temporaires, non committés
- ✅ Cause : tout groupe de 3 empreintes devenait un locuteur. Or les fenêtres de 10 s avancent de 2 s : quelques secondes de voix « différente » (rire, mots isolés, changement de ton, bruit) donnent à elles seules 3 à 5 empreintes voisines. Les vraies voix étaient bien séparées ; s'y ajoutaient de petits groupes, d'autant plus nombreux que l'enregistrement est long (jusqu'à 61 « intervenants » pour 4 sur 29 min)
- ✅ Empreintes sûres seulement : une voix doit parler seule au moins 2 s dans la fenêtre pour que son empreinte serve au regroupement (règle de pyannote ; avant : 0,5 s) ; les autres rejoignent ensuite le groupe le plus proche
- ✅ Petit groupe proche d'un locuteur = même voix : un groupe de moins de 15 empreintes n'est un locuteur que si son centre est à plus de 0,6 (distance cosinus) du centre de chaque locuteur déjà retenu. Mesuré : faux locuteurs entre 0,3 et 0,6 d'une grande voix, vrais participants discrets entre 0,63 et 0,98
- ✅ Seuil de coupe 0,5 au lieu de 0,55 (deux voix proches au téléphone étaient confondues une fois les empreintes peu sûres écartées)
- ✅ Nombre d'intervenants imposé : les plus grands groupes sont gardés (la coupe descend s'il en manque), au lieu de couper l'arbre en autant de groupes, ce qui isolait d'abord les empreintes atypiques (9 appels à 2 voix sur 26 mal partagés, erreur moyenne 14 %)
- ✅ Format de `voix.bin` inchangé : une réunion déjà enregistrée se recalcule en changeant « Nombre d'intervenants » (un nombre, puis « Selon les voix »)
- ✅ Résultat, réglages choisis sur 26 appels et vérifiés sur 18 autres jamais utilisés pour régler, et sur 6 réunions :
  - 44 appels (43 à 2 voix, 1 à 3) : bon nombre d'intervenants dans 41 cas au lieu de 7 ; trop d'intervenants dans 0 cas au lieu de 37 (jusqu'à 11 pour 2) ; 3 fois un de moins (dont le troisième de l'appel à 3 voix, qui ne parle que 6 s). Sur les 18 appels de vérification seuls : 17 justes au lieu de 2
  - 6 réunions à 4 voix : 3 à 6 intervenants trouvés, au lieu de 9 à 61 ; les intervenants en trop qui restent sont un seul passage de quelques secondes
  - parole attribuée à la mauvaise personne : 2,3 % au lieu de 5,2 % (appels), 3,5 % au lieu de 7,2 % (réunions)
  - nombre imposé (2 ou 4) : partage juste dans tous les enregistrements (2,5 % et 3,2 % de parole mal attribuée)
- Limites : un participant qui ne dit que quelques mots (moins de 3 empreintes sûres) n'est pas distingué ; deux voix très proches au téléphone peuvent encore être confondues (nombre à imposer) ; entre 0,6 et 0,7 de distance, vrais et faux petits locuteurs se recouvrent (réglé pour ne pas perdre de vrais participants)
- À vérifier sur le PC : vraies réunions Teams / Meet en français

## v0.28 — Android : réunions ✅

Le mode Réunions du PC sur le téléphone, adapté au petit écran (étude et maquettes : `ETUDE-REUNIONS-ANDROID.md`). Android ne laisse capter que le micro : réunions en présentiel, cours, conférences, et import d'enregistrements ; pas de piste « Eux » pour un appel passé sur le téléphone.

- ✅ Format commun : fiche `meeting.json` (`MeetingRecord`) déplacée du PC vers `vtt-core` (`meeting::record`), même fichier sur les deux plateformes ; moments marqués (`marks`) et modèle de compte rendu choisi (`conference`) ajoutés, champs facultatifs ignorés par une version précédente. Transcription et Markdown sans nom de piste quand il n'y a qu'une source
- ✅ Moteur (`vtt-android::meetings`, JNI) : création, réception de l'audio au fil de l'eau (16 kHz mono, rééchantillonnage sans décalage cumulé d'un envoi à l'autre, 8 à 48 kHz, mono ou multicanal), moments marqués, arrêt, transcription **à la fin** par blocs de 6 à 15 s coupés aux pauses (moteur de la dictée pris bloc par bloc : les dictées passent entre deux), reprise là où elle s'est arrêtée, compte rendu (prompts réunion et conférence du PC), liste, purge selon `meetings.keep_days`, renommer, supprimer, export Markdown / compte rendu / transcription, extrait audio pour la réécoute ; 7 tests
- ✅ Audio au choix à chaque réunion, **gardé par défaut** : stockage privé de l'application (chiffré par Android), effacé après la transcription, ou en mémoire seulement (16 bits, 115 Mo par heure) ; reprise après un arrêt de l'application (`recover`), audio en mémoire perdu signalé
- ✅ Service au premier plan (`MeetingService`, types micro puis traitement de données) : enregistrement écran éteint, notification avec chrono, « Marquer un moment » et « Arrêter » ; lecture du fichier importé et transcription, avec progression ; notification « Compte rendu prêt » ; modèle gardé en mémoire pendant la transcription ; clavier et bulle refusent de dicter pendant un enregistrement (micro pris)
- ✅ Import : bouton « Importer » (audio ou vidéo) et « Partager → VoiceToText » ; décodage par Android (`MediaExtractor` / `MediaCodec` : MP3, AAC, Opus, AMR, FLAC, vidéos…), sortie 16 bits ou flottante
- ✅ Écrans d'après les maquettes, sans bibliothèque : carte Réunions de l'accueil et raccourci de l'icône, liste groupée par période (progression), feuille « Nouvelle réunion » (titre, type de compte rendu, audio, compte rendu à la fin, rappel de consentement ; refus sans modèle téléchargé), enregistrement (chrono, anneau de niveau, informations, Marquer / Arrêter), suivi après l'arrêt (étapes cochées, premières phrases), réunion (onglets Compte rendu / Transcription, Markdown mis en forme, moments marqués, ▶ réécoute, Partager / Copier, menu ⋯)
- ✅ Intervenants (diarisation du PC, v0.20 et v0.27.1) : interrupteur « Distinguer les intervenants » dans la feuille (piste notée dans la fiche dès la création, donc gardé après une coupure) ; modèles pyannote segmentation 3.0 et WeSpeaker ResNet34-LM (33 Mo, mêmes révisions et empreintes que le PC) téléchargés sur accord, ONNX Runtime de l'APK ; analyse **après la transcription**, audio relu par morceaux de 30 s (moitié des cœurs, progression) ; analyse gardée (`voix.bin`, stockage privé) sauf « en mémoire seulement » ; après coup depuis le menu ⋯ (audio gardé) ; pastilles colorées dans la transcription et fenêtre Intervenants des maquettes (nom, temps de parole, première phrase, ▶ écouter sa voix, nombre imposé recalculé sans l'audio, « Ne plus distinguer ») ; noms repris dans la transcription, le partage et le compte rendu ; 2 tests de plus
- ✅ Vérifié sur le serveur : tests Rust (Linux), les deux éditions compilent, captures Robolectric en clair et en sombre (liste, nouvelle réunion, enregistrement, suivi, compte rendu, transcription, import); captures des intervenants (pastilles, fenêtre, feuille, suivi)
- À vérifier sur le téléphone : une heure écran éteint sans arrêt par HyperOS ; qualité de la transcription micro sur la table ; durée de la transcription d'une heure ; import d'une note vocale WhatsApp (Opus) et d'un enregistrement d'appel HyperOS ; notification et partage
- À vérifier aussi : intervenants avec de vraies voix autour d'une table (seuil réglé sur des appels et des réunions au micro-casque), durée de l'analyse d'une heure
- ⬜ Plus tard, comme sur PC : recherche dans toutes les réunions, questions au LLM sur les enregistrements

## v0.29 — Android : clés API par fournisseur ✅

La gestion des clés du PC (v0.3, v0.6, v0.10) sur le téléphone :

- ✅ Une clé par fournisseur, rangée par hôte de l'adresse (déjà le cas en interne) et chiffrée par le Keystore Android ; la clé d'un fournisseur n'est jamais envoyée à un autre
- ✅ Carte « Clés API » : fournisseurs ayant une clé (seuls les noms sont lus, jamais déchiffrés), usage (« réglage actif », profils importés du PC, « inutilisée ») ; **Remplacer** (champ masqué, jamais prérempli) et **Supprimer** en deux appuis
- ✅ Menu « Fournisseur » (préréglages du PC : DeepSeek, GLM, OpenAI, Mistral, OpenRouter, Ollama du réseau local, Autre) qui remplit adresse et modèle ; le menu suit l'adresse saisie ; état de la clé pour l'adresse en cours de saisie (enregistrée, absente, facultative en local, jamais envoyée en http://)
- ✅ Changement de fournisseur : délai, tarif et `extra_body` suivent le préréglage (plus de `thinking` de DeepSeek envoyé à OpenAI) ; même fournisseur : réglages du PC gardés
- ✅ « Tester ces réglages », comme sur PC : adresse, modèle et clé saisis (sinon la clé enregistrée pour cette adresse), sans rien enregistrer ; nettoyage de la phrase d'exemple du PC et `GET /models` ; réponse, durée, jetons, coût, ou erreur et pistes (`vtt_llm::hints`) ; modèles du serveur en pastilles, un appui remplit le champ (40 au plus) ; le fournisseur testé reçoit les paramètres qu'il aurait une fois enregistré
- ✅ « Tester » sur chaque clé de la liste : adresse et modèle du réglage actif ou du profil qui l'utilisent, sinon du préréglage ; sans modèle connu, seule la liste des modèles est demandée (vérifie la clé)
- ✅ Moteur : `vtt-android::providers` (préréglages, `keyTarget`, `apiKeys`, `testLlm`), 8 tests (dont un faux serveur compatible OpenAI : clé acceptée, refusée, serveur injoignable, clé seule) + 1 complété
- ✅ Vérifié sur le serveur : tests Rust, les deux éditions compilent, captures Robolectric en clair et en sombre, comportement du menu et de l'adresse, test des réglages contre un faux serveur (tests temporaires)
- À vérifier sur le téléphone : enregistrer une clé, passer à un autre fournisseur et revenir, dictée nettoyée avec chacun ; « Tester » avec une vraie clé DeepSeek, puis une clé fausse

## v0.30 — Android : réglages repensés ✅

L'accueil empilait une douzaine de cartes (près de dix écrans à faire défiler), avec un bouton « Enregistrer les réglages » qui ne valait que pour une partie d'entre elles. Refonte d'après les conseils de l'article « 9 astuces pour améliorer l'UX de la page paramètres » (graphiste.com) :

- ✅ Catégories (regrouper) : Dictée, Nettoyage du texte, Dictionnaire et snippets, Bulle de dictée, Historique, Mises à jour, Aide et réglages du PC ; une page chacune (`SettingsPage`, `Section`), ouverte depuis une liste de l'accueil
- ✅ Ordre d'usage (hiérarchiser) : ce qui sert à chaque dictée d'abord ; chaque ligne résume l'état actuel (modèle prêt ou à télécharger, fournisseur et modèle, « clé manquante », nombre de mots, état de la bulle, nouvelle version)
- ✅ Recherche (« Rechercher un réglage ») : index des réglages avec synonymes (`SettingsIndex`), sans accents ni casse, filtré selon l'édition ; le résultat ouvre la page, la fait défiler jusqu'au réglage et l'entoure un instant
- ✅ Clarté : titres courts, explication en gris sous chaque réglage (`toggle`), ce que le réglage fait plutôt que comment
- ✅ Enregistrement confirmé : interrupteurs et choix enregistrés dès qu'on les touche (`Config.save`, mise à jour partielle de config.toml), confirmation qui dit l'effet (« ✓ Nettoyage désactivé : le texte brut sera inséré ») ; seul le formulaire du fournisseur LLM garde « Enregistrer », et demande quoi faire en quittant avec des changements (retour d'Android 13 et plus compris)
- ✅ Section haute : nouvelle version, bulle arrêtée, mise en route seulement tant qu'il manque le micro, le modèle ou un moyen de dicter
- ✅ Réglages avancés repliés : « Adresse et modèle (avancé) » (ouvert d'office pour Ollama ou une adresse libre), prompts système dans « Avancé »
- ✅ Bons contrôles : interrupteur pour une option, boutons radio pour un choix exclusif (modèle, taille de la bulle), menu pour une liste longue (langue, fournisseur) ; toute la ligne d'un interrupteur est touchable, lue en une fois par TalkBack
- ✅ Autres produits : édition bulle présentée dans l'édition clavier (lien vers la release), VoiceToText pour PC en pied d'accueil et dans Aide
- ✅ Vérifié sur le serveur : les deux éditions compilent, captures Robolectric de chaque page (clair et sombre, recherche)
- À vérifier sur le téléphone : retour par geste avec un fournisseur non enregistré, téléchargement d'un modèle en quittant la page Dictée, confirmations

## v0.31 — Réunions : intervenants nommés d'après leur présentation ✅

Quelqu'un se présente pendant la réunion (« Bonjour, je m'appelle Marie ») : « Intervenant 2 » devient « Marie », sur le PC et sur le téléphone. Nécessite « Distinguer les intervenants » ; les noms arrivent avec les intervenants, à la fin.

- ✅ Détection locale par règles dans `vtt-core` (`meeting::names`), sans LLM ni réseau, testée sous Linux : « je m'appelle », « je me nomme », « mon nom / prénom est (c'est) », « moi c'est », « je me présente : », « je suis » + nom à majuscule, « Marie Dupont à l'appareil », « Bonjour, c'est / ici Marie » ; anglais : « my name is », « I'm », « Hi, this is »
- ✅ Prudence : mieux vaut pas de nom qu'un faux nom. Nom à majuscule (sauf après « je m'appelle », « mon nom est » : Vosk écrit en minuscules), jusqu'à trois mots collés, civilité gardée (« Madame Dupont ») mais jamais seule ; pas de sigle (« je suis RH »), ni de mot courant (« je suis désolé », « moi c'est pareil »), ni de nom dans une autre phrase, ni « c'est Paul qui s'en occupe » (on parle de quelqu'un d'autre) ; « Bonjour Paul » ne nomme personne
- ✅ Attribution : présentations groupées par intervenant (formule forte 3, moyenne 2, « je suis » 1), le nom le plus complet gardé (« Marie Dupont » plutôt que « Marie ») ; un nom ne va qu'à un intervenant (le plus net, puis le premier), un intervenant n'a qu'un nom ; jamais le nom qu'un autre a reçu de l'utilisateur
- ✅ Calculé avec les tours de parole (`MeetingRecord::set_speakers`) : fin de réunion, nombre imposé, « Distinguer les intervenants » après coup ; donc repris par le compte rendu rédigé ensuite. Gardé dans `meeting.json` (`speaker_guesses` : nom, moment, phrase)
- ✅ Le nom donné par l'utilisateur passe toujours devant ; le corriger ou l'effacer (retour à « Intervenant N ») oublie la proposition ; donner un nom à une voix retire ce nom des propositions faites aux autres. Renommage et « Ne plus distinguer » mis en commun dans `vtt-core` (PC et Android)
- ✅ PC (bloc Intervenants) et Android (fenêtre Intervenants) : nom pré-rempli, « Présentation à 03:12 : « … » » dessous ; Android : un nom trouvé laissé tel quel reste une proposition
- ✅ 7 tests de plus (formules, phrases ordinaires, citation, attribution, noms donnés, fiche)
- Limites : seulement quand l'intervenant se présente lui-même (pas « je laisse la parole à Paul », ni « merci Marie ») ; pas en direct (les intervenants sont distingués à la fin) ; un nom mal transcrit est repris tel quel ; réunions enregistrées avant : changer le nombre d'intervenants pour relancer la recherche
- À vérifier : vraies réunions en français (tour de table), rendu Windows du bloc Intervenants, fenêtre Android

## v0.32 — Linux et Steam Deck (mode Bureau) ✅

Première version Linux de l'application PC, pensée pour le Steam Deck en mode Bureau (KDE Plasma, session X11) et valable pour tout PC Linux sous X11. Une seule AppImage, rien à installer : SteamOS efface à chaque mise à jour les paquets ajoutés au système.

- ✅ `vtt-platform` pour Linux (session X11), sans bibliothèque système en plus (x11rb, zbus : Rust pur) :
  - touches globales par XInput2 (événements « bruts », reçus quelle que soit la fenêtre, sans droit particulier) ; codes de touche du noyau (evdev) ; nos frappes simulées (périphérique XTEST) ignorées, celles de Steam Input (manette changée en clavier) gardées
  - Ctrl+V / Ctrl+C / Ctrl+Z simulés par XTest, avec la touche qui produit la lettre dans la disposition active (AZERTY, QWERTZ…)
  - presse-papier (arboard) gardé ouvert : sous X11, c'est le programme qui a copié qui fournit le contenu ; ancien contenu remis après le collage
  - Command Mode : sélection copiée, attente du changement de propriétaire du presse-papier (XFixes, équivalent du numéro de séquence de Windows) ; sans sélection, rien ne change
  - fenêtre active : classe de fenêtre (`WM_CLASS`, fiable aussi pour les applications Flatpak du Deck) ou exécutable, titre ; plein écran (`_NET_WM_STATE_FULLSCREEN`) pour libérer la mémoire graphique
  - clés API dans le trousseau de la session (Secret Service : KWallet, GNOME Keyring), une par fournisseur ; liste sans déverrouiller ni lire les clés
  - chiffrement des réunions et des empreintes vocales (remplace DPAPI) : ChaCha20-Poly1305, clé aléatoire gardée dans le trousseau
  - sons de dictée sur la sortie par défaut (cpal), flux fermé dès la fin du son
  - démarrage avec la session : `~/.config/autostart/voicetotext.desktop`, qui lance le fichier AppImage lui-même
- ✅ Styles par application : `slack.exe` vaut aussi pour `Slack` (le `.exe` est ignoré des deux côtés)
- ✅ Whisper sur la carte graphique par Vulkan (AMD du Deck, Mesa, NVIDIA) dans l'AppImage (`--features vulkan`)
- ✅ Corrigé : sans pilote Vulkan (machine virtuelle, NVIDIA sans pilote), whisper.cpp arrêtait tout le programme par une exception C++, même GPU décoché. Une instance Vulkan de test (API C, qui renvoie un code d'erreur) est créée avant : Whisper refusé avec un message clair (« choisir Parakeet ou Vosk »), l'application reste ouverte
- ✅ Corrigé : la pastille plantait au démarrage sous GTK (clics traversants demandés avant l'affichage)
- ✅ Textes de l'interface sans « Windows » codé en dur (trousseau, micro par défaut, démarrage avec la session)
- ✅ CI : job `appimage` (Ubuntu 22.04 pour une glibc ancienne, SDK Vulkan) ; AppImage de test sur les PR, signée sur les tags ; `latest.json` annonce aussi `linux-x86_64` : mise à jour automatique sur Linux, avec la même signature
- ✅ Vérifié sur le serveur (Xvfb, openbox, PulseAudio, GNOME Keyring) : dictée complète touche → micro → Vosk → texte collé dans une fenêtre ; collage avec accents et presse-papier remis ; Command Mode avec et sans sélection ; touches enfoncées ; clés API (enregistrer, remplacer, lister, supprimer) ; chiffrement ; démarrage avec la session ; AppImage construite (91 Mo, chargeur Vulkan inclus) et lancée jusqu'à « prêt » ; Whisper sans pilote Vulkan → message au lieu du plantage
- ⬜ À valider sur le Steam Deck : AppImage, icône dans la zone de notification de KDE, pastille, gâchette arrière (Steam Input → F13), KWallet, vitesse de Whisper Vulkan, mise à jour automatique
- Pas encore sous Linux : session Wayland (le raccourci n'est vu que dans les applications X11), capture commentée, pause de la musique, détection des appels (programmes qui utilisent le micro), mode Jeu

## v0.33 — Linux : confort ⬜

- ⬜ Pause de la musique et des vidéos pendant la dictée (MPRIS, D-Bus)
- ⬜ Capture commentée : capture d'écran, clics, collage d'image (X11)
- ⬜ Détection des appels : programmes qui utilisent le micro (PipeWire / PulseAudio)
- ⬜ Steam Deck : conseils dans l'application (choisir F13, configurer une gâchette arrière dans Steam Input)
- ⬜ Session Wayland : raccourci par le portail « raccourcis globaux » de KDE, collage par le portail « bureau à distance » ou `uinput`

## Outillage ✅

Accélérer la CI sans baisser la garde :

- ✅ Sur les PR, ne vérifier que ce qui est touché (`.github/scripts/changes.sh`) : PR de doc → quelques secondes ; crate modifié → clippy et tests de ce crate et de l'application ; `vtt-core`, dépendances ou CI modifiés → tout
- ✅ Les vérifications obligatoires `linux` et `windows` répondent toujours, même pour la doc (étapes sautées, job réussi). Pas de `[skip ci]` sur une branche de PR
- ✅ Au moindre doute (fichier non classé, historique introuvable), tout est vérifié
- ✅ Suite complète + installeur toujours sur `main` et sur les tags : rien n'est publié sans tout repasser
- ✅ Cache de compilation corrigé : rust-cache recevait un chemin absolu (`C:\t`) qu'il collait derrière le dépôt, et ne sauvegardait donc aucun fichier compilé ; chemin relatif (`D:\t`), cache alimenté par `main` et réutilisé par les PR
- ✅ CLI Tauri précompilée via npm au lieu de `cargo install` (10 min de compilation à chaque cache vide)
- ✅ Un nouveau push sur une PR annule la vérification précédente
- ✅ Mise à jour proposée seulement aux plateformes concernées : à chaque tag, `.github/scripts/platforms.py` liste ce qui change depuis la version que chaque manifeste annonce (doc, CI et simple montée de version ignorées ; `vtt-core`, `vtt-asr`, `vtt-llm`, dépendances ou fichier non classé → les deux). Plateforme non touchée : manifeste précédent repris tel quel (`latest.json` ou `latest-android.json`), l'application ne voit rien à installer ; les installeurs restent publiés pour les nouvelles installations. Notes de la release : plateformes mises à jour ou non
- ✅ Publication par `./release.sh` (Git Bash, sur le PC) : version lue dans le `Cargo.toml` de `main` sur GitHub, refus si déjà publiée, plus ancienne que la dernière ou sans notes de version, nouveautés affichées et confirmation, puis tag `vX.Y.Z` posé sur le dernier commit de `main` et envoyé ; dossier local inchangé, tag local d'un envoi échoué repris ; `.gitattributes` garde les `.sh` en LF sous Windows
- ✅ Actions GitHub sur Node.js 24 : checkout v7, upload-artifact v7, download-artifact v8 (contrôle d'empreinte des artefacts désormais bloquant)

## Plus tard

- Installeur signé (certificat de signature de code) : plus d'avertissement SmartScreen. Reporté tant que l'application n'a pas d'utilisateurs : les certificats sont payants, et SignPath Foundation (gratuit) exige un logiciel déjà diffusé et utilisé. Avertissement non bloquant en attendant (« Informations complémentaires » → « Exécuter quand même »).
- Anglais, nettoyage local. Pas de macOS (pas de Mac pour tester).
- Steam Deck en mode Jeu (gamescope : pas d'icône de notification ni de pastille, collage dans les jeux incertain), à étudier une fois le mode Bureau éprouvé.
- Android : ne garder que la bulle (plus pratique que changer de clavier). À faire une fois la bulle éprouvée ; à régler avant : installation bloquée par Play Protect hors Play Store (service d'accessibilité), navigation privée invisible pour la bulle (le clavier la voit).
