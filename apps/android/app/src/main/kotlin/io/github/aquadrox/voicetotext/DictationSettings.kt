package io.github.aquadrox.voicetotext

import android.content.Intent
import android.provider.Settings
import android.util.TypedValue
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/** Dictée : modèle de transcription, clavier, traduction, commandes vocales. */
class DictationSettings : SettingsPage() {
    override val section = Section.DICTATION
    override val subtitle = "La voix est transcrite sur le téléphone"

    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var models: RadioGroup
    private lateinit var modelState: TextView
    private lateinit var download: Button
    private lateinit var progress: ProgressBar
    private lateinit var keyboardState: TextView
    private lateinit var keyboardActivate: Button
    private lateinit var language: Spinner
    private lateinit var voiceCommands: Switch
    private var languages = emptyList<String>()

    /** Langue posée par le code : Android la signale ensuite, ignorée ici. */
    private var shownLanguage = -1

    override fun LinearLayout.build() {
        card("Modèle de transcription", R.drawable.ic_audio_lines).anchor("model").apply {
            note("Plus le modèle est gros, plus il est précis, mais plus il est lent et lourd. Il se télécharge une seule fois, puis fonctionne sans internet.")
            models = RadioGroup(this@DictationSettings).also { addView(it) }
            modelState = note("")
            progress = ProgressBar(this@DictationSettings, null, android.R.attr.progressBarStyleHorizontal).apply {
                max = 100
                visibility = View.GONE
            }.also { addView(it) }
            download = primary("Télécharger") { downloadModel() }
        }

        card("Clavier VoiceToText", R.drawable.ic_keyboard).anchor("keyboard").apply {
            keyboardState = note("")
            row {
                keyboardActivate = secondary("Activer") { startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS)) }
                secondary("Changer de clavier") {
                    getSystemService(InputMethodManager::class.java).showInputMethodPicker()
                }
            }
        }

        card("Options", R.drawable.ic_languages).apply {
            text("Langue de traduction", 16f).anchor("translation").setPadding(0, dp(8), 0, 0)
            note("Le mode traduction (touche à gauche du micro) dicte directement dans cette langue.")
            language = Spinner(this@DictationSettings).also { addView(it) }
            divider()
            voiceCommands = toggle(
                "Commandes vocales",
                "Dites « nouvelle ligne » ou « annule la dernière dictée » au lieu de les écrire ; épelez un mot lettre par lettre (« A, Q, U… ») pour l'écrire d'un bloc.",
            ) { on ->
                saveSettings(
                    JSONObject().put("voice_commands", on),
                    if (on) "✓ Commandes vocales activées" else "✓ Commandes vocales désactivées : tout est écrit tel quel",
                )
            }.anchor("voice_commands")
        }
        refresh()
        language.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                if (position == shownLanguage) return
                shownLanguage = position
                val chosen = languages.getOrNull(position) ?: return
                saveSettings(JSONObject().put("translation", chosen), "✓ Mode traduction : vers le $chosen")
            }
            override fun onNothingSelected(parent: AdapterView<*>?) {}
        }
    }

    override fun onResume() {
        super.onResume()
        // Retour des réglages d'Android (clavier activé) : état à jour.
        if (::keyboardState.isInitialized) refreshKeyboard()
    }

    private fun refresh() {
        val view = Config.view(store) ?: return say("Réglages illisibles.")
        voiceCommands.quietly(view.getBoolean("voice_commands"))
        val translation = view.getString("translation")
        languages = view.getJSONArray("languages").let { a -> List(a.length()) { a.getString(it) } }
            .let { if (translation in it) it else it + translation }
        language.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_dropdown_item, languages)
        shownLanguage = languages.indexOf(translation)
        language.setSelection(shownLanguage, false)
        refreshModels(view)
        refreshKeyboard()
    }

    private fun refreshModels(view: JSONObject) {
        models.setOnCheckedChangeListener(null)
        models.removeAllViews()
        val list = view.getJSONArray("models").let { a -> List(a.length()) { a.getJSONObject(it) } }
        val current = view.getString("model")
        for (model in list) {
            val downloaded = if (model.getBoolean("downloaded")) " · téléchargé" else ""
            models.addView(RadioButton(this).apply {
                id = View.generateViewId()
                tag = model.getString("id")
                text = "${model.getString("label")} · ${model.getInt("size_mb")} Mo$downloaded"
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setPadding(dp(4), dp(10), 0, dp(10))
                isChecked = model.getString("id") == current
            })
        }
        val chosen = list.firstOrNull { it.getString("id") == current }
        val ready = chosen?.getBoolean("downloaded") == true
        val name = chosen?.getString("label")?.substringBefore(" (").orEmpty()
        val busy = downloading != null
        modelState.text = when {
            busy -> "Téléchargement de $name…"
            chosen == null -> "Choisissez un modèle."
            ready -> "✓ $name : prêt."
            else -> "$name n'est pas encore sur le téléphone : téléchargez-le pour dicter."
        }
        download.text = "Télécharger $name (${chosen?.optInt("size_mb") ?: 0} Mo)"
        download.visibility = if (ready || chosen == null) View.GONE else View.VISIBLE
        download.isEnabled = !busy
        progress.visibility = if (busy) View.VISIBLE else View.GONE
        if (busy) poll()
        models.setOnCheckedChangeListener { group, id ->
            val model = group.findViewById<RadioButton>(id)?.tag as? String ?: return@setOnCheckedChangeListener
            val label = list.first { it.getString("id") == model }
            val short = label.getString("label").substringBefore(" (")
            saveSettings(
                JSONObject().put("model", model),
                if (label.getBoolean("downloaded")) "✓ $short : utilisé dès la prochaine dictée" else "✓ $short choisi : téléchargez-le pour l'utiliser",
            )
            // Après la fin de l'appui : les boutons sont recréés.
            main.post { Config.view(store)?.let { refreshModels(it) } }
        }
    }

    private fun refreshKeyboard() {
        val enabled = getSystemService(InputMethodManager::class.java).enabledInputMethodList
            .any { it.packageName == packageName }
        keyboardState.text = if (enabled) {
            "✓ Activé. Dans une application, passez au clavier VoiceToText avec l'icône clavier en bas de l'écran, ou ci-dessous."
        } else {
            "Pas encore activé : activez-le une fois dans les réglages d'Android, puis choisissez-le."
        }
        keyboardActivate.visibility = if (enabled) View.GONE else View.VISIBLE
    }

    private fun poll() {
        main.post(object : Runnable {
            override fun run() {
                if (isDestroyed) return
                progress.progress = Native.progress()
                if (downloading != null) main.postDelayed(this, 500)
            }
        })
    }

    private fun downloadModel() {
        val id = models.findViewById<RadioButton>(models.checkedRadioButtonId)?.tag as? String ?: return
        if (downloading != null) return
        downloading = id
        progress.progress = 0
        Config.view(store)?.let { refreshModels(it) }
        val modelsDir = store.modelsDir
        worker.execute {
            val reply = JSONObject(Native.download(modelsDir, id))
            downloading = null
            main.post {
                if (isDestroyed) return@post
                if (reply.has("error")) say(reply.getString("error")) else saved("✓ Modèle prêt : vous pouvez dicter.")
                Config.view(store)?.let { refreshModels(it) }
            }
        }
    }

    override fun onDestroy() {
        // Le téléchargement en cours continue : la page le retrouve à son retour.
        worker.shutdown()
        super.onDestroy()
    }

    private companion object {
        /** Modèle en cours de téléchargement (null : aucun), d'une page à l'autre. */
        @Volatile
        var downloading: String? = null
    }
}
