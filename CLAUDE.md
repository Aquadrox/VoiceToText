# VoiceToText — notes pour Claude

- Le propriétaire ne code pas en Rust : Claude écrit tout le code. Expliquer les choix en termes Python/C/.NET si utile.
- Docs, commits et commentaires en français ; identifiants de code en anglais.
- `docs/ROADMAP.md` est le suivi d'avancement : le mettre à jour à chaque étape livrée.
- Dev sur un serveur Debian sans GPU ni Windows : tester sous Linux tout ce qui est portable (`vtt-core`), la CI Windows valide le reste. Garder `vtt-core` sans dépendance OS.
- Avant de livrer : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
