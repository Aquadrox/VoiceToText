package io.github.aquadrox.voicetotext

import android.app.Activity
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/**
 * Prompts système du nettoyage et du Command Mode, comme la page Prompts du
 * PC : prompt intégré en lecture seule, copies modifiables, choix du prompt
 * utilisé. Les prompts importés du PC apparaissent ici.
 */
class PromptsActivity : Activity() {
    private val store by lazy { Store(this) }
    private lateinit var message: TextView
    private lateinit var cleanup: PromptEditor
    private lateinit var command: PromptEditor

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val reply = JSONObject(Native.prompts(store.config))
        val prompts = reply.optJSONObject("ok")
        screen(bar = { primary("Enregistrer") { save() } }) {
            header("Prompts système", "Consignes données au LLM", onBack = { finish() })
            message = notice()
            if (prompts == null) {
                message.say(reply.optString("error"))
                return@screen
            }
            note("Utilisés quand « Nettoyer et ponctuer avec un LLM » est activé. Le prompt intégré reste disponible : « Créer une copie » pour le modifier.")
            cleanup = PromptEditor(
                card("Nettoyage des dictées", R.drawable.ic_spell_check_2),
                prompts.getJSONObject("cleanup"),
                "Ponctuation, hésitations, auto-corrections : texte dicté → texte propre.",
            )
            command = PromptEditor(
                card("Command Mode", R.drawable.ic_text_select),
                prompts.getJSONObject("command"),
                "Consigne dictée appliquée au texte sélectionné, ou texte rédigé au curseur.",
            )
        }
    }

    private fun save() {
        if (!::cleanup.isInitialized) return
        val prompts = JSONObject().put("cleanup", cleanup.toJson()).put("command", command.toJson())
        val reply = JSONObject(Native.savePrompts(store.config, prompts.toString()))
        if (reply.has("error")) {
            message.say(reply.getString("error"))
            return
        }
        store.config = reply.getString("ok")
        message.say("✓ Enregistré : appliqué à la prochaine ouverture du clavier.")
    }

    private class Preset(var name: String, var text: String)

    /** Un usage : liste des prompts (intégré en tête), nom et texte du prompt choisi. */
    private inner class PromptEditor(card: LinearLayout, json: JSONObject, description: String) {
        private val builtin = json.getString("builtin")
        private val presets = json.getJSONArray("presets").let { array ->
            MutableList(array.length()) { i ->
                array.getJSONObject(i).let { Preset(it.getString("name"), it.getString("text")) }
            }
        }
        private val labels = mutableListOf<String>()
        private val adapter = ArrayAdapter(this@PromptsActivity, android.R.layout.simple_spinner_dropdown_item, labels)
        private val choice: Spinner
        private val name: EditText
        private val readOnly: TextView
        private val text: EditText
        private lateinit var delete: Button

        /** 0 : prompt intégré ; sinon `presets[selected - 1]`. */
        private var selected = 0

        init {
            card.note(description)
            card.text("Prompt utilisé", 14f, bold = true).setPadding(0, dp(8), 0, 0)
            choice = Spinner(this@PromptsActivity).also { card.addView(it) }
            name = card.field("Nom du prompt")
            readOnly = card.note("Prompt intégré, en lecture seule.")
            text = card.field("Texte du prompt", lines = 8).apply {
                gravity = Gravity.TOP or Gravity.START
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
            }
            card.row {
                secondary("Créer une copie") { copy() }
                delete = secondary("Supprimer") { remove() }
            }
            refreshLabels()
            choice.adapter = adapter
            val active = json.getString("active")
            show(presets.indexOfFirst { it.name == active } + 1, keep = false)
            choice.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                    if (position != selected) show(position)
                }

                override fun onNothingSelected(parent: AdapterView<*>?) {}
            }
            // Nom modifié : la liste suit.
            name.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) {
                    if (selected == 0) return
                    presets[selected - 1].name = s.toString()
                    refreshLabels()
                }
            })
        }

        private fun refreshLabels() {
            labels.clear()
            labels += "Prompt intégré"
            presets.forEach { labels += it.name.trim().ifEmpty { "(sans nom)" } }
            adapter.notifyDataSetChanged()
        }

        /** Texte saisi rangé dans le prompt choisi. */
        private fun keep() {
            if (selected > 0) presets[selected - 1].text = text.text.toString()
        }

        /** Affiche le prompt `index` ; `keep` : garder d'abord la saisie en cours. */
        private fun show(index: Int, keep: Boolean = true) {
            if (keep) keep()
            selected = index
            val custom = index > 0
            val preset = presets.getOrNull(index - 1)
            name.visibility = if (custom) View.VISIBLE else View.GONE
            readOnly.visibility = if (custom) View.GONE else View.VISIBLE
            // `selected` déjà à jour : l'observateur réécrit le même nom.
            if (custom) name.setText(preset!!.name)
            text.setText(preset?.text ?: builtin)
            text.isFocusable = custom
            text.isFocusableInTouchMode = custom
            text.isCursorVisible = custom
            text.setTextColor(getColor(if (custom) R.color.fg else R.color.muted))
            delete.isEnabled = custom
            if (choice.selectedItemPosition != index) choice.setSelection(index)
        }

        /** Nouveau prompt modifiable, copie de celui affiché. */
        private fun copy() {
            keep()
            val base = "Mon prompt"
            val taken = presets.map { it.name.trim().lowercase() }.toSet()
            val label = generateSequence(1) { it + 1 }
                .map { if (it == 1) base else "$base $it" }
                .first { it.lowercase() !in taken }
            presets += Preset(label, text.text.toString())
            refreshLabels()
            show(presets.size, keep = false)
            message.say("Copie « $label » créée : modifiez-la puis enregistrez.")
        }

        private fun remove() {
            if (selected == 0) return
            presets.removeAt(selected - 1)
            refreshLabels()
            show(0, keep = false)
        }

        fun toJson(): JSONObject {
            keep()
            val active = presets.getOrNull(selected - 1)?.name?.trim().orEmpty()
            return JSONObject()
                .put("active", active)
                .put("presets", JSONArray(presets.map { JSONObject().put("name", it.name).put("text", it.text) }))
        }
    }
}
