package io.github.aquadrox.voicetotext

/**
 * Découpage borné et règles de confidentialité pour la lecture du contexte
 * autour du curseur sur Android (clavier et bulle).
 *
 * Logique pure (sans dépendance Android) afin d'être testée directement sur JVM.
 */
object ContextLogic {
    /** Caractères lus avant le curseur dans un champ (InputConnection / accessibilité). */
    const val BEFORE_CURSOR_CHARS = 1_500

    /** Caractères lus après le curseur. */
    const val AFTER_CURSOR_CHARS = 1_000

    /** Plafond absolu envoyé au moteur Rust (qui borne aussi à 4 000 caractères). */
    const val MAX_CONTEXT_CHARS = 4_000

    /**
     * Vrai uniquement lorsque l'option opt-in est activée, que le champ est
     * ordinaire (jamais mot de passe ni navigation privée / sans historique) et
     * hors Command Mode.
     */
    fun canReadContext(enabled: Boolean, privacy: Int, mode: Int): Boolean =
        enabled && privacy == Native.NORMAL && mode != Native.COMMAND

    /**
     * Assemble le texte lu avant et après le curseur (`InputConnection`), borné
     * à [MAX_CONTEXT_CHARS]. Renvoie une chaîne vide s'il n'y a rien d'utile.
     */
    fun aroundCursor(before: CharSequence?, after: CharSequence?): String {
        val left = before?.toString().orEmpty()
        val right = after?.toString().orEmpty()
        if (left.isBlank() && right.isBlank()) return ""
        val combined = if (left.isNotEmpty() && right.isNotEmpty()) {
            val sep = if (left.last().isWhitespace() || right.first().isWhitespace()) "" else " "
            left + sep + right
        } else {
            left + right
        }
        return if (combined.length <= MAX_CONTEXT_CHARS) {
            combined
        } else {
            combined.substring(0, MAX_CONTEXT_CHARS)
        }
    }

    /**
     * Extrait une fenêtre bornée autour de la sélection / du curseur dans le
     * texte complet d'un nœud d'accessibilité (bulle).
     */
    fun windowAroundSelection(text: String, selectionStart: Int, selectionEnd: Int): String {
        if (text.isBlank()) return ""
        val len = text.length
        var start = if (selectionStart < 0 || selectionEnd < 0) len else selectionStart
        var end = if (selectionStart < 0 || selectionEnd < 0) len else selectionEnd
        if (start > end) {
            val tmp = start
            start = end
            end = tmp
        }
        start = start.coerceIn(0, len)
        end = end.coerceIn(0, len)
        val from = (start - BEFORE_CURSOR_CHARS).coerceAtLeast(0)
        val to = (end + AFTER_CURSOR_CHARS).coerceAtMost(len)
        val slice = text.substring(from, to)
        if (slice.isBlank()) return ""
        return if (slice.length <= MAX_CONTEXT_CHARS) {
            slice
        } else {
            slice.substring(0, MAX_CONTEXT_CHARS)
        }
    }

    /**
     * Découpe une saisie d'applications exclues (séparées par virgules,
     * points-virgules ou retours à la ligne), nettoie et déduplique sans casse.
     */
    fun parseExcludedApps(raw: String): List<String> {
        val seen = LinkedHashSet<String>()
        val out = mutableListOf<String>()
        for (part in raw.split(',', ';', '\n', '\r')) {
            val cleaned = part.trim()
            if (cleaned.isEmpty()) continue
            val key = cleaned.lowercase()
            if (seen.add(key)) out += cleaned
        }
        return out
    }

    /** Formate la liste des applications exclues pour l'affichage dans le champ texte. */
    fun formatExcludedApps(apps: List<String>): String = apps.joinToString(", ")
}
