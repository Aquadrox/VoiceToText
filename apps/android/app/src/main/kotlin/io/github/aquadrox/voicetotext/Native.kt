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

    /**
     * Où ranger la clé de l'adresse saisie (pas encore enregistrée) :
     * `{"ok": {"host", "provider", "optional", "cleartext"}}`.
     */
    external fun keyTarget(baseUrl: String): String

    /**
     * Clés enregistrées (`hosts` : tableau JSON des hôtes) avec leur usage :
     * `{"ok": [{"host", "provider", "used_by": [...]}]}`. Jamais la clé elle-même.
     */
    external fun apiKeys(toml: String, hosts: String): String

    /**
     * Teste des réglages saisis, sans les enregistrer (bloquant : réseau) :
     * `request` = `{"base_url", "model", "key"}` (modèle vide : clé seule).
     * `{"ok": {"ok", "endpoint", "host", "model", "key", "duration_ms", "input",
     * "output", "error", "usage", "cost", "currency", "models", "models_error", "hints"}}`.
     */
    external fun testLlm(toml: String, request: String): String

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

    // Réunions (dossier `root`) : réponses `{"ok": …}` ou `{"error": "…"}`.

    /**
     * Nouvelle réunion : `{"ok": "<id>"}`. `audio` : « conserve », « temporaire »
     * ou « memoire » ; `source` : nom du fichier importé, vide pour un enregistrement.
     */
    external fun meetingCreate(
        root: String,
        title: String,
        conference: Boolean,
        audio: String,
        summary: Boolean,
        source: String,
        speakers: Boolean,
    ): String

    /** Audio reçu (PCM 16 bits entrelacé, `count` valeurs) : `{"ok": <durée en ms>}`. */
    external fun meetingFeed(root: String, id: String, pcm: ShortArray, count: Int, rate: Int, channels: Int): String

    /** Lecture d'un fichier importé (pourcentage). */
    external fun meetingProgress(root: String, id: String, percent: Int): String

    /** Moment marqué : `{"ok": <position en ms>}`. */
    external fun meetingMark(root: String, id: String): String

    /** Fin de l'enregistrement ou de la lecture du fichier. */
    external fun meetingStop(root: String, id: String): String

    /** Après un arrêt de l'application : `{"ok": [ids à transcrire]}`. */
    external fun meetingRecover(root: String): String

    /** Transcription, intervenants si demandés, puis compte rendu (bloquant, des minutes). */
    external fun meetingProcess(root: String, id: String, toml: String, modelsDir: String): String

    /** Distingue les intervenants après coup (audio gardé ; bloquant). */
    external fun meetingIdentify(root: String, id: String, modelsDir: String): String

    /** Nombre d'intervenants imposé (0 : selon les voix), recalculé aussitôt. */
    external fun meetingSpeakerCount(root: String, id: String, count: Int): String

    /** Nom d'un intervenant (vide : « Intervenant N »). */
    external fun meetingRenameSpeaker(root: String, id: String, index: Int, name: String): String

    /** Ne plus distinguer : tours de parole, noms et empreintes effacés. */
    external fun meetingForgetSpeakers(root: String, id: String): String

    /** Modèles des intervenants (bloquant) : `{"ok": null}` ou erreur. */
    external fun downloadVoices(modelsDir: String): String

    /** Progression de ce téléchargement (0–100). */
    external fun voicesProgress(): Int

    /** (Re)fait le compte rendu (bloquant). `language` vide : celle des réglages. */
    external fun meetingSummarize(root: String, id: String, toml: String, language: String): String

    /** Liste, la plus récente d'abord : `{"ok": [{"id", "title", "status", …}]}`. */
    external fun meetings(root: String, toml: String): String

    /** Réunion ouverte : répliques, compte rendu, moments marqués… */
    external fun meeting(root: String, id: String): String

    external fun meetingRename(root: String, id: String, title: String): String

    /** Modèle de compte rendu : conférence (cours, exposé) ou réunion. */
    external fun meetingSetConference(root: String, id: String, conference: Boolean): String

    external fun meetingDelete(root: String, id: String): String

    /** Texte à partager : `what` « markdown », « summary » ou « transcript ». */
    external fun meetingExport(root: String, id: String, what: String, date: String): String

    /** Passage de l'audio conservé (PCM 16 bits 16 kHz mono), vide si indisponible. */
    external fun meetingClip(root: String, id: String, startMs: Long, endMs: Long): ShortArray?

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
