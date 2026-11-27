package io.github.aquadrox.voicetotext

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.SystemClock
import android.provider.Settings
import android.util.TypedValue
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors

/** Dictée : modèle de transcription, micro, clavier, traduction, commandes vocales. */
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
    private lateinit var audioEnhance: Switch
    private lateinit var screenContext: Switch
    private lateinit var liveInsert: Switch
    private lateinit var excludedBox: LinearLayout
    private lateinit var excludedInput: EditText
    private var languages = emptyList<String>()
    private lateinit var microphones: RadioGroup
    private lateinit var microphoneState: TextView
    private lateinit var meter: ProgressBar
    private lateinit var testButton: Button

    /** Test du micro en cours (null : aucun), et son début. */
    private var tester: Recorder? = null
    private var testStart = 0L

    /** Micro branché ou débranché : la liste suit, page ouverte. */
    private val plugged = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) = refreshMicrophones()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) = refreshMicrophones()
    }

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

        card("Micro", R.drawable.ic_mic).anchor("microphone").apply {
            note("Pour dicter et enregistrer les réunions. Un micro branché (prise jack, USB, Bluetooth) apparaît ici dès qu'il est connecté.")
            microphones = RadioGroup(this@DictationSettings).also { addView(it) }
            microphoneState = note("")
            meter = ProgressBar(this@DictationSettings, null, android.R.attr.progressBarStyleHorizontal).apply {
                max = 100
                visibility = View.GONE
            }.also { addView(it) }
            testButton = secondary("Tester le micro") { testMicrophone() }
            divider()
            audioEnhance = toggle(
                "Améliorer le son du micro avant transcription",
                "Filtre les bruits sourds (passe-haut 80 Hz) et remonte doucement les voix faibles ou éloignées sans amplifier le silence. 100 % local, appliqué aux dictées et aux réunions.",
            ) { on ->
                saveSettings(
                    JSONObject().put("audio_enhance", on),
                    if (on) "✓ Prétraitement audio activé" else "✓ Prétraitement audio désactivé",
                )
            }.anchor("audio_enhance")
            more(
                "Automatique : en général, Android prend le micro du casque filaire ou USB branché, sinon celui du téléphone. " +
                    "Le micro d'un casque Bluetooth n'est utilisé que si vous le choisissez ici.\n\n" +
                    "Bluetooth : pendant l'enregistrement, le casque passe en mode appel (son de moindre qualité). " +
                    "Il lui faut environ une seconde pour s'ouvrir : attendez que l'anneau du micro bouge avant de parler.\n\n" +
                    "Micro choisi débranché : Android choisit, jusqu'à ce qu'il soit rebranché.",
            )
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
            divider()
            liveInsert = toggle(
                "Insérer le texte au fil de la dictée",
                "Pendant une longue dictée, chaque phrase est nettoyée puis insérée dès que vous marquez une pause, au lieu de tout insérer à la fin : vous voyez ce qui est écrit et pouvez vous reprendre. Un appel au LLM par phrase. Jamais sur un mot de passe.",
            ) { on ->
                saveSettings(
                    JSONObject().put("live_insert", on),
                    if (on) "✓ Le texte s'insère phrase par phrase pendant la dictée" else "✓ Le texte s'insère en une fois à la fin de la dictée",
                )
            }.anchor("live_insert")
            divider()
            screenContext = toggle(
                "Contexte du champ actif",
                "Lit quelques mots autour du curseur pour mieux orthographier les noms propres et termes techniques. Jamais lu sur les mots de passe, la navigation privée ni les gestionnaires de mots de passe ; rien n'est enregistré.",
            ) { on ->
                excludedBox.visibility = if (on) View.VISIBLE else View.GONE
                saveSettings(
                    JSONObject().put("screen_context", on),
                    if (on) "✓ Contexte du champ actif activé" else "✓ Contexte du champ actif désactivé",
                )
            }.anchor("screen_context")
            excludedBox = LinearLayout(this@DictationSettings).apply {
                orientation = LinearLayout.VERTICAL
                visibility = View.GONE
                note("Applications exclues (noms ou paquets séparés par des virgules ; les gestionnaires de mots de passe sont déjà exclus).")
                excludedInput = field("Ex. : whatsapp, banque, com.example.app")
                secondary("Enregistrer les exclusions") {
                    val apps = ContextLogic.parseExcludedApps(excludedInput.text.toString())
                    excludedInput.setText(ContextLogic.formatExcludedApps(apps))
                    saveSettings(
                        JSONObject().put("screen_context_excluded_apps", JSONArray(apps)),
                        if (apps.isEmpty()) "✓ Aucune application exclue" else "✓ ${apps.size} application(s) exclue(s) du contexte",
                    )
                }
            }.also { addView(it) }
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
        // Appelé aussitôt avec les micros présents : la liste se remplit.
        getSystemService(AudioManager::class.java).registerAudioDeviceCallback(plugged, main)
    }

    override fun onPause() {
        getSystemService(AudioManager::class.java).unregisterAudioDeviceCallback(plugged)
        // Le micro n'écoute pas en arrière-plan.
        finishTest()
        super.onPause()
    }

    /** Liste des micros (choix enregistré dès qu'on le touche) et état. */
    private fun refreshMicrophones() {
        if (isDestroyed) return
        microphones.setOnCheckedChangeListener(null)
        microphones.removeAllViews()
        val list = Microphones.list(this)
        val chosen = store.microphone
        val present = Microphones.match(list, chosen)
        val choices = mutableListOf("" to "Automatique (choix d'Android)")
        list.forEach { choices += it.key to it.label }
        // Débranché : reste affiché, coché, pour qu'on sache ce qui est choisi.
        if (chosen.isNotEmpty() && present == null) {
            choices += chosen to "${store.microphoneLabel.ifEmpty { "Micro choisi" }} · débranché"
        }
        val checked = present?.key ?: chosen
        for ((key, label) in choices) {
            microphones.addView(RadioButton(this).apply {
                id = View.generateViewId()
                tag = key
                text = label
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setPadding(dp(4), dp(10), 0, dp(10))
                isChecked = key == checked
            })
        }
        microphones.setOnCheckedChangeListener { group, id ->
            val button = group.findViewById<RadioButton>(id) ?: return@setOnCheckedChangeListener
            val key = button.tag as? String ?: return@setOnCheckedChangeListener
            if (key == store.microphone) return@setOnCheckedChangeListener
            val label = list.firstOrNull { it.key == key }?.label ?: store.microphoneLabel
            store.microphone = key
            store.microphoneLabel = if (key.isEmpty()) "" else label
            saved(if (key.isEmpty()) "✓ Android choisit le micro" else "✓ $label : utilisé dès le prochain enregistrement")
            showMicrophoneState()
        }
        if (tester == null) showMicrophoneState()
    }

    /** Micro choisi absent, et micro qui a capté le dernier enregistrement. */
    private fun showMicrophoneState() {
        val chosen = store.microphone
        val missing = chosen.isNotEmpty() && Microphones.match(Microphones.list(this), chosen) == null
        val last = store.lastMicrophone
        microphoneState.text = listOfNotNull(
            if (missing) "⚠ ${store.microphoneLabel.ifEmpty { "Le micro choisi" }} n'est pas branché : Android choisit en attendant." else null,
            if (last.isEmpty()) "Touchez « Tester le micro » pour voir lequel capte votre voix." else "Dernier enregistrement : $last.",
        ).joinToString("\n")
    }

    /** Écoute quelques secondes : niveau de la voix et micro qui la capte. */
    private fun testMicrophone() {
        if (tester != null) {
            finishTest()
            return
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), MICROPHONE_REQUEST)
            return
        }
        if (MeetingService.recording) {
            microphoneState.text = "Réunion en cours : le micro est pris par l'enregistrement."
            return
        }
        val recorder = Recorder(this)
        if (!recorder.start()) {
            microphoneState.text = "⚠ Micro indisponible (pris par une autre application ?)."
            return
        }
        tester = recorder
        testStart = SystemClock.elapsedRealtime()
        testButton.text = "Arrêter le test"
        meter.progress = 0
        meter.visibility = View.VISIBLE
        main.post(testTick)
    }

    private val testTick = object : Runnable {
        override fun run() {
            val recorder = tester ?: return
            meter.progress = (kotlin.math.sqrt(recorder.level) * 100).toInt()
            val microphone = recorder.microphone
            microphoneState.text = if (microphone.isEmpty()) "Parlez…" else "Parlez… Le son arrive par : $microphone."
            if (SystemClock.elapsedRealtime() - testStart >= TEST_MS) finishTest() else main.postDelayed(this, 50)
        }
    }

    private fun finishTest() {
        val recorder = tester ?: return
        tester = null
        main.removeCallbacks(testTick)
        recorder.stop()
        testButton.text = "Tester le micro"
        meter.visibility = View.GONE
        val microphone = recorder.microphone.ifEmpty { "le micro" }
        microphoneState.text = when {
            recorder.loudest == 0f -> "⚠ Rien n'est arrivé par $microphone : vérifiez qu'il est branché et pas coupé."
            recorder.loudest < QUIET -> "⚠ Son très faible par $microphone : parlez plus près, ou choisissez un autre micro."
            else -> "✓ Votre voix arrive bien par : $microphone."
        }
    }

    @Deprecated("API Activity simple, sans AndroidX")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == MICROPHONE_REQUEST && grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) testMicrophone()
    }

    private fun refresh() {
        val view = Config.view(store) ?: return say("Réglages illisibles.")
        voiceCommands.quietly(view.getBoolean("voice_commands"))
        audioEnhance.quietly(view.optBoolean("audio_enhance", true))
        liveInsert.quietly(view.optBoolean("live_insert", false))
        val contextOn = view.optBoolean("screen_context", false)
        screenContext.quietly(contextOn)
        excludedBox.visibility = if (contextOn) View.VISIBLE else View.GONE
        val excluded = view.optJSONArray("screen_context_excluded_apps")
            ?.let { a -> List(a.length()) { a.getString(it) } }
            .orEmpty()
        excludedInput.setText(ContextLogic.formatExcludedApps(excluded))
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
        const val MICROPHONE_REQUEST = 1
        const val TEST_MS = 5_000L

        /** En dessous (crête de la voix), le micro capte à peine. */
        const val QUIET = 0.05f

        /** Modèle en cours de téléchargement (null : aucun), d'une page à l'autre. */
        @Volatile
        var downloading: String? = null
    }
}
