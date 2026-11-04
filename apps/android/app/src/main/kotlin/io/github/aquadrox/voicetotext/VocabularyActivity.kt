package io.github.aquadrox.voicetotext

import android.app.Activity
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
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
    private lateinit var wordList: LinearLayout
    private lateinit var snippetList: LinearLayout
    private lateinit var message: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Dictionnaire et snippets"
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
        val content = column().apply {
            message = note("")
            title("Dictionnaire")
            note("Noms propres, jargon, sigles : leur orthographe est donnée à Whisper et au LLM.")
            val word = field("Mot à ajouter")
            button("Ajouter") {
                val value = word.text.toString().trim()
                if (value.isNotEmpty()) {
                    words += value
                    word.setText("")
                    render()
                }
            }
            wordList = LinearLayout(this@VocabularyActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(wordList)

            title("Snippets")
            note("Dicter seulement le déclencheur insère le texte tel quel (ex. « ma signature »).")
            val trigger = field("Déclencheur")
            val text = EditText(this@VocabularyActivity).apply {
                hint = "Texte à insérer"
                minLines = 2
            }.also { addView(it) }
            button("Ajouter le snippet") {
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
            button("Enregistrer") { save() }
        }
        setContentView(ScrollView(this).apply { addView(content) })
        render()
    }

    private fun row(label: String, remove: () -> Unit) = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        addView(TextView(this@VocabularyActivity).apply { text = label }, LinearLayout.LayoutParams(0, -2, 1f))
        addView(Button(this@VocabularyActivity).apply {
            text = "×"
            contentDescription = "Retirer $label"
            setOnClickListener {
                remove()
                render()
            }
        })
    }

    private fun render() {
        wordList.removeAllViews()
        words.toList().forEach { w -> wordList.addView(row(w) { words.remove(w) }) }
        snippetList.removeAllViews()
        snippets.toList().forEach { s -> snippetList.addView(row("« ${s.first} » → ${s.second}") { snippets.remove(s) }) }
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
            message.text = reply.getString("error")
            return
        }
        store.config = reply.getString("ok")
        message.text = "✓ Enregistré : appliqué à la prochaine ouverture du clavier."
    }
}
