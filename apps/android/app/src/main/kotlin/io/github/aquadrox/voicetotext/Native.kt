package io.github.aquadrox.voicetotext

/**
 * Moteur Rust (crates/vtt-android) : transcription Whisper sur le téléphone,
 * pipeline commun avec le PC, nettoyage LLM. Échanges en JSON ou config.toml.
 */
object Native {
    init {
        System.loadLibrary("vtt_android")
        init()
    }

    private external fun init()

    /** Réglages affichés : `{"ok": {...}}` ou `{"error": "..."}`. */
    external fun settings(toml: String, modelsDir: String): String

    /** Applique les champs modifiables : `{"ok": "<config.toml>"}` ou erreur. */
    external fun updateSettings(toml: String, update: String): String

    /** Fichier exporté depuis le PC : `{"ok": {"toml", "notes"}}` ou erreur. */
    external fun importSettings(file: String): String

    /** Télécharge un modèle (bloquant) : `{"ok": null}` ou erreur. */
    external fun download(modelsDir: String, id: String): String

    /** Progression du téléchargement en cours (0–100). */
    external fun progress(): Int

    /** Clé API du fournisseur actif (gardée en mémoire seulement). */
    external fun setApiKey(key: String)

    /** Charge le modèle (si besoin) et applique les réglages. */
    external fun start(modelsDir: String, historyFile: String, toml: String): String

    /**
     * Dicte (16 kHz mono) : `{"kind": "text|undo|skip|error", "text", "error", "ms"}`.
     * `mode` : [DICTATE], [TRANSLATE] ou [COMMAND] (consigne appliquée à `selection`).
     */
    external fun dictate(samples: FloatArray, app: String, mode: Int, selection: String): String

    /** Dictées récentes : `{"ok": [...]}`. */
    external fun history(historyFile: String, limit: Int): String

    external fun clearHistory(historyFile: String): String

    /** Dictionnaire et snippets : `{"ok": {"dictionary": [...], "snippets": [...]}}`. */
    external fun vocabulary(toml: String): String

    /** Enregistre dictionnaire et snippets : `{"ok": "<config.toml>"}` ou erreur. */
    external fun saveVocabulary(toml: String, vocabulary: String): String

    const val DICTATE = 0
    const val TRANSLATE = 1
    const val COMMAND = 2
}
