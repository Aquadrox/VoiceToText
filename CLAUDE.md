# VoiceToText — notes pour Claude

- Le propriétaire ne code pas en Rust : Claude écrit tout le code. Expliquer les choix en termes Python/C/.NET si utile.
- Docs, commits et commentaires en français ; identifiants de code en anglais.
- `docs/ROADMAP.md` est le suivi d'avancement : le mettre à jour à chaque étape livrée.
- Dev sur un serveur Debian sans GPU ni Windows : tester sous Linux tout ce qui est portable (`vtt-core`), la CI Windows valide le reste. Garder `vtt-core` sans dépendance OS.
- Avant de livrer : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- **Version** : chaque PR qui change l'application monte `version` dans le Cargo.toml racine (l'installeur l'affiche) ; le tag `vX.Y.Z` doit correspondre, la CI refuse sinon.
- CI : les jobs `linux` et `windows` sont obligatoires pour merger ; ne jamais mettre `[skip ci]` sur une branche de PR (la PR resterait bloquée). Les PR de doc passent en quelques secondes grâce à `.github/scripts/changes.sh`.
- **Signature des mises à jour** : la clé privée minisign appartient au propriétaire (secrets GitHub `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD`, lus seulement par les builds de tags). Ne jamais la générer, la demander ni la stocker ici ; seule la clé publique est dans `tauri.conf.json`.
- **Dépôt public** : ne jamais committer de secret (clé API, jeton, mot de passe, clé de signature, fichier `.env`), ni dans le code, ni dans les tests, ni dans les journaux collés, ni dans les PR. Clés de test fictives uniquement. Les secrets de la CI passent par les secrets GitHub Actions. Vérifier le diff avant chaque push.
