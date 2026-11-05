# Politique de confidentialité — VoiceToText

*Dernière mise à jour : 5 novembre 2026*

VoiceToText est une application de dictée vocale libre et gratuite (licence MIT), développée par un particulier. Elle n'a **ni compte, ni publicité, ni statistiques d'usage, ni serveur** : rien n'est envoyé au développeur. Le code source est public : <https://github.com/Aquadrox/VoiceToText>.

Cette page vaut pour l'application Android (Google Play et APK de GitHub) et pour l'application PC.

## Ce qui reste sur votre appareil

| Donnée | Traitement |
|---|---|
| **Voix (audio du micro)** | Transcrite **sur l'appareil** par un modèle local (Parakeet, Whisper ou Vosk). Gardée en mémoire le temps de la dictée seulement, **jamais enregistrée sur le stockage** ni envoyée. Le micro n'est ouvert que pendant une dictée. |
| **Historique des dictées** | Sur l'appareil seulement, dans l'espace privé de l'application ; désactivable et effaçable. Exclu des sauvegardes Android. |
| **Clé API du fournisseur LLM** | Chiffrée par le Keystore Android (Gestionnaire d'identifiants sur PC) ; jamais affichée, jamais exportée. |
| **Réglages, dictionnaire, snippets** | Sur l'appareil seulement. |

## Service d'accessibilité (bulle de dictée)

L'édition « bulle » utilise le service d'accessibilité d'Android, **uniquement** pour afficher la bulle sur le champ de texte actif et y écrire le texte dicté. Il est désactivé tant que vous n'avez pas lu et accepté l'avertissement dans l'application, puis activé le service dans les réglages d'Android.

- Il regarde seulement si le champ actif accepte du texte et s'il s'agit d'un mot de passe (**jamais de bulle sur un mot de passe**).
- Il ne lit le texte du champ et la sélection **que pendant une dictée**, pour écrire au bon endroit.
- Rien de ce qu'il voit n'est enregistré ni transmis, **sauf** le texte que vous sélectionnez vous-même pour une consigne (appui long sur la bulle), envoyé avec la consigne au fournisseur LLM que vous avez choisi (voir ci-dessous).

## Ce qui peut quitter votre appareil, et vers qui

| Donnée | Destinataire | Quand |
|---|---|---|
| **Texte transcrit** (jamais la voix) | Le fournisseur LLM **que vous configurez** (adresse, modèle et clé saisis par vous), pour ponctuer et nettoyer le texte, le traduire ou appliquer une consigne | Seulement si le nettoyage par LLM est activé. Désactivé : rien ne part. |
| **Texte sélectionné** (consigne ✨ ou appui long) | Ce même fournisseur LLM, avec la consigne | Seulement quand vous dictez une consigne |
| **Téléchargement des modèles** | Hugging Face (Parakeet, Whisper) ou alphacephei.com (Vosk) | Quand vous téléchargez un modèle ; ces sites voient votre adresse IP, comme pour tout téléchargement |
| **Vérification des mises à jour** (PC) | github.com | Au démarrage de l'application PC |

Les échanges avec le fournisseur LLM sont chiffrés (**HTTPS obligatoire**, sauf un serveur que vous installez vous-même sur votre réseau local). Ce fournisseur traite le texte selon **sa propre** politique de confidentialité : choisissez-le en conséquence, ou utilisez un modèle local (Ollama) ou aucun LLM.

## Ce que l'application ne fait pas

- Pas de compte, pas d'identifiant publicitaire, pas de traceur, pas de statistiques d'usage, pas de rapport de plantage.
- Pas de vente ni de partage de données à des fins publicitaires.
- Aucun journal ne contient le texte dicté.

## Vos choix

- Couper le nettoyage par LLM : plus rien ne quitte le téléphone (hors téléchargement des modèles).
- Couper ou effacer l'historique dans l'application.
- Retirer votre accord pour la bulle et désactiver son service d'accessibilité à tout moment.
- Désinstaller l'application supprime toutes ses données.

## Enfants

L'application ne s'adresse pas spécialement aux enfants et ne collecte sciemment aucune donnée les concernant.

## Contact

Questions ou demandes : ouvrir un ticket sur <https://github.com/Aquadrox/VoiceToText/issues>, ou écrire à l'adresse de contact indiquée sur la fiche Google Play.

---

# Privacy policy — VoiceToText (English)

VoiceToText is a free, open-source (MIT) voice dictation app made by an individual. It has **no account, no ads, no analytics and no server**: nothing is sent to the developer.

- **Voice audio** is transcribed **on the device** by a local model, kept in memory only during the dictation, never stored or sent.
- **Dictation history**, settings, dictionary and snippets stay on the device (app-private storage, excluded from backups). History can be turned off and cleared.
- **API key** for the LLM provider is encrypted with the Android Keystore and never displayed or exported.
- **Accessibility service** (bubble edition only, off until you accept an in-app disclosure): used solely to show the dictation bubble on the focused text field and insert the dictated text. It checks whether the focused field is editable and whether it is a password (no bubble on passwords), and reads the field's text and selection only during a dictation. Nothing it sees is stored or transmitted, except text you select yourself for a voice instruction, which is sent with that instruction to the LLM provider you configured.
- **Transcribed text** (never audio) is sent only if LLM cleanup is enabled, to the LLM provider **you configure**, over HTTPS (except a server you run on your own local network). That provider processes it under its own privacy policy.
- **Model downloads** come from Hugging Face or alphacephei.com, which see your IP address.
- No tracking, no advertising ID, no crash reporting, no data sale or sharing for advertising. Logs never contain dictated text.

Contact: <https://github.com/Aquadrox/VoiceToText/issues>.
