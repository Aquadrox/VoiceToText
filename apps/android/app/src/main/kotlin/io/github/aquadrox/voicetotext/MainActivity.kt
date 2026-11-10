package io.github.aquadrox.voicetotext

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.graphics.Typeface
import android.text.InputType
import android.util.TypedValue
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ArrayAdapter
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import org.json.JSONObject
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.Executors

/**
 * Réglages et mise en route : micro, modèle Whisper, clavier, nettoyage LLM,
 * import des réglages du PC, zone d'essai.
 */
class MainActivity : Activity() {
    private val store by lazy { Store(this) }
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    private lateinit var micStep: Step
    private lateinit var micButton: Button
    private lateinit var modelStep: Step
    private lateinit var keyboardStep: Step
    private lateinit var keyboardActivate: Button
    /** Édition bulle seulement. */
    private var bubbleStep: Step? = null
    private var bubbleButtons: View? = null
    private var bubbleBanner: View? = null
    private var bubbleBannerText: TextView? = null
    private var bubbleStop: TextView? = null
    private val bubbleCheck = Runnable { if (!isDestroyed) refreshStatus() }
    private lateinit var models: RadioGroup
    private lateinit var download: Button
    private lateinit var progress: ProgressBar
    private lateinit var llmEnabled: Switch
    private lateinit var baseUrl: EditText
    private lateinit var llmModel: EditText
    private lateinit var apiKey: EditText
    private lateinit var keyStatus: TextView
    private lateinit var voiceCommands: Switch
    private lateinit var language: Spinner
    private lateinit var history: Switch
    private lateinit var vocabularyStatus: TextView
    private lateinit var promptsStatus: TextView
    private lateinit var message: TextView
    private lateinit var updateBanner: LinearLayout
    private lateinit var updateBannerText: TextView
    private lateinit var updateStatus: TextView
    private lateinit var updateInstall: Button
    private lateinit var updateChangelog: TextView
    private lateinit var updateNotes: Button
    private lateinit var updateCheck: Button
    private lateinit var updateProgress: ProgressBar
    private lateinit var notificationsButton: Button

    /** Téléchargement et vérification de la mise à jour, à part du modèle. */
    private val updates = Executors.newSingleThreadExecutor()
    private var installing = false
    private var checking = false

    /**
     * Autorisation « applis inconnues » demandée : l'écran des réglages
     * d'Android s'ouvre (pause), l'installation reprend au retour (reprise).
     */
    private var askedInstallPermission = false
    private var pendingInstall = false

    /** Hôte de l'API affiché (la clé est rangée par hôte). */
    private var host = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        screen(bar = { primary("Enregistrer les réglages") { save() } }) { content() }
        refresh()
        startUpdates()
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** Notification « Installer » : installation lancée aussitôt. */
    private fun handle(intent: Intent?) {
        if (intent?.action == Updater.ACTION_INSTALL) {
            intent.action = null
            installUpdate()
        }
    }

    override fun onResume() {
        super.onResume()
        Updater.visible = true
        Updater.listener = { refreshUpdate() }
        // Retour du dictionnaire ou des prompts : compteurs à jour.
        if (::message.isInitialized) refresh()
        // Retour de l'autorisation « applis inconnues » : l'installation reprend.
        if (pendingInstall) {
            pendingInstall = false
            if (packageManager.canRequestPackageInstalls()) installUpdate()
            else store.updateError = "Mise à jour non installée : VoiceToText n'est pas autorisé à installer des applications."
        }
        refreshUpdate()
        // Comme sur PC au démarrage : vérification si la dernière date d'une heure.
        val stale = System.currentTimeMillis() - store.updateCheckedAt > 60 * 60 * 1000L
        if (Updater.supported && store.updatesEnabled && stale) checkUpdate(manual = false)
    }

    override fun onPause() {
        if (askedInstallPermission) {
            askedInstallPermission = false
            pendingInstall = true
        }
        Updater.visible = false
        Updater.listener = null
        super.onPause()
    }

    private fun LinearLayout.content() {
        header("VoiceToText", "Version ${BuildConfig.VERSION_NAME} · édition ${Updater.edition}")
        note("Dictez au clavier ou avec une bulle : le texte nettoyé s'insère là où vous écrivez. La voix est transcrite sur le téléphone.")
        message = notice()
        updateBanner = card("Nouvelle version", R.drawable.ic_refresh_cw).apply {
            updateBannerText = note("")
            primary("Mettre à jour") { installUpdate() }
            visibility = View.GONE
        }
        if (BuildConfig.BUBBLE) {
            bubbleBanner = card("Bulle arrêtée par le téléphone", R.drawable.ic_message_circle).apply {
                bubbleBannerText = note("")
                primary("Relancer la bulle") { BubbleStatus.openSettings(this@MainActivity) }
                visibility = View.GONE
            }
        }

        card("Mise en route", R.drawable.ic_list_checks).apply {
            micStep = step("Micro")
            micButton = micStep.actions.secondary("Autoriser le micro") {
                requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
            }
            modelStep = step("Modèle de transcription")
            keyboardStep = step("Clavier VoiceToText")
            keyboardActivate = keyboardStep.actions.secondary("Activer") {
                startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
            }
            keyboardStep.actions.secondary("Choisir") {
                getSystemService(InputMethodManager::class.java).showInputMethodPicker()
            }
            if (BuildConfig.BUBBLE) {
                val bubbleRow = step("Bulle de dictée")
                bubbleRow.actions.secondary("Infos de l'appli") {
                    startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
                }
                bubbleRow.actions.secondary("Accessibilité") { BubbleStatus.openSettings(this@MainActivity) }
                bubbleStep = bubbleRow
                bubbleButtons = bubbleRow.actions
            }
        }

        card("Modèle de transcription", R.drawable.ic_audio_lines).apply {
            note("La voix est transcrite sur le téléphone ; le modèle choisi se télécharge une seule fois.")
            models = RadioGroup(this@MainActivity).also { addView(it) }
            progress = ProgressBar(this@MainActivity, null, android.R.attr.progressBarStyleHorizontal).apply {
                max = 100
                visibility = View.GONE
            }.also { addView(it) }
            download = primary("Télécharger le modèle choisi") { downloadModel() }
        }

        card("Comment dicter", R.drawable.ic_mic).apply {
            note("Clavier : maintenez le micro pour dicter, relâchez pour insérer. Appui court : mains libres, nouvel appui pour finir.")
            note("À droite du micro, la touche consigne (maintenue) : la consigne dictée s'applique au texte sélectionné (« rends ce texte plus formel », « traduis en anglais »).")
            note("À gauche du micro : le mode traduction. Tout à gauche : retour à votre clavier habituel.")
            if (BuildConfig.BUBBLE) {
                note("Bulle : toucher pour dicter, toucher encore pour insérer. Maintenir : consigne sur le texte sélectionné. Glisser : déplacer. Double appui : passer au clavier VoiceToText (Android 11 et plus).")
            }
        }

        card("Bulle de dictée", R.drawable.ic_message_circle).apply {
            if (BuildConfig.BUBBLE) {
                bubbleOptions()
            } else {
                note("Cette édition n'a que le clavier. Pour dicter avec une bulle flottante sans changer de clavier, installez l'APK « android-bulle » de la même version : même application, réglages gardés (voir le README).")
            }
        }

        card("Nettoyage du texte (LLM)", R.drawable.ic_spell_check_2).apply {
            llmEnabled = option("Nettoyer et ponctuer avec un LLM")
            baseUrl = field("Adresse de l'API (ex. https://api.deepseek.com)")
            llmModel = field("Modèle (ex. deepseek-flash)")
            apiKey = field("Nouvelle clé API (jamais affichée)").apply {
                inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                // Android passe les champs de mot de passe en police à chasse fixe.
                typeface = Typeface.DEFAULT
            }
            keyStatus = note("")
            secondary("Supprimer la clé") {
                if (host.isNotEmpty()) store.deleteKey(host)
                refresh()
                say("Clé supprimée.")
            }
            divider()
            promptsStatus = note("")
            secondary("Prompts système") {
                startActivity(Intent(this@MainActivity, PromptsActivity::class.java))
            }
            divider()
            voiceCommands = option("Commandes vocales (« nouvelle ligne », « annule la dernière dictée »)")
        }

        card("Traduction", R.drawable.ic_languages).apply {
            note("Langue d'arrivée du mode traduction (touche à gauche du micro, ou réglage de la bulle) :")
            language = Spinner(this@MainActivity).also { addView(it) }
        }

        card("Historique et dictionnaire", R.drawable.ic_history).apply {
            history = option("Garder les dictées sur le téléphone")
            secondary("Voir l'historique") {
                startActivity(Intent(this@MainActivity, HistoryActivity::class.java))
            }
            divider()
            vocabularyStatus = note("")
            secondary("Dictionnaire et snippets") {
                startActivity(Intent(this@MainActivity, VocabularyActivity::class.java))
            }
        }

        card("Réglages du PC", R.drawable.ic_import).apply {
            note("Sur PC : Réglages → Exporter… puis copiez le fichier sur le téléphone. Dictionnaire, snippets, styles et fournisseur LLM sont repris ; les clés API ne sont jamais exportées.")
            secondary("Importer un fichier de réglages…") {
                startActivityForResult(
                    Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE)
                        type = "*/*"
                    },
                    IMPORT,
                )
            }
        }

        card("Mises à jour", R.drawable.ic_refresh_cw).apply {
            updateStatus = note("")
            if (Updater.supported) {
                option("Vérifier automatiquement (à l'ouverture puis toutes les 6 h)", store.updatesEnabled) {
                    store.updatesEnabled = it
                    Updater.schedule(this@MainActivity)
                }
                updateInstall = primary("Télécharger et installer") { installUpdate() }
                updateCheck = secondary("Vérifier maintenant") { checkUpdate(manual = true) }
                updateProgress = ProgressBar(this@MainActivity, null, android.R.attr.progressBarStyleHorizontal).apply {
                    max = 100
                    visibility = View.GONE
                }.also { addView(it) }
                updateChangelog = note("").apply { visibility = View.GONE }
                updateNotes = secondary("Page de la version sur GitHub") {
                    Updater.cached(store)?.notes?.takeIf { it.isNotEmpty() }?.let {
                        startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it)))
                    }
                }
                notificationsButton = secondary("Autoriser les notifications") {
                    startActivity(
                        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                            .putExtra(Settings.EXTRA_APP_PACKAGE, packageName),
                    )
                }
            }
            // Nouveautés des versions installées, consultables après la mise à jour.
            lateinit var versions: TextView
            secondary("Nouveautés de chaque version") {
                if (versions.text.isEmpty()) {
                    versions.text = Updater.notesText("", "", BuildConfig.VERSION_NAME)
                        .ifEmpty { "Aucune note de version." }
                }
                versions.visibility = if (versions.visibility == View.VISIBLE) View.GONE else View.VISIBLE
            }
            versions = note("").apply { visibility = View.GONE }
            if (Updater.supported) {
                more(
                    "Rien n'est installé sans votre accord. Android vérifie que la mise à jour est signée avec la même clé que l'application installée, et refuse une version plus ancienne ; l'application vérifie en plus l'empreinte SHA-256 publiée avec la version. La première fois, Android demande d'autoriser VoiceToText à installer des applications. Vos réglages, modèles et clés sont gardés." +
                        if (BuildConfig.BUBBLE) "\n\nÉdition bulle : si Play Protect bloque la mise à jour, coupez-le le temps de l'installation puis réactivez-le, comme à la première installation." else "",
                )
            }
        }

        card("Essayer", R.drawable.ic_text_cursor_input).apply {
            field("Touchez ici et dictez avec le clavier VoiceToText ou la bulle", lines = 3)
        }
    }

    /** Édition « bulle » : réglages de la bulle (l'activation est dans « Mise en route »). */
    private fun LinearLayout.bubbleOptions() {
        note("Gardez votre clavier habituel : une bulle apparaît quand vous écrivez. Clavier ou bulle, ou les deux : au choix.")
        more(
            "Activation (une seule fois) :\n1. Android 13 et plus : Infos de l'application → ⋮ (en haut à droite) → « Autoriser les paramètres restreints » (obligatoire pour une application installée hors Play Store).\n2. Accessibilité → Applications installées → « VoiceToText : bulle de dictée » → activer.\n3. Xiaomi, Samsung… : dans Infos de l'application, autorisez le démarrage automatique et mettez la batterie sur « Pas de restriction » ; dans les applications récentes, verrouillez VoiceToText (appui long sur sa carte → cadenas). Sinon le téléphone arrête la bulle, et Android ne la relance pas seul.\n\nBulle arrêtée : désactivez-la puis réactivez-la dans l'accessibilité. Inutile de forcer l'arrêt de l'application.",
            "Comment l'activer",
        )
        bubbleStop = note("").apply { visibility = View.GONE }
        option("Seulement quand le clavier est ouvert", store.bubbleWithKeyboard) { store.bubbleWithKeyboard = it }
        option("Insérer en collant (si le texte n'apparaît pas dans une application ; remplace le presse-papier)", store.bubblePaste) {
            store.bubblePaste = it
        }
        option("Traduire les dictées (comme le mode traduction du clavier)", store.translate) { store.translate = it }
        option("Garder les dictées de la bulle dans l'historique", store.bubbleHistory) { store.bubbleHistory = it }
        text("Taille de la bulle", 15f).setPadding(0, dp(10), 0, 0)
        addView(RadioGroup(this@MainActivity).apply {
            orientation = RadioGroup.HORIZONTAL
            listOf("Petite", "Moyenne", "Grande").forEachIndexed { size, label ->
                addView(RadioButton(this@MainActivity).apply {
                    id = View.generateViewId()
                    text = label
                    setTextColor(getColor(R.color.fg))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                    setPadding(dp(4), 0, dp(16), 0)
                    isChecked = size == store.bubbleSize
                    setOnCheckedChangeListener { _, checked -> if (checked) store.bubbleSize = size }
                })
            }
        })
        secondary("Remettre la bulle à sa place") {
            store.bubbleX = -1
            store.bubbleY = -1
            say("✓ La bulle reprendra sa place à sa prochaine apparition.")
        }
        note("La bulle n'apparaît jamais sur un champ de mot de passe. Contrairement au clavier, elle ne voit pas la navigation privée : décochez « Garder les dictées de la bulle » pour qu'aucune n'aille dans l'historique.")
    }

    private fun say(text: String) = message.say(text)

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
        history.isChecked = view.getBoolean("history")
        vocabularyStatus.text = "${view.getInt("dictionary")} mot(s), ${view.getInt("snippets")} snippet(s)"
        val prompt = { name: String -> if (name.isEmpty()) "intégré" else "« $name »" }
        promptsStatus.text = "Prompt de nettoyage : ${prompt(view.getString("cleanup_prompt"))} · " +
            "Command Mode : ${prompt(view.getString("command_prompt"))}"
        val languages = view.getJSONArray("languages").let { array -> List(array.length()) { array.getString(it) } }
            .let { list -> if (view.getString("translation") in list) list else list + view.getString("translation") }
        language.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_dropdown_item, languages)
        language.setSelection(languages.indexOf(view.getString("translation")))

        models.removeAllViews()
        val list = view.getJSONArray("models")
        for (i in 0 until list.length()) {
            val model = list.getJSONObject(i)
            val downloaded = if (model.getBoolean("downloaded")) " · téléchargé" else ""
            models.addView(RadioButton(this).apply {
                id = View.generateViewId()
                tag = model.getString("id")
                text = "${model.getString("label")} · ${model.getInt("size_mb")} Mo$downloaded"
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setPadding(dp(4), dp(8), 0, dp(8))
                isChecked = model.getString("id") == view.getString("model")
            })
        }
        val chosen = (0 until list.length()).map { list.getJSONObject(it) }
            .firstOrNull { it.getString("id") == view.getString("model") }
        val ready = chosen?.getBoolean("downloaded") == true
        // Nom court (« Parakeet TDT v3, 8 bits ») : la description est dans la carte Modèle.
        val name = chosen?.getString("label")?.substringBefore(" (").orEmpty()
        modelStep.set(
            ready,
            when {
                chosen == null -> "Choisissez un modèle ci-dessous"
                ready -> "$name : prêt"
                else -> "$name : à télécharger ci-dessous"
            },
        )
        refreshStatus()
    }

    private fun refreshStatus() {
        val mic = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        micStep.set(mic, if (mic) "Autorisé" else "Nécessaire pour dicter")
        micButton.visibility = if (mic) View.GONE else View.VISIBLE
        val enabled = getSystemService(InputMethodManager::class.java).enabledInputMethodList
            .any { it.packageName == packageName }
        keyboardStep.set(enabled, if (enabled) "Activé ; « Choisir » pour passer d'un clavier à l'autre" else "À activer dans les réglages d'Android")
        keyboardActivate.visibility = if (enabled) View.GONE else View.VISIBLE
        val bubbleStep = bubbleStep ?: return
        main.removeCallbacks(bubbleCheck)
        val state = runCatching { BubbleStatus.state(this) }.getOrDefault(BubbleStatus.State.DISABLED)
        val stopped = state == BubbleStatus.State.STOPPED
        if (stopped) BubbleStatus.recordStop(this, store)
        // Application tout juste lancée : Android reconnecte peut-être encore la bulle.
        if (state == BubbleStatus.State.STARTING) main.postDelayed(bubbleCheck, BubbleStatus.SETTLE_MS)
        val running = state == BubbleStatus.State.RUNNING
        bubbleButtons?.visibility = if (running || state == BubbleStatus.State.STARTING) View.GONE else View.VISIBLE
        bubbleStep.set(running, when (state) {
            BubbleStatus.State.DISABLED -> "À activer : « Comment l'activer » dans la carte Bulle"
            BubbleStatus.State.RUNNING -> "Activée"
            BubbleStatus.State.STARTING -> "Démarrage…"
            BubbleStatus.State.STOPPED -> "Activée mais arrêtée par le téléphone : voir en haut de l'écran"
        })
        val last = store.bubbleStop.takeIf { it.isNotEmpty() }
        bubbleBanner?.visibility = if (stopped) View.VISIBLE else View.GONE
        bubbleBannerText?.text = "La bulle est activée, mais le téléphone a arrêté l'application et Android ne relance pas la bulle tout seul, " +
            "même quand le clavier VoiceToText refonctionne.\n\n" +
            "Pour la relancer : « Relancer la bulle », puis désactivez-la et réactivez-la. Inutile de forcer l'arrêt de l'application." +
            last?.let { "\n\nCause donnée par Android : $it." }.orEmpty() +
            "\n\nPour que cela n'arrive plus : étape 3 de « Comment l'activer », dans la carte Bulle."
        bubbleStop?.apply {
            text = last?.let { "Dernier arrêt de la bulle par le téléphone : $it." }.orEmpty()
            visibility = if (last == null) View.GONE else View.VISIBLE
        }
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
            .put("translation", language.selectedItem?.toString() ?: "anglais")
            .put("history", history.isChecked)
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

    /** Au lancement : « mis à jour », ancien paquet effacé, vérification planifiée, notifications. */
    private fun startUpdates() {
        val previous = store.lastRunVersion
        if (previous != BuildConfig.VERSION_NAME) {
            store.lastRunVersion = BuildConfig.VERSION_NAME
            store.updateError = ""
            if (previous.isNotEmpty()) {
                val notes = Updater.notesText("", previous, BuildConfig.VERSION_NAME)
                say(
                    "✓ VoiceToText mis à jour : version ${BuildConfig.VERSION_NAME}." +
                        if (notes.isEmpty()) "" else "\n\nNouveautés :\n\n$notes",
                )
            }
        }
        java.io.File(cacheDir, "update").deleteRecursively()
        if (!Updater.supported) return
        runCatching { Updater.schedule(this) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && store.updatesEnabled && !store.askedNotifications &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            store.askedNotifications = true
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS)
        }
    }

    /** Bandeau et section « Mises à jour » d'après la dernière vérification. */
    private fun refreshUpdate() {
        if (!::updateStatus.isInitialized) return
        val installed = "Version installée : ${BuildConfig.VERSION_NAME} (édition ${Updater.edition})."
        if (!Updater.supported) {
            updateStatus.text = "$installed\nVersion de test : pas de mise à jour automatique (installez l'APK d'une release)."
            return
        }
        val release = Updater.cached(store)
        // Nouveautés de la version proposée (et des versions sautées).
        val notes = release?.let { Updater.notesText(it.changelog, BuildConfig.VERSION_NAME, it.version) }.orEmpty()
        updateBanner.visibility = if (release != null && !installing) View.VISIBLE else View.GONE
        updateBannerText.text = release?.let {
            "VoiceToText ${it.version} est disponible." + if (notes.isEmpty()) "" else "\n\n$notes"
        }.orEmpty()
        val checkedAt = store.updateCheckedAt
        val checked = if (checkedAt == 0L) "jamais" else
            DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(checkedAt))
        val state = when {
            installing -> "Téléchargement et vérification de la ${release?.version ?: "nouvelle version"}…"
            checking -> "Vérification…"
            release != null -> "Nouvelle version disponible : ${release.version} (${release.size / (1024 * 1024)} Mo)."
            checkedAt != 0L -> "✓ VoiceToText est à jour."
            else -> ""
        }
        val error = store.updateError.takeIf { it.isNotEmpty() }?.let { "\n⚠ $it" }.orEmpty()
        updateStatus.text = "$installed\nDernière vérification : $checked.\n$state$error".trimEnd()
        updateInstall.visibility = if (release != null) View.VISIBLE else View.GONE
        updateInstall.text = release?.let { "Télécharger et installer la ${it.version}" } ?: "Télécharger et installer"
        updateInstall.isEnabled = !installing
        updateChangelog.text = if (notes.isEmpty()) "" else "Nouveautés :\n\n$notes"
        updateChangelog.visibility = if (notes.isEmpty()) View.GONE else View.VISIBLE
        updateNotes.visibility = if (release?.notes?.isNotEmpty() == true) View.VISIBLE else View.GONE
        updateCheck.isEnabled = !checking && !installing
        updateProgress.visibility = if (installing) View.VISIBLE else View.GONE
        val notifications = getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        notificationsButton.visibility = if (store.updatesEnabled && !notifications) View.VISIBLE else View.GONE
    }

    private fun checkUpdate(manual: Boolean) {
        if (checking || installing) return
        checking = true
        if (manual) store.updateError = ""
        refreshUpdate()
        updates.execute {
            val result = runCatching { Updater.check(this, notify = false) }
            main.post {
                checking = false
                // Vérification automatique : un échec (hors ligne…) reste discret.
                result.exceptionOrNull()?.let { if (manual) store.updateError = it.message ?: it.toString() }
                refreshUpdate()
            }
        }
    }

    /** Clic « Installer » : autorisation si besoin, téléchargement vérifié, installateur d'Android. */
    private fun installUpdate() {
        if (!Updater.supported || installing) return
        val release = Updater.cached(store) ?: run {
            checkUpdate(manual = true)
            return
        }
        if (!packageManager.canRequestPackageInstalls()) {
            askedInstallPermission = true
            say("Autorisez VoiceToText à installer des applications, puis revenez : la mise à jour continuera.")
            startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
            return
        }
        installing = true
        store.updateError = ""
        updateProgress.progress = 0
        refreshUpdate()
        val poll = object : Runnable {
            override fun run() {
                updateProgress.progress = Updater.progress.get()
                if (installing) main.postDelayed(this, 300)
            }
        }
        main.post(poll)
        updates.execute {
            val result = runCatching { Updater.install(this, Updater.download(this, release)) }
            main.post {
                installing = false
                result.exceptionOrNull()?.let { store.updateError = "Mise à jour non installée : ${it.message ?: it}" }
                if (result.isSuccess) say("Installation de la ${release.version} en cours : Android ferme VoiceToText le temps de la mise à jour ; une notification la rouvre ensuite.")
                refreshUpdate()
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
        refreshUpdate()
    }

    override fun onDestroy() {
        main.removeCallbacks(bubbleCheck)
        worker.shutdown()
        updates.shutdown()
        super.onDestroy()
    }

    private companion object {
        const val IMPORT = 2
        const val NOTIFICATIONS = 3
    }
}
