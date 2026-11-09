package io.github.aquadrox.voicetotext

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/**
 * Dictionnaire (orthographe des noms propres, jargon) et snippets (un
 * déclencheur dicté insère un texte), comme sur PC.
 */
class VocabularyActivity : Activity() {
    private val store by lazy { Store(this) }
    private val words = mutableListOf<String>()
    private val snippets = mutableListOf<Pair<String, String>>()
    private lateinit var wordList: FlowLayout
    private lateinit var wordsEmpty: TextView
    private lateinit var snippetList: LinearLayout
    private lateinit var message: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val reply = JSONObject(Native.vocabulary(store.config)).optJSONObject("ok")
        reply?.getJSONArray("dictionary")?.let { array ->
            for (i in 0 until array.length()) words += array.getString(i)
        }
        reply?.getJSONArray("snippets")?.let { array ->
            for (i in 0 until array.length()) {
                val snippet = array.getJSONObject(i)
                snippets += snippet.getString("trigger") to snippet.getString("text")
            }
        }
        screen(bar = { primary("Enregistrer") { save() } }) {
            header("Dictionnaire et snippets", "Appliqués à la prochaine ouverture du clavier", onBack = { finish() })
            message = notice()

            card("Dictionnaire", R.drawable.ic_book_a).apply {
                note("Noms propres, jargon, sigles : leur orthographe est donnée à Whisper et au LLM.")
                lateinit var word: EditText
                row {
                    word = field("Mot à ajouter")
                    addAction("Ajouter") {
                        val value = word.text.toString().trim()
                        if (value.isNotEmpty() && value !in words) {
                            words += value
                            render()
                        }
                        word.setText("")
                    }
                }
                wordsEmpty = note("Aucun mot pour l'instant.")
                wordList = FlowLayout(this@VocabularyActivity)
                addView(wordList, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                    topMargin = dp(10)
                })
            }

            card("Snippets", R.drawable.ic_clipboard_paste).apply {
                note("Dicter seulement le déclencheur insère le texte tel quel (ex. « ma signature »).")
                val trigger = field("Déclencheur")
                val text = field("Texte à insérer", lines = 2)
                secondary("Ajouter le snippet") {
                    val t = trigger.text.toString().trim()
                    if (t.isNotEmpty() && text.text.isNotBlank()) {
                        snippets += t to text.text.toString()
                        trigger.setText("")
                        text.setText("")
                        render()
                    }
                }
                snippetList = LinearLayout(this@VocabularyActivity).apply { orientation = LinearLayout.VERTICAL }
                addView(snippetList)
            }
        }
        render()
    }

    /** Bouton « Ajouter » à côté d'un champ : largeur de son texte. */
    private fun LinearLayout.addAction(label: String, action: () -> Unit) {
        secondary(label, action).layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
        ).apply {
            marginStart = dp(8)
            topMargin = dp(8)
        }
    }

    /** Pastille d'un mot, avec sa croix pour le retirer. */
    private fun chip(label: String, remove: () -> Unit) = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        background = rounded(getColor(R.color.bg), 999f, getColor(R.color.border_strong))
        setPadding(dp(12), 0, dp(2), 0)
        text(label, 14f)
        addView(iconButton(R.drawable.ic_x, "Retirer $label") { remove(); render() }, LinearLayout.LayoutParams(dp(30), dp(30)))
    }

    /** Ligne d'un snippet : déclencheur, texte, croix pour le retirer. */
    private fun snippetRow(trigger: String, value: String, remove: () -> Unit) = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, dp(8), 0, dp(8))
        val texts = LinearLayout(this@VocabularyActivity).apply { orientation = LinearLayout.VERTICAL }
        texts.text("« $trigger »", 15f, bold = true)
        texts.text(value, 14f, R.color.muted)
        addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        addView(iconButton(R.drawable.ic_x, "Retirer « $trigger »") { remove(); render() }, LinearLayout.LayoutParams(dp(36), dp(36)))
    }

    private fun render() {
        wordList.removeAllViews()
        words.toList().forEach { w -> wordList.addView(chip(w) { words.remove(w) }) }
        wordsEmpty.say(if (words.isEmpty()) "Aucun mot pour l'instant." else "")
        snippetList.removeAllViews()
        snippets.toList().forEachIndexed { i, s ->
            if (i > 0) snippetList.divider(0)
            snippetList.addView(snippetRow(s.first, s.second) { snippets.remove(s) })
        }
    }

    private fun save() {
        val vocabulary = JSONObject()
            .put("dictionary", JSONArray(words))
            .put(
                "snippets",
                JSONArray(snippets.map { JSONObject().put("trigger", it.first).put("text", it.second) }),
            )
        val reply = JSONObject(Native.saveVocabulary(store.config, vocabulary.toString()))
        if (reply.has("error")) {
            message.say(reply.getString("error"))
            return
        }
        store.config = reply.getString("ok")
        message.say("✓ Enregistré : appliqué à la prochaine ouverture du clavier.")
    }
}
