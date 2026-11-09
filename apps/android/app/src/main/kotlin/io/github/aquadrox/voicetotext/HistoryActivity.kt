package io.github.aquadrox.voicetotext

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import java.math.MathContext
import java.text.DateFormat
import java.text.NumberFormat
import java.util.Currency
import java.util.Date
import java.util.Locale

/** Dictées récentes (sur le téléphone seulement) : toucher pour copier. */
class HistoryActivity : Activity() {
    private val store by lazy { Store(this) }
    private lateinit var summary: TextView
    private lateinit var notes: LinearLayout
    private lateinit var list: LinearLayout
    private lateinit var clearButton: TextView
    private var confirmClear = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        screen {
            summary = header("Historique", "Gardé sur le téléphone seulement", onBack = { finish() })
            notes = LinearLayout(this@HistoryActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(notes)
            note("Touchez une dictée pour la copier.")
            // Liste dans une carte, dictées séparées par un trait.
            list = card()
            list.setPadding(0, dp(4), 0, dp(4))
            // Effet d'appui des dictées rogné aux coins arrondis de la carte.
            list.clipToOutline = true
            clearButton = secondary("Tout effacer") { clear() }.apply { setTextColor(getColor(R.color.danger)) }
        }
        load()
    }

    private fun load() {
        summary.text = "Gardé sur le téléphone seulement"
        notes.removeAllViews()
        list.removeAllViews()
        // Pourquoi l'historique peut rester vide : dit clairement.
        val settings = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")
        if (settings?.optBoolean("history") == false) {
            notes.note("Historique coupé dans les réglages (« Garder les dictées sur le téléphone ») : rien n'est gardé.")
        } else if (BuildConfig.BUBBLE && !store.bubbleHistory) {
            notes.note("Les dictées de la bulle ne sont pas gardées (réglage de la bulle).")
        }
        val reply = JSONObject(Native.history(store.historyFile, SHOWN))
        val entries = reply.optJSONArray("ok")
        if (entries == null || entries.length() == 0) {
            list.note(reply.optString("error").ifEmpty { "Aucune dictée pour l'instant." }).setPadding(dp(16), dp(12), dp(16), dp(12))
            clearButton.visibility = View.GONE
            return
        }
        clearButton.visibility = View.VISIBLE
        val total = (0 until entries.length()).map { entries.getJSONObject(it) }
            .filter { it.has("cost") && !it.isNull("cost") }
            .groupBy { it.optString("currency", "USD") }
            .map { (currency, items) -> cost(items.sumOf { it.getDouble("cost") }, currency) }
        summary.text = "${entries.length()} dictée(s), gardées sur le téléphone" +
            if (total.isEmpty()) "" else " · coût LLM estimé : ${total.joinToString(" + ")}"
        val format = DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT)
        for (i in 0 until entries.length()) {
            val entry = entries.getJSONObject(i)
            val text = entry.getString("text")
            val meta = buildList {
                add(format.format(Date(entry.getLong("at_ms"))))
                entry.optString("app").takeIf { it.isNotEmpty() && it != "null" }?.let { add(it) }
                if (entry.has("cost") && !entry.isNull("cost")) {
                    add(cost(entry.getDouble("cost"), entry.optString("currency", "USD")))
                }
                if (entry.optBoolean("command")) add("consigne")
                entry.optString("translated").takeIf { it.isNotEmpty() && it != "null" }?.let { add("traduit : $it") }
                if (entry.optBoolean("cleanup_failed")) add("non nettoyé")
            }.joinToString(" · ")
            if (i > 0) list.divider(0)
            val item = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(16), dp(10), dp(16), dp(10))
                background = pressable(null, 0f)
                setOnClickListener {
                    getSystemService(ClipboardManager::class.java)
                        .setPrimaryClip(ClipData.newPlainText("Dictée", text))
                    Toast.makeText(this@HistoryActivity, "Copié", Toast.LENGTH_SHORT).show()
                }
            }
            item.text(meta, 12f, R.color.muted)
            item.text(text, 15f)
            list.addView(item)
        }
    }

    /** Comme sur PC : chiffres significatifs pour les montants minuscules. */
    private fun cost(value: Double, currency: String): String {
        val tiny = value > 0 && value < 0.01
        val shown = if (tiny) value.toBigDecimal().round(MathContext(2)).toDouble() else value
        return runCatching {
            NumberFormat.getCurrencyInstance(Locale.FRANCE).apply {
                this.currency = Currency.getInstance(currency)
                maximumFractionDigits = if (tiny) 8 else 2
                minimumFractionDigits = 2
            }.format(shown)
        }.getOrElse { "%.4f %s".format(shown, currency) }
    }

    /** Effacement en deux appuis. */
    private fun clear() {
        if (!confirmClear) {
            confirmClear = true
            Toast.makeText(this, "Appuyez de nouveau pour tout effacer", Toast.LENGTH_SHORT).show()
            return
        }
        confirmClear = false
        Native.clearHistory(store.historyFile)
        load()
    }

    private companion object {
        const val SHOWN = 300
    }
}
