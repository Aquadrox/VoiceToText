package io.github.aquadrox.voicetotext

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import java.text.DateFormat
import java.util.Date

/** Dictées récentes (sur le téléphone seulement) : toucher pour copier. */
class HistoryActivity : Activity() {
    private val store by lazy { Store(this) }
    private lateinit var list: LinearLayout
    private var confirmClear = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Historique"
        val content = column().apply {
            note("Gardé sur le téléphone seulement. Touchez une dictée pour la copier.")
            button("Tout effacer") { clear() }
            list = LinearLayout(this@HistoryActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(list)
        }
        setContentView(ScrollView(this).apply { addView(content) })
        load()
    }

    private fun load() {
        list.removeAllViews()
        val reply = JSONObject(Native.history(store.historyFile, SHOWN))
        val entries = reply.optJSONArray("ok")
        if (entries == null || entries.length() == 0) {
            list.note(reply.optString("error").ifEmpty { "Aucune dictée pour l'instant." })
            return
        }
        val format = DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT)
        for (i in 0 until entries.length()) {
            val entry = entries.getJSONObject(i)
            val text = entry.getString("text")
            val meta = buildList {
                add(format.format(Date(entry.getLong("at_ms"))))
                entry.optString("app").takeIf { it.isNotEmpty() && it != "null" }?.let { add(it) }
                if (entry.optBoolean("command")) add("consigne")
                entry.optString("translated").takeIf { it.isNotEmpty() && it != "null" }?.let { add("traduit : $it") }
                if (entry.optBoolean("cleanup_failed")) add("non nettoyé")
            }.joinToString(" · ")
            list.addView(TextView(this).apply {
                this.text = "$meta\n$text"
                setPadding(0, dp(12), 0, dp(12))
                setOnClickListener {
                    getSystemService(ClipboardManager::class.java)
                        .setPrimaryClip(ClipData.newPlainText("Dictée", text))
                    Toast.makeText(this@HistoryActivity, "Copié", Toast.LENGTH_SHORT).show()
                }
            })
        }
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
