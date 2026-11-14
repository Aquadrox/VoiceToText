package io.github.aquadrox.voicetotext

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.StyleSpan
import android.widget.LinearLayout
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.text.DateFormat
import java.util.Calendar
import java.util.Date

/** Outils communs aux écrans des réunions. */
object Meetings {
    /** `12:34`, ou `1:02:03` au-delà d'une heure. */
    fun clock(ms: Long): String {
        val seconds = ms / 1000
        val (h, m, s) = Triple(seconds / 3600, seconds / 60 % 60, seconds % 60)
        return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%02d:%02d".format(m, s)
    }

    /** « Aujourd'hui, 10:32 », « Hier, 9:05 » ou la date courte. */
    fun date(ms: Long): String {
        val day = Calendar.getInstance().apply { timeInMillis = ms }
        val today = Calendar.getInstance()
        val time = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(ms))
        val sameDay = { a: Calendar, b: Calendar ->
            a.get(Calendar.YEAR) == b.get(Calendar.YEAR) && a.get(Calendar.DAY_OF_YEAR) == b.get(Calendar.DAY_OF_YEAR)
        }
        if (sameDay(day, today)) return "Aujourd'hui, $time"
        if (sameDay(day, (today.clone() as Calendar).apply { add(Calendar.DAY_OF_YEAR, -1) })) return "Hier, $time"
        return DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(ms))
    }

    /** Groupe d'une réunion dans la liste. */
    fun group(ms: Long): String {
        val day = Calendar.getInstance().apply { timeInMillis = ms }
        val now = Calendar.getInstance()
        val days = ((startOfDay(now) - startOfDay(day)) / (24 * 3600 * 1000L)).toInt()
        return when {
            days <= 0 -> "Aujourd'hui"
            days == 1 -> "Hier"
            days < 7 -> "Cette semaine"
            day.get(Calendar.YEAR) == now.get(Calendar.YEAR) ->
                day.getDisplayName(Calendar.MONTH, Calendar.LONG, java.util.Locale.FRANCE)
                    .orEmpty().replaceFirstChar { it.uppercase() }
            else -> day.get(Calendar.YEAR).toString()
        }
    }

    private fun startOfDay(day: Calendar): Long = (day.clone() as Calendar).apply {
        set(Calendar.HOUR_OF_DAY, 0)
        set(Calendar.MINUTE, 0)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    /** Ce qu'on sait d'une réunion, en une ligne (liste, en-tête). */
    fun meta(meeting: JSONObject): String = buildList {
        val speakers = meeting.optInt("speakers")
        if (meeting.optBoolean("imported")) add("importée")
        add(if (meeting.optBoolean("conference")) "cours ou conférence" else "réunion")
        if (speakers > 0) add("$speakers intervenant(s)")
        when (meeting.optString("status")) {
            "enregistrement" -> add(if (meeting.optBoolean("imported")) "lecture du fichier" else "enregistrement en cours")
            "transcription" -> add(
                when (meeting.optString("step")) {
                    "transcription" -> "transcription ${meeting.optInt("progress")} %"
                    "speakers" -> "intervenants ${meeting.optInt("progress")} %"
                    "summary" -> "compte rendu en cours"
                    else -> "en attente de transcription"
                },
            )
            "interrompue" -> add("interrompue")
            else -> when {
                meeting.optString("step") == "summary" -> add("compte rendu en cours")
                meeting.optString("step") == "speakers" -> add("intervenants ${meeting.optInt("progress")} %")
                meeting.optBoolean("summary") -> add("compte rendu")
                else -> add("transcription seule")
            }
        }
    }.joinToString(" · ")

    /** Coût LLM, comme sur PC : chiffres significatifs pour les montants minuscules. */
    fun cost(value: Double, currency: String): String {
        val tiny = value > 0 && value < 0.01
        val shown = if (tiny) value.toBigDecimal().round(java.math.MathContext(2)).toDouble() else value
        return runCatching {
            java.text.NumberFormat.getCurrencyInstance(java.util.Locale.FRANCE).apply {
                this.currency = java.util.Currency.getInstance(currency)
                maximumFractionDigits = if (tiny) 8 else 2
                minimumFractionDigits = 2
            }.format(shown)
        }.getOrElse { "%.4f %s".format(shown, currency) }
    }

    /** Champ texte JSON absent ou nul : vide. */
    fun JSONObject.text(name: String): String = optString(name).takeIf { it != "null" }.orEmpty()

    fun JSONArray.objects(): List<JSONObject> = List(length()) { getJSONObject(it) }

    fun copy(context: Context, label: String, text: String) {
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText(label, text))
        Toast.makeText(context, "Copié", Toast.LENGTH_SHORT).show()
    }

    /** Téléchargement des modèles des intervenants, partagé par les écrans. */
    object Voices {
        @Volatile
        var downloading = false
            private set

        /** Lance le téléchargement (rien s'il tourne déjà) ; `done` (fil principal) : erreur ou null. */
        fun download(context: Context, store: Store, done: (String?) -> Unit) {
            if (downloading) return
            downloading = true
            val main = android.os.Handler(android.os.Looper.getMainLooper())
            val dir = store.modelsDir
            Thread {
                val error = runCatching { JSONObject(Native.downloadVoices(dir)).optString("error") }
                    .getOrElse { it.message ?: "erreur" }
                downloading = false
                main.post { done(error.ifEmpty { null }) }
            }.start()
        }
    }

    /** Feuille de partage d'Android (mail, messagerie, Drive, Notes…). */
    fun share(context: Context, subject: String, text: String) {
        val send = Intent(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, subject)
            .putExtra(Intent.EXTRA_TEXT, text)
        context.startActivity(Intent.createChooser(send, "Partager"))
    }

    /**
     * Compte rendu (Markdown du LLM) affiché simplement : titres en gras,
     * listes à puces, **gras** ; le reste tel quel.
     */
    fun LinearLayout.markdown(text: String) {
        val paragraph = StringBuilder()
        val flush = {
            if (paragraph.isNotBlank()) text(inline(paragraph.toString().trim()), 15f).setPadding(0, 0, 0, context.dp(6))
            paragraph.clear()
        }
        for (raw in text.lines()) {
            val line = raw.trimEnd()
            val heading = Regex("^#{1,6}\\s+(.*)").find(line)
            val bullet = Regex("^\\s*[-*•]\\s+(.*)").find(line)
            val numbered = Regex("^\\s*(\\d+)[.)]\\s+(.*)").find(line)
            when {
                heading != null -> {
                    flush()
                    text(heading.groupValues[1].replace("**", ""), 16f, bold = true).setPadding(0, context.dp(10), 0, context.dp(4))
                }
                bullet != null -> {
                    flush()
                    text(inline("•  " + bullet.groupValues[1]), 15f).setPadding(context.dp(4), 0, 0, context.dp(4))
                }
                numbered != null -> {
                    flush()
                    text(inline("${numbered.groupValues[1]}.  ${numbered.groupValues[2]}"), 15f).setPadding(context.dp(4), 0, 0, context.dp(4))
                }
                line.isBlank() -> flush()
                else -> paragraph.append(line).append(' ')
            }
        }
        flush()
    }

    /** `**gras**` en gras, le reste tel quel. */
    private fun inline(text: String): CharSequence {
        val out = SpannableStringBuilder()
        val parts = text.split("**")
        parts.forEachIndexed { index, part ->
            val start = out.length
            out.append(part)
            if (index % 2 == 1 && index < parts.size - 1) {
                out.setSpan(StyleSpan(Typeface.BOLD), start, out.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            }
        }
        return out
    }
}
