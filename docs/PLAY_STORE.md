# Publier VoiceToText sur Google Play

Pas à pas pour mettre l'application Android sur le Play Store, puis la mettre à jour automatiquement à chaque version. Les textes à copier-coller sont prêts plus bas.

**Ce qui est publié** : l'édition « bulle » (clavier + bulle de dictée), en **AAB** (`VoiceToText_x.y.z_play.aab`, produit par la CI à chaque tag et joint à la release GitHub). Même identifiant (`io.github.aquadrox.voicetotext`) et même clé que les APK de GitHub : on passe de l'un à l'autre en gardant ses réglages.

## 1. Compte développeur (une fois)

1. <https://play.google.com/console> → **Créer un compte** → **Compte personnel**.
2. Frais d'inscription : **25 $, une seule fois**.
3. Vérification d'identité (pièce d'identité), numéro de téléphone, adresse e-mail de contact (**visible publiquement** sur la fiche). L'adresse postale n'est pas affichée tant que l'application est gratuite et sans achat.

## 2. Créer l'application

Play Console → **Créer une application** :

- Nom : `VoiceToText`
- Langue par défaut : Français (France)
- Application (pas jeu), **Gratuite**
- Accepter les règles et les lois sur l'exportation des États-Unis.

## 3. Clé de signature : garder celle des APK GitHub

À la première publication, Play demande la **clé de signature de l'application**. Choisir **« Utiliser une clé de signature existante d'un keystore Java »** : les versions Play et GitHub se mettront alors à jour l'une l'autre.

1. La Console fournit l'outil `pepk.jar` et une clé de chiffrement `encryption_public_key.pem` : les télécharger.
2. Sur **ton** ordinateur, là où est ton keystore (jamais sur le serveur, jamais envoyé à personne d'autre que Google) :

   ```
   java -jar pepk.jar --keystore=voicetotext.jks --alias=<ton alias> --output=cle-play.zip --include-cert --rsa-aes-encryption --encryption-key-path=encryption_public_key.pem
   ```

3. Envoyer `cle-play.zip` dans la Console. La même clé sert aussi de **clé d'importation** (celle qui signe l'AAB dans la CI) : rien d'autre à faire.

## 4. Test fermé : 12 testeurs pendant 14 jours

Obligatoire pour un compte personnel récent avant de publier pour tout le monde.

1. Console → **Tester → Test fermé** → piste **Alpha** → **Testeurs** : créer une liste d'adresses Gmail (au moins **12**).
2. **Créer une version** → importer l'AAB de la dernière release GitHub (`VoiceToText_x.y.z_play.aab`) → Notes de version : le lien de la release.
3. Envoyer en examen. Une fois validée, partager le **lien d'inscription** aux testeurs : chacun s'inscrit **puis installe** l'application depuis ce lien.
4. Les 12 testeurs doivent rester inscrits **14 jours d'affilée**. Ensuite : **Tableau de bord → Demander l'accès à la production** (quelques questions sur le test).

## 5. Fiche Play Store

Console → **Croissance → Fiche Play Store principale**.

**Nom** (30 caractères max) :

```
VoiceToText : dictée vocale
```

**Description courte** (80 caractères max) :

```
Parlez, le texte s'écrit dans toutes vos applis. Voix transcrite sur l'appareil.
```

**Description complète** :

```
VoiceToText transforme votre voix en texte propre, là où vous écrivez : messages, e-mails, notes, réseaux sociaux.

🎤 UNE BULLE, VOTRE CLAVIER HABITUEL
Gardez votre clavier : une bulle apparaît quand vous écrivez. Touchez-la, parlez, touchez à nouveau : le texte s'insère au curseur. Vous préférez un clavier dédié ? Le clavier VoiceToText est inclus aussi.

🔒 VOTRE VOIX RESTE SUR LE TÉLÉPHONE
La transcription se fait sur l'appareil (Parakeet, Whisper ou Vosk, au choix). L'audio n'est jamais enregistré ni envoyé. Ni compte, ni publicité, ni statistiques.

✨ UN TEXTE PROPRE, SI VOUS LE VOULEZ
Branchez le service d'IA de votre choix (DeepSeek, OpenAI, Mistral, Ollama…) avec votre propre clé : ponctuation, majuscules, « euh » supprimés, corrections à la volée (« le chat, euh non le chien » → « le chien »). Sans IA, tout fonctionne quand même.

🌍 TRADUCTION ET CONSIGNES
• Mode traduction : dictez en français, le texte s'écrit dans la langue choisie.
• Consignes : sélectionnez un texte, maintenez la bulle et dites « rends ce texte plus formel » ou « traduis en anglais ».

⚙️ À VOTRE MANIÈRE
• Dictionnaire (noms propres, jargon) et snippets (« ma signature » → votre signature complète)
• Commandes vocales : « nouvelle ligne », « annule la dernière dictée »
• Historique sur le téléphone, désactivable
• Réglages importables depuis l'application PC VoiceToText (Windows)

🧩 LIBRE ET GRATUIT
Code source ouvert (licence MIT) : https://github.com/Aquadrox/VoiceToText

ACCESSIBILITÉ : la bulle utilise le service d'accessibilité d'Android, uniquement pour s'afficher sur le champ de texte actif et y écrire le texte dicté. Elle ne s'affiche jamais sur un mot de passe et n'enregistre rien. Un avertissement détaillé est présenté dans l'application avant toute activation.
```

**Visuels** (dans `apps/android/play/`, régénérables avec `python3 apps/android/play/visuels.py`) :

- Icône 512 × 512 : `icone-512.png`
- Image de présentation 1024 × 500 : `banniere-1024x500.png`
- Captures d'écran du téléphone (2 à 8, à faire toi-même) : la bulle sur WhatsApp ou Notes, l'écran des réglages, l'historique, le choix du modèle.

**Catégorie** : Productivité. **Coordonnées** : ton e-mail ; site web : `https://github.com/Aquadrox/VoiceToText`.

## 6. Contenu de l'application (Console → Règles → Contenu de l'application)

**Règles de confidentialité** — URL :

```
https://github.com/Aquadrox/VoiceToText/blob/main/docs/PRIVACY.md
```

**Accès à l'application** : toutes les fonctionnalités sont accessibles sans identifiants.

**Annonces** : Non, l'application ne contient pas d'annonces.

**Classification du contenu** : catégorie « Utilitaire, productivité, communication ou autre » ; répondre **Non** à toutes les questions (violence, sexualité, langage, substances, jeux d'argent…). Interactions entre utilisateurs : Non. Partage de position : Non. Achats numériques : Non.

**Public cible** : **18 ans et plus** (évite les règles « Familles » ; l'application n'attire pas spécialement les enfants).

**Application d'actualités / gouvernementale / financière / santé** : Non.

### Sécurité des données

| Question | Réponse |
|---|---|
| L'application collecte-t-elle ou partage-t-elle des types de données obligatoires ? | **Oui** |
| Toutes les données collectées sont-elles chiffrées en transit ? | **Oui** (HTTPS imposé ; seule exception, un serveur que l'utilisateur installe lui-même sur son réseau local) |
| Les utilisateurs peuvent-ils demander la suppression de leurs données ? | **Oui** (historique effaçable dans l'application ; rien n'est gardé ailleurs ; désinstaller supprime tout) |

Types de données à déclarer (proposition : relis-la, c'est toi qui la signes) :

| Type | Collecté | Partagé | Traitement | Obligatoire ? | Finalité |
|---|---|---|---|---|---|
| **Messages → Autre contenu généré par l'utilisateur** (texte transcrit, texte sélectionné pour une consigne) | Oui | Non¹ | **Éphémère** | **Facultatif** (nettoyage LLM désactivable) | Fonctionnalité de l'application |
| Audio → Enregistrements vocaux ou sonores | **Non** (traité sur l'appareil, jamais transmis) | — | — | — | — |

¹ Le texte est envoyé au fournisseur LLM **que l'utilisateur configure lui-même** (adresse et clé saisies par lui) : Google ne compte pas comme « partage » un transfert fait à la demande explicite de l'utilisateur vers un service qu'il a choisi.

Aucune autre donnée (position, contacts, identifiants, historique de navigation, identifiants publicitaires, diagnostics) n'est collectée.

### Service d'accessibilité (déclaration)

Console → Contenu de l'application → **API AccessibilityService** :

- L'application est-elle un outil d'accessibilité (`isAccessibilityTool`) ? **Non.**
- Fonctionnalité principale utilisant l'API — texte proposé :

```
VoiceToText is a voice dictation app. Its accessibility service shows a floating dictation bubble on the focused text field of other apps and inserts the dictated text into that field, so users can dictate anywhere without switching keyboards.

The service only checks whether the focused field is editable and whether it is a password field (the bubble never appears on password fields). It reads the field's text and selection only during a dictation, to insert text at the cursor or to apply a voice instruction to the selected text. Nothing it accesses is stored or logged. The only data leaving the device is text the user explicitly selects for a voice instruction, sent with that instruction to the LLM provider the user configured (optional feature). Voice audio is transcribed on-device and never transmitted.

The service is inactive until the user reads and accepts a prominent in-app disclosure (Settings > "4. Bulle" > "2. Lire l'avertissement et ouvrir l'accessibilité"), then enables the service in Android settings. Declining keeps the bubble disabled; the app's voice keyboard remains fully usable without the accessibility service.
```

- **Vidéo** (obligatoire, 30 s à 2 min, sur YouTube en « non répertoriée ») — scénario :
  1. Ouvrir VoiceToText → section « 4. Bulle » → bouton **2. Lire l'avertissement…** : l'avertissement s'affiche (le laisser lisible 5 s).
  2. **Refuser** : message « Bulle non activée », la bulle n'apparaît pas dans une application de notes.
  3. Revenir, bouton 2 → **Accepter** → écran Accessibilité d'Android → activer « VoiceToText : bulle de dictée ».
  4. Ouvrir une application de notes, toucher un champ : la bulle apparaît ; toucher, dicter une phrase, toucher : le texte s'insère.
  5. Toucher un champ de mot de passe : pas de bulle.

## 7. Mises à jour automatiques (après la première version)

La première version s'envoie **à la main** (étape 4). Ensuite, chaque tag `vX.Y.Z` envoie l'AAB sur Play automatiquement (job `play` de la CI), une fois la release GitHub publiée.

1. **Google Cloud** (<https://console.cloud.google.com>) : créer un projet, activer **Google Play Android Developer API**.
2. **IAM → Comptes de service** → créer un compte (ex. `voicetotext-ci`) → **Clés → Ajouter une clé → JSON** : un fichier `.json` se télécharge. **C'est un secret** : ne l'envoie à personne, ne le mets pas dans le dépôt.
3. **Play Console → Utilisateurs et autorisations → Inviter** l'adresse du compte de service (`…@….iam.gserviceaccount.com`), application VoiceToText, droits **« Publier des versions sur les pistes de test »** (et **« en production »** plus tard).
4. **GitHub → Settings → Secrets and variables → Actions** :
   - **Secret** `PLAY_SERVICE_ACCOUNT_JSON` : coller tout le contenu du fichier `.json` (puis supprimer le fichier de ton ordinateur ou le ranger en lieu sûr) ;
   - **Variables** (onglet *Variables*) :
     - `PLAY_TRACK` : `alpha` pendant le test fermé, puis `production` (ou `beta` pour un test ouvert) ;
     - `PLAY_STATUS` : `completed` (la version part en examen puis en ligne). Mettre `draft` seulement si Play refuse l'envoi tant que l'application n'a jamais été publiée.

Sans le secret, le job `play` ne fait rien (simple note dans les journaux) : rien ne casse.

La clé du compte de service n'est lue que par `.github/scripts/play-upload.sh` (openssl, curl, jq), dans ce seul job, sur les tags : aucune action tierce ne la voit, elle n'est jamais écrite dans un fichier ni affichée.

## 8. Ce qui change pour les utilisateurs

- **Depuis Play** : installation et mises à jour normales, **sans couper Play Protect** et sans « paramètres restreints » pour la bulle.
- **Depuis GitHub** : les APK restent publiés à chaque version (clavier seul, ou clavier + bulle), mêmes réglages, même clé.
