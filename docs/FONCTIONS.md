# Fonctions

Référence : Wispr Flow. Objectif : dicter n'importe où, texte propre inséré dans l'application active.

## Cœur (v0.1 – v0.2)

| Fonction | Comportement | Version |
|---|---|---|
| Push-to-talk | Maintenir **AltGr** (configurable) → enregistrement ; relâcher → transcription + insertion | v0.1 |
| Transcription locale | Whisper large-v3-turbo sur GPU (RTX 4070 Ti), français forcé | v0.1 |
| Insertion universelle | Texte collé dans le champ actif de n'importe quelle application | v0.1 |
| Indicateur d'enregistrement | Petite pastille flottante (écoute / traitement / erreur) | v0.1 |
| Icône zone de notification | Menu : réglages, historique, pause, quitter | v0.1 |
| Nettoyage LLM | Suppression des « euh », auto-corrections orales, ponctuation, listes | v0.2 |
| Dictionnaire perso | Mots/noms propres injectés comme indices à Whisper et au LLM | v0.2 |
| Snippets | Phrase déclencheuse → bloc de texte (signature, adresse…) | v0.2 |
| Historique | Dernières dictées (brut + final), copie en un clic | v0.2 |

## Confort (v0.3)

| Fonction | Comportement |
|---|---|
| Mode mains libres | Double appui sur le raccourci → enregistrement verrouillé jusqu'au prochain appui |
| Styles par application | Ton adapté selon l'appli au premier plan (Outlook formel, Discord décontracté…) |
| Command Mode | Sélectionner un texte, maintenir un raccourci, dicter une consigne (« plus formel », « traduis ») → texte remplacé |
| Annuler | Raccourci pour coller la version brute à la place de la version nettoyée |

## Plus tard

- Anglais / détection automatique de langue.
- Linux (X11 puis Wayland).
- Clavier Android.
- Contexte d'écran (lecture des noms visibles pour l'orthographe).
- Nettoyage 100 % local (petit LLM sur le GPU) pour un mode hors-ligne.

## Contraintes AltGr (AZERTY)

Sur un clavier AZERTY, AltGr sert à taper `@ # { [ | € ~`. Le raccourci ne doit pas casser ça :

- L'enregistrement démarre à l'appui sur AltGr, **sans** bloquer la touche.
- Si une autre touche est pressée pendant qu'AltGr est maintenu → c'est une combinaison de caractère : enregistrement annulé, rien n'est envoyé.
- Appui trop court (< `min_hold_ms`, 250 ms par défaut) → ignoré.
