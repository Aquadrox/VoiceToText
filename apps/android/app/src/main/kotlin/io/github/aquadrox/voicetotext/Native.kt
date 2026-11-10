package io.github.aquadrox.voicetotext

import android.text.InputType

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

    /** Libère le modèle de la mémoire ; le prochain [start] le recharge. */
    external fun release()

    /**
     * Dicte (16 kHz mono) : `{"kind": "text|undo|skip|error", "text", "error", "ms"}`.
     * `mode` : [DICTATE], [TRANSLATE] ou [COMMAND] (consigne appliquée à `selection`).
     */
    external fun dictate(samples: FloatArray, app: String, mode: Int, selection: String, privacy: Int): String

    /** Dictées récentes : `{"ok": [...]}`. */
    external fun history(historyFile: String, limit: Int): String

    external fun clearHistory(historyFile: String): String

    /** Dictionnaire et snippets : `{"ok": {"dictionary": [...], "snippets": [...]}}`. */
    external fun vocabulary(toml: String): String

    /** Enregistre dictionnaire et snippets : `{"ok": "<config.toml>"}` ou erreur. */
    external fun saveVocabulary(toml: String, vocabulary: String): String

    /**
     * Prompts système : `{"ok": {"cleanup": {...}, "command": {...}}}`, chacun
     * `{"active", "presets": [{"name", "text"}], "builtin"}` (`active` vide : intégré).
     */
    external fun prompts(toml: String): String

    /** Enregistre les prompts (même forme, sans `builtin`) : `{"ok": "<config.toml>"}` ou erreur. */
    external fun savePrompts(toml: String, prompts: String): String

    /**
     * Notes de version pour le téléphone : `{"ok": [{"version", "date"?, "items": [...]}]}`,
     * la plus récente d'abord. `changelog` vide : notes embarquées dans l'application
     * (CHANGELOG.md) ; sinon champ `changelog` du manifeste. Versions après `after`
     * (exclue) et jusqu'à `upTo` (incluse) ; borne vide : sans limite.
     */
    external fun releaseNotes(changelog: String, after: String, upTo: String): String

    const val DICTATE = 0
    const val TRANSLATE = 1
    const val COMMAND = 2

    /** Champ ordinaire. */
    const val NORMAL = 0
    /** Navigation privée, appli sensible : rien dans l'historique. */
    const val NO_HISTORY = 1
    /** Mot de passe : transcription seule, ni LLM ni historique. */
    const val PASSWORD = 2

    /** Type de saisie d'un mot de passe (texte, web, visible ou code chiffré). */
    fun isPassword(inputType: Int): Boolean {
        val variation = inputType and InputType.TYPE_MASK_VARIATION
        return when (inputType and InputType.TYPE_MASK_CLASS) {
            InputType.TYPE_CLASS_TEXT -> variation in listOf(
                InputType.TYPE_TEXT_VARIATION_PASSWORD,
                InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
                InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD,
            )
            InputType.TYPE_CLASS_NUMBER -> variation == InputType.TYPE_NUMBER_VARIATION_PASSWORD
            else -> false
        }
    }
}
