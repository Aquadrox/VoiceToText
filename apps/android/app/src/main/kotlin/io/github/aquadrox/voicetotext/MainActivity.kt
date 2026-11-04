package io.github.aquadrox.voicetotext

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.InputType
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Réglages et mise en route : micro, modèle Whisper, clavier, nettoyage LLM,
 * import des réglages du PC, zone d'essai.
 */
class MainActivity : Activity() {
    private val store by lazy { Store(this) }
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    private lateinit var micStatus: TextView
    private lateinit var keyboardStatus: TextView
    private lateinit var models: RadioGroup
    private lateinit var download: Button
    private lateinit var progress: ProgressBar
    private lateinit var llmEnabled: Switch
    private lateinit var baseUrl: EditText
    private lateinit var llmModel: EditText
    private lateinit var apiKey: EditText
    private lateinit var keyStatus: TextView
    private lateinit var voiceCommands: Switch
    private lateinit var message: TextView

    /** Hôte de l'API affiché (la clé est rangée par hôte). */
    private var host = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(ScrollView(this).apply { addView(content()) })
        refresh()
    }

    override fun onResume() {
        super.onResume()
        refreshStatus()
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private fun content(): View = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(20), dp(24), dp(20), dp(32))

        title("VoiceToText", 26f)
        note("Clavier de dictée : parlez, le texte nettoyé s'insère là où vous écrivez. La voix est transcrite sur le téléphone.")
        message = note("").apply { visibility = View.GONE }

        section("1. Micro")
        micStatus = note("")
        button("Autoriser le micro") {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
        }

        section("2. Modèle de transcription")
        models = RadioGroup(this@MainActivity).also { addView(it) }
        progress = ProgressBar(this@MainActivity, null, android.R.attr.progressBarStyleHorizontal).apply {
            max = 100
            visibility = View.GONE
        }.also { addView(it) }
        download = button("Télécharger le modèle choisi") { downloadModel() }

        section("3. Clavier")
        keyboardStatus = note("")
        button("Activer le clavier VoiceToText") {
            startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
        }
        button("Choisir le clavier") {
            getSystemService(InputMethodManager::class.java).showInputMethodPicker()
        }
        note("Maintenez 🎤 pour dicter, relâchez pour insérer ; appui court : mains libres. 🌐 revient à votre clavier habituel.")

        section("Nettoyage du texte (LLM)")
        llmEnabled = Switch(this@MainActivity).apply { text = "Nettoyer et ponctuer avec un LLM" }.also { addView(it) }
        baseUrl = field("Adresse de l'API (ex. https://api.deepseek.com)")
        llmModel = field("Modèle (ex. deepseek-flash)")
        apiKey = field("Nouvelle clé API (jamais affichée)").apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        }
        keyStatus = note("")
        voiceCommands = Switch(this@MainActivity).apply {
            text = "Commandes vocales (« nouvelle ligne », « annule la dernière dictée »)"
        }.also { addView(it) }
        button("Enregistrer") { save() }
        button("Supprimer la clé") {
            if (host.isNotEmpty()) store.deleteKey(host)
            refresh()
            say("Clé supprimée.")
        }

        section("Réglages du PC")
        note("Sur PC : Réglages → Exporter… puis copiez le fichier sur le téléphone. Dictionnaire, snippets, styles et fournisseur LLM sont repris ; les clés API ne sont jamais exportées.")
        button("Importer un fichier de réglages…") {
            startActivityForResult(
                Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = "*/*"
                },
                IMPORT,
            )
        }

        section("Essayer")
        addView(EditText(this@MainActivity).apply {
            hint = "Touchez ici, choisissez le clavier VoiceToText et dictez"
            minLines = 3
        })
    }

    private fun LinearLayout.title(text: String, size: Float) = TextView(this@MainActivity).apply {
        this.text = text
        textSize = size
        setTypeface(typeface, Typeface.BOLD)
    }.also { addView(it) }

    private fun LinearLayout.section(text: String) = title(text, 18f).apply {
        setPadding(0, dp(24), 0, dp(4))
    }

    private fun LinearLayout.note(text: String) = TextView(this@MainActivity).apply {
        this.text = text
        alpha = 0.8f
        setPadding(0, dp(4), 0, dp(4))
    }.also { addView(it) }

    private fun LinearLayout.button(text: String, action: () -> Unit) = Button(this@MainActivity).apply {
        this.text = text
        setOnClickListener { action() }
    }.also { addView(it) }

    private fun LinearLayout.field(hint: String) = EditText(this@MainActivity).apply {
        this.hint = hint
        isSingleLine = true
    }.also { addView(it) }

    private fun say(text: String) {
        message.text = text
        message.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
    }

    /** Réglages relus et affichés. */
    private fun refresh() {
        val reply = JSONObject(Native.settings(store.config, store.modelsDir))
        val view = reply.optJSONObject("ok") ?: run {
            say(reply.optString("error"))
            return
        }
        val llm = view.getJSONObject("llm")
        llmEnabled.isChecked = llm.getBoolean("enabled")
        baseUrl.setText(llm.getString("base_url"))
        llmModel.setText(llm.getString("model"))
        host = llm.getString("host")
        apiKey.setText("")
        keyStatus.text = if (store.hasKey(host)) "✓ Clé enregistrée pour $host" else "Aucune clé pour $host"
        voiceCommands.isChecked = view.getBoolean("voice_commands")

        models.removeAllViews()
        val list = view.getJSONArray("models")
        for (i in 0 until list.length()) {
            val model = list.getJSONObject(i)
            val downloaded = if (model.getBoolean("downloaded")) " · téléchargé" else ""
            models.addView(RadioButton(this).apply {
                id = View.generateViewId()
                tag = model.getString("id")
                text = "${model.getString("label")} · ${model.getInt("size_mb")} Mo$downloaded"
                isChecked = model.getString("id") == view.getString("model")
            })
        }
        refreshStatus()
    }

    private fun refreshStatus() {
        val mic = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        micStatus.text = if (mic) "✓ Micro autorisé" else "Micro non autorisé"
        val enabled = getSystemService(InputMethodManager::class.java).enabledInputMethodList
            .any { it.packageName == packageName }
        keyboardStatus.text = if (enabled) "✓ Clavier activé" else "Clavier pas encore activé"
    }

    private fun selectedModel(): String =
        models.findViewById<RadioButton>(models.checkedRadioButtonId)?.tag as? String ?: ""

    private fun save(): Boolean {
        val update = JSONObject()
            .put("llm_enabled", llmEnabled.isChecked)
            .put("base_url", baseUrl.text.toString())
            .put("llm_model", llmModel.text.toString())
            .put("model", selectedModel())
            .put("voice_commands", voiceCommands.isChecked)
        val reply = JSONObject(Native.updateSettings(store.config, update.toString()))
        if (reply.has("error")) {
            say(reply.getString("error"))
            return false
        }
        store.config = reply.getString("ok")
        val newHost = JSONObject(Native.settings(store.config, store.modelsDir))
            .optJSONObject("ok")?.getJSONObject("llm")?.getString("host").orEmpty()
        val key = apiKey.text.toString()
        if (key.isNotBlank() && newHost.isNotEmpty()) store.saveKey(newHost, key)
        refresh()
        say("✓ Enregistré : appliqué à la prochaine ouverture du clavier.")
        return true
    }

    private fun downloadModel() {
        if (!save()) return
        val id = selectedModel()
        download.isEnabled = false
        progress.visibility = View.VISIBLE
        progress.progress = 0
        say("Téléchargement du modèle…")
        val poll = object : Runnable {
            override fun run() {
                progress.progress = Native.progress()
                if (!download.isEnabled) main.postDelayed(this, 500)
            }
        }
        main.post(poll)
        worker.execute {
            val reply = JSONObject(Native.download(store.modelsDir, id))
            main.post {
                download.isEnabled = true
                progress.visibility = View.GONE
                say(if (reply.has("error")) reply.getString("error") else "✓ Modèle prêt.")
                refresh()
            }
        }
    }

    @Deprecated("API Activity simple, sans AndroidX")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri: Uri = data?.data ?: return
        if (requestCode != IMPORT || resultCode != RESULT_OK) return
        val text = runCatching {
            contentResolver.openInputStream(uri)?.use { it.readBytes().toString(Charsets.UTF_8) }
        }.getOrNull()
        if (text == null || text.length > 1_000_000) {
            say("Fichier illisible.")
            return
        }
        val reply = JSONObject(Native.importSettings(text))
        val ok = reply.optJSONObject("ok") ?: run {
            say(reply.optString("error"))
            return
        }
        store.config = ok.getString("toml")
        refresh()
        val notes = ok.getJSONArray("notes")
        val extra = (0 until notes.length()).joinToString("") { "\n• ${notes.getString(it)}" }
        say("✓ Réglages du PC importés. Saisissez la clé API ci-dessus.$extra")
    }

    @Deprecated("API Activity simple, sans AndroidX")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        refreshStatus()
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    private companion object {
        const val IMPORT = 2
    }
}
