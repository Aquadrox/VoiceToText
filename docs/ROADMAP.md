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
- À vérifier sur le téléphone : si la bulle s'arrête encore, la raison affichée (mémoire insuffisante, arrêtée par le système…) dira s'il faut libérer le modèle écran éteint

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
- Limites : seuil (0,55) calé sur peu d'exemples ; pas d'intervenants en direct (à la fin de la réunion seulement)
- À vérifier sur le PC : vraies réunions Teams / Meet en français (seuil), vitesse pendant un appel avec Whisper sur la carte graphique, rendu Windows du bloc Intervenants

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
- Anglais, Linux, nettoyage local, réunions sur le téléphone. Pas de macOS (pas de Mac pour tester).
- Android : ne garder que la bulle (plus pratique que changer de clavier). À faire une fois la bulle éprouvée ; à régler avant : installation bloquée par Play Protect hors Play Store (service d'accessibilité), navigation privée invisible pour la bulle (le clavier la voit).
