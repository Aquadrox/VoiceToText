package io.github.aquadrox.voicetotext

import org.json.JSONObject

/** Résultat décodé d'un appel à [Native.liveFeed]. */
data class LiveFeedResult(
    val changed: Boolean,
    val preview: String,
    val segments: Int,
)

/**
 * Logique pure de prévisualisation en continu pour la dictée mains libres sur
 * Android (sans dépendance SDK, testée directement sur la JVM).
 */
object LiveLogic {
    /** Intervalle de prélèvement incrémental du micro en mode mains libres (ms). */
    const val LIVE_DRAIN_MS = 250L

    /**
     * Vrai uniquement lorsque la session mains libres en continu est autorisée :
     * jamais en champ de mot de passe (pour ne jamais afficher le texte en clair)
     * et jamais en Command Mode.
     */
    fun canUseLive(privacy: Int, mode: Int): Boolean =
        privacy != Native.PASSWORD && mode != Native.COMMAND

    /**
     * Vrai si un aperçu non vide peut être affiché dans la barre d'état du
     * clavier : toujours faux sur un champ de mot de passe.
     */
    fun canShowPreview(privacy: Int, preview: String): Boolean =
        privacy != Native.PASSWORD && preview.isNotBlank()

    /**
     * Texte affiché dans la barre d'état du clavier en mode mains libres :
     * l'aperçu glissant dès qu'une première tranche est prête (jamais sur un
     * mot de passe), sinon l'indication d'arrêt en un appui.
     */
    fun handsFreeStatus(mode: Int, language: String, privacy: Int, preview: String): String {
        if (canShowPreview(privacy, preview)) {
            return preview.trim()
        }
        return if (mode == Native.TRANSLATE && language.isNotBlank()) {
            "Mains libres → $language : appuyez pour terminer"
        } else {
            "Mains libres : appuyez pour terminer"
        }
    }

    /** Décode la réponse JSON `{"ok": <sessionId>}` de [Native.liveStart] (null si erreur). */
    fun parseStartSession(raw: String): Long? = runCatching {
        val obj = JSONObject(raw)
        if (obj.has("error") && !obj.isNull("error") && obj.optString("error").isNotEmpty()) {
            return null
        }
        val id = obj.optLong("ok", 0L)
        if (id > 0L) id else null
    }.getOrNull()

    /** Extrait le message d'erreur éventuel d'une réponse `{"error": "..."}`. */
    fun parseError(raw: String): String? = runCatching {
        JSONObject(raw).optString("error").takeIf { it.isNotEmpty() && it != "null" }
    }.getOrNull()

    /** Décode la réponse JSON `{"ok": {"changed", "preview", "segments"}}` de [Native.liveFeed]. */
    fun parseFeed(raw: String): LiveFeedResult? = runCatching {
        val obj = JSONObject(raw)
        val ok = obj.optJSONObject("ok") ?: return null
        LiveFeedResult(
            changed = ok.optBoolean("changed", false),
            preview = ok.optString("preview", ""),
            segments = ok.optInt("segments", 0),
        )
    }.getOrNull()
}
