package io.github.aquadrox.voicetotext

import org.json.JSONObject

/**
 * Ce que le champ doit recevoir pendant une dictée insérée au fil de l'eau :
 * un morceau de texte (`text`, séparateur compris) ou l'annulation du
 * morceau précédent (`undo`).
 */
data class LiveAction(val kind: String, val text: String) {
    val isUndo get() = kind == "undo"
}

/** Résultat décodé d'un appel à [Native.liveFeed]. */
data class LiveFeedResult(
    val changed: Boolean,
    val preview: String,
    val segments: Int,
    val actions: List<LiveAction> = emptyList(),
)

/**
 * Morceaux insérés pendant une dictée au fil de l'eau : leurs longueurs,
 * pour annuler le dernier (« annule la dernière dictée » dicté en cours de
 * route) puis, la dictée finie, toute la dictée d'un coup.
 */
class LivePieces {
    private val lengths = ArrayDeque<Int>()

    /** Rien n'a encore été inséré : le premier morceau prend l'espace habituelle. */
    val isEmpty get() = lengths.isEmpty()

    /** Longueur totale insérée par cette dictée. */
    val total get() = lengths.sum()

    fun added(length: Int) {
        if (length > 0) lengths.addLast(length)
    }

    /** Longueur du dernier morceau, à effacer (0 s'il n'y en a pas). */
    fun undoLast(): Int = lengths.removeLastOrNull() ?: 0

    fun clear() = lengths.clear()
}

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

    /** Décode la réponse JSON `{"ok": {"changed", "preview", "segments", "actions"}}` de [Native.liveFeed]. */
    fun parseFeed(raw: String): LiveFeedResult? = runCatching {
        val obj = JSONObject(raw)
        val ok = obj.optJSONObject("ok") ?: return null
        LiveFeedResult(
            changed = ok.optBoolean("changed", false),
            preview = ok.optString("preview", ""),
            segments = ok.optInt("segments", 0),
            actions = parseActions(ok),
        )
    }.getOrNull()

    /** Morceaux à insérer ou annuler (`"actions"` d'un apport ou de la fin d'une dictée). */
    fun parseActions(obj: JSONObject): List<LiveAction> {
        val array = obj.optJSONArray("actions") ?: return emptyList()
        return List(array.length()) { i ->
            val action = array.optJSONObject(i)
            LiveAction(action?.optString("kind").orEmpty(), action?.optString("text").orEmpty())
        }.filter { it.kind == "text" || it.kind == "undo" }
    }

    /**
     * Texte à écrire pour un morceau : le premier d'une dictée prend une
     * espace s'il suit un mot (comme une dictée insérée d'un coup) ; les
     * suivants arrivent avec leur séparateur, mis par le moteur.
     */
    fun spaced(before: String, text: String, first: Boolean): String =
        if (first && before.isNotEmpty() && !before.last().isWhitespace() &&
            text.isNotEmpty() && !text.first().isWhitespace()
        ) {
            " $text"
        } else {
            text
        }

    /**
     * Barre d'état pendant une dictée en maintenant le micro : les derniers
     * mots reconnus dès la première pause (jamais sur un mot de passe).
     */
    fun holdStatus(mode: Int, language: String, privacy: Int, preview: String): String {
        if (canShowPreview(privacy, preview)) return preview.trim()
        return if (mode == Native.TRANSLATE && language.isNotBlank()) "Écoute… → $language" else "Écoute…"
    }
}
