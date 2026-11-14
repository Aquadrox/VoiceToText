package io.github.aquadrox.voicetotext

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.content.res.ColorStateList
import android.graphics.drawable.Icon
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.Editable
import android.text.TextWatcher
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Accueil : alertes en haut (nouvelle version, bulle arrêtée, mise en route
 * inachevée), réunions, puis les réglages par catégorie, avec une recherche.
 */
class MainActivity : Activity() {
    private val store by lazy { Store(this) }
    private val main = Handler(Looper.getMainLooper())

    private lateinit var message: TextView
    private lateinit var updateBanner: LinearLayout
    private lateinit var updateBannerText: TextView
    private var bubbleBanner: View? = null
    private var bubbleBannerText: TextView? = null
    private val bubbleCheck = Runnable { if (!isDestroyed) refreshStatus() }

    private lateinit var setup: LinearLayout
    private lateinit var micStep: Step
    private lateinit var micButton: Button
    private lateinit var modelStep: Step
    private lateinit var modelButton: Button
    private lateinit var keyboardStep: Step
    private lateinit var keyboardActivate: Button
    /** Édition bulle seulement. */
    private var bubbleStep: Step? = null
    private var bubbleButtons: View? = null

    private lateinit var meetingsList: LinearLayout
    private lateinit var search: EditText
    private lateinit var results: LinearLayout
    private lateinit var categories: LinearLayout
    private val summaries = mutableMapOf<Section, TextView>()

    /** Modèle de dictée prêt (dernière lecture des réglages). */
    private var modelReady = false

    /** Vérification des mises à jour à l'ouverture. */
    private val updates = Executors.newSingleThreadExecutor()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        screen { content() }
        startUpdates()
        shortcut()
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** Notification « Installer » : la page Mises à jour lance l'installation. */
    private fun handle(intent: Intent?) {
        if (intent?.action == Updater.ACTION_INSTALL) {
            intent.action = null
            openInstall()
        }
    }

    private fun openInstall() {
        startActivity(Intent(this, UpdatesSettings::class.java).setAction(Updater.ACTION_INSTALL))
    }

    override fun onResume() {
        super.onResume()
        Updater.visible = true
        Updater.listener = { refreshUpdate() }
        // Retour d'une page de réglages : résumés et mise en route à jour.
        refresh()
        // Comme sur PC au démarrage : vérification si la dernière date d'une heure.
        val stale = System.currentTimeMillis() - store.updateCheckedAt > 60 * 60 * 1000L
        if (Updater.supported && store.updatesEnabled && stale) checkUpdate()
    }

    override fun onPause() {
        Updater.visible = false
        Updater.listener = null
        super.onPause()
    }

    private fun LinearLayout.content() {
        header("VoiceToText", "Version ${BuildConfig.VERSION_NAME} · édition ${Updater.edition}")
        note("Dictez au clavier ou avec une bulle : le texte nettoyé s'insère là où vous écrivez. La voix est transcrite sur le téléphone.")
        message = notice()

        // Section haute : ce qui demande l'attention, seulement quand c'est le cas.
        updateBanner = card("Nouvelle version", R.drawable.ic_refresh_cw).apply {
            updateBannerText = note("")
            primary("Mettre à jour") { openInstall() }
            visibility = View.GONE
        }
        if (BuildConfig.BUBBLE) {
            bubbleBanner = card("Bulle arrêtée par le téléphone", R.drawable.ic_message_circle).apply {
                bubbleBannerText = note("")
                primary("Relancer la bulle") { BubbleStatus.openSettings(this@MainActivity) }
                visibility = View.GONE
            }
        }
        setup = card("Mise en route", R.drawable.ic_list_checks).apply {
            micStep = step("Micro")
            micButton = micStep.actions.secondary("Autoriser le micro") {
                requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
            }
            modelStep = step("Modèle de transcription")
            modelButton = modelStep.actions.secondary("Choisir et télécharger") {
                Section.DICTATION.open(this@MainActivity, "model")
            }
            keyboardStep = step("Clavier VoiceToText")
            keyboardActivate = keyboardStep.actions.secondary("Activer") {
                startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
            }
            keyboardStep.actions.secondary("Choisir") {
                getSystemService(InputMethodManager::class.java).showInputMethodPicker()
            }
            if (BuildConfig.BUBBLE) {
                val bubbleRow = step("Bulle de dictée (facultative)")
                bubbleRow.actions.secondary("L'activer") { Section.BUBBLE.open(this@MainActivity, "bubble_status") }
                bubbleStep = bubbleRow
                bubbleButtons = bubbleRow.actions
            }
        }

        card("Réunions", R.drawable.ic_audio_lines).apply {
            note("Posez le téléphone sur la table, ou importez un enregistrement : transcription et compte rendu à la fin.")
            meetingsList = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(meetingsList)
            row {
                primary("Enregistrer") {
                    startActivity(Intent(this@MainActivity, MeetingsActivity::class.java).setAction(MeetingsActivity.ACTION_NEW))
                }.apply {
                    background = pressable(rounded(getColor(R.color.signal), 10f), 10f)
                    setTextColor(getColor(R.color.on_signal))
                }
                secondary("Toutes les réunions") {
                    startActivity(Intent(this@MainActivity, MeetingsActivity::class.java))
                }
            }
        }

        groupLabel("Réglages")
        search = field("Rechercher un réglage").apply {
            setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_search, 0, 0, 0)
            compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.muted))
            compoundDrawablePadding = dp(10)
            imeOptions = EditorInfo.IME_ACTION_SEARCH
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) = showResults()
            })
        }
        results = card().apply { visibility = View.GONE }
        categories = card().apply {
            setPadding(dp(12), dp(6), dp(12), dp(6))
            val list = Section.entries.filter { it != Section.MEETINGS }
            list.forEachIndexed { index, section ->
                if (index > 0) divider(0)
                summaries[section] = navRow(section.title, icon = section.icon) { section.open(this@MainActivity) }
            }
        }

        card("Essayer", R.drawable.ic_text_cursor_input).apply {
            field("Touchez ici et dictez avec le clavier VoiceToText ou la bulle", lines = 3)
        }

        // Pied de page : l'application existe aussi sur PC.
        card().apply {
            setPadding(dp(12), dp(4), dp(12), dp(4))
            navRow("VoiceToText sur PC", "La même dictée sur Windows, gratuite", R.drawable.ic_monitor) {
                Section.HELP.open(this@MainActivity, "pc")
            }
        }
    }

    private fun say(text: String) = message.say(text)

    /** Résultats de la recherche à la place des catégories. */
    private fun showResults() {
        val query = search.text.toString()
        val found = SettingsIndex.search(query, BuildConfig.BUBBLE)
        results.removeAllViews()
        val searching = query.isNotBlank()
        results.visibility = if (searching) View.VISIBLE else View.GONE
        categories.visibility = if (searching) View.GONE else View.VISIBLE
        if (!searching) return
        if (found.isEmpty()) {
            results.note("Aucun réglage ne correspond à « ${query.trim()} ». Essayez un autre mot, ou parcourez les catégories en effaçant la recherche.")
            return
        }
        results.setPadding(dp(12), dp(6), dp(12), dp(6))
        found.forEachIndexed { index, entry ->
            if (index > 0) results.divider(0)
            results.navRow(entry.title, entry.section.title, entry.section.icon) {
                entry.section.open(this, entry.key.ifEmpty { null })
            }
        }
    }

    /** Réglages relus : résumés des catégories, mise en route, réunions. */
    private fun refresh() {
        val reply = JSONObject(Native.settings(store.config, store.modelsDir))
        val view = reply.optJSONObject("ok") ?: run {
            say(reply.optString("error"))
            return
        }
        val models = view.getJSONArray("models").let { a -> List(a.length()) { a.getJSONObject(it) } }
        val chosen = models.firstOrNull { it.getString("id") == view.getString("model") }
        modelReady = chosen?.getBoolean("downloaded") == true
        // Nom court (« Parakeet TDT v3, 8 bits ») : la description est dans la page Dictée.
        val name = chosen?.getString("label")?.substringBefore(" (").orEmpty()
        modelStep.set(
            modelReady,
            when {
                chosen == null -> "Choisissez un modèle"
                modelReady -> "$name : prêt"
                else -> "$name : à télécharger"
            },
        )
        modelButton.visibility = if (modelReady) View.GONE else View.VISIBLE
        summaries[Section.DICTATION]?.summary(
            (if (modelReady) name else "$name à télécharger") + " · traduction : ${view.getString("translation")}",
        )
        summaries[Section.CLEANUP]?.summary(cleanupSummary(view))
        summaries[Section.VOCABULARY]?.summary(
            "${count(view.getInt("dictionary"), "mot", "mots")} · ${count(view.getInt("snippets"), "snippet", "snippets")}",
        )
        summaries[Section.HISTORY]?.summary(
            if (view.getBoolean("history")) "Dictées gardées sur le téléphone" else "Désactivé : rien n'est gardé",
        )
        summaries[Section.HELP]?.summary("Comment dicter · importer les réglages du PC")
        refreshStatus()
        refreshUpdate()
        refreshMeetings()
        showResults()
    }

    /** Fournisseur et modèle, ou ce qui manque pour nettoyer. */
    private fun cleanupSummary(view: JSONObject): String {
        val llm = view.getJSONObject("llm")
        if (!llm.getBoolean("enabled")) return "Désactivé : le texte brut est inséré"
        val target = JSONObject(Native.keyTarget(llm.getString("base_url"))).optJSONObject("ok")
        val providers = view.getJSONArray("providers").let { a -> List(a.length()) { a.getJSONObject(it) } }
        val label = providers.firstOrNull { it.getString("id") == target?.optString("provider") }
            ?.getString("label")?.takeIf { target?.optString("provider") != "custom" }
            ?: target?.optString("host").orEmpty()
        val host = target?.optString("host").orEmpty()
        val missingKey = host.isNotEmpty() && target?.optBoolean("optional") != true && !store.hasKey(host)
        return "$label · ${llm.getString("model")}" + if (missingKey) " · ⚠ clé manquante" else ""
    }

    private fun count(n: Int, one: String, many: String) = "$n ${if (n > 1) many else one}"

    /** Deux dernières réunions dans la carte de l'accueil. */
    private fun refreshMeetings() {
        meetingsList.removeAllViews()
        val rows = JSONObject(Native.meetings(store.meetingsDir, store.config)).optJSONArray("ok")
        for (i in 0 until minOf(2, rows?.length() ?: 0)) {
            val row = rows!!.getJSONObject(i)
            meetingsList.divider(6)
            val item = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                background = pressable(null, 8f)
                setOnClickListener { startActivity(MeetingActivity.intent(this@MainActivity, row.getString("id"))) }
            }
            item.text("${row.optString("title")} · ${Meetings.clock(row.optLong("duration_ms"))}", 15f, bold = true)
            item.text("${Meetings.date(row.getLong("started_at_ms"))} · ${Meetings.meta(row)}", 13f, R.color.muted)
            meetingsList.addView(item)
        }
    }

    /** Appui long sur l'icône : « Enregistrer une réunion ». */
    private fun shortcut() {
        runCatching {
            val intent = Intent(this, MeetingsActivity::class.java).setAction(MeetingsActivity.ACTION_NEW)
            getSystemService(ShortcutManager::class.java).dynamicShortcuts = listOf(
                ShortcutInfo.Builder(this, "meeting")
                    .setShortLabel("Réunion")
                    .setLongLabel("Enregistrer une réunion")
                    .setIcon(Icon.createWithResource(this, R.mipmap.ic_launcher))
                    .setIntent(intent)
                    .build(),
            )
        }
    }

    /** Mise en route (cachée quand tout est prêt) et état de la bulle. */
    private fun refreshStatus() {
        val mic = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        micStep.set(mic, if (mic) "Autorisé" else "Nécessaire pour dicter")
        micButton.visibility = if (mic) View.GONE else View.VISIBLE
        val keyboard = getSystemService(InputMethodManager::class.java).enabledInputMethodList
            .any { it.packageName == packageName }
        keyboardStep.set(keyboard, if (keyboard) "Activé ; « Choisir » pour passer d'un clavier à l'autre" else "À activer dans les réglages d'Android")
        keyboardActivate.visibility = if (keyboard) View.GONE else View.VISIBLE

        var bubbleRunning = false
        val bubbleStep = bubbleStep
        if (bubbleStep == null) {
            summaries[Section.BUBBLE]?.summary("Dans l'édition bulle : dicter sans changer de clavier")
        } else {
            main.removeCallbacks(bubbleCheck)
            val state = runCatching { BubbleStatus.state(this) }.getOrDefault(BubbleStatus.State.DISABLED)
            val stopped = state == BubbleStatus.State.STOPPED
            if (stopped) BubbleStatus.recordStop(this, store)
            // Application tout juste lancée : Android reconnecte peut-être encore la bulle.
            if (state == BubbleStatus.State.STARTING) main.postDelayed(bubbleCheck, BubbleStatus.SETTLE_MS)
            bubbleRunning = state == BubbleStatus.State.RUNNING
            bubbleButtons?.visibility = if (bubbleRunning || state == BubbleStatus.State.STARTING) View.GONE else View.VISIBLE
            val size = listOf("petite", "moyenne", "grande").getOrElse(store.bubbleSize) { "moyenne" }
            val text = when (state) {
                BubbleStatus.State.DISABLED -> "Pas encore activée"
                BubbleStatus.State.RUNNING -> "Activée"
                BubbleStatus.State.STARTING -> "Démarrage…"
                BubbleStatus.State.STOPPED -> "⚠ Arrêtée par le téléphone"
            }
            bubbleStep.set(bubbleRunning, text)
            summaries[Section.BUBBLE]?.summary(if (bubbleRunning) "$text · taille $size" else text)
            val last = store.bubbleStop.takeIf { it.isNotEmpty() }
            bubbleBanner?.visibility = if (stopped) View.VISIBLE else View.GONE
            bubbleBannerText?.text = "La bulle est activée, mais le téléphone a arrêté l'application et Android ne relance pas la bulle tout seul, " +
                "même quand le clavier VoiceToText refonctionne.\n\n" +
                "Pour la relancer : « Relancer la bulle », puis désactivez-la et réactivez-la. Inutile de forcer l'arrêt de l'application." +
                last?.let { "\n\nCause donnée par Android : $it." }.orEmpty() +
                "\n\nPour que cela n'arrive plus : « Comment l'activer », étape 3, dans Réglages → Bulle de dictée."
        }
        // Prêt à dicter : micro, modèle et au moins un moyen de dicter.
        setup.visibility = if (mic && modelReady && (keyboard || bubbleRunning)) View.GONE else View.VISIBLE
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

    /** Bandeau « Nouvelle version » et résumé de la catégorie Mises à jour. */
    private fun refreshUpdate() {
        val release = if (Updater.supported) Updater.cached(store) else null
        val notes = release?.let { Updater.notesText(it.changelog, BuildConfig.VERSION_NAME, it.version) }.orEmpty()
        updateBanner.visibility = if (release != null && !UpdatesSettings.installing) View.VISIBLE else View.GONE
        updateBannerText.text = release?.let {
            "VoiceToText ${it.version} est disponible. Rien n'est installé sans votre accord." + if (notes.isEmpty()) "" else "\n\n$notes"
        }.orEmpty()
        summaries[Section.UPDATES]?.summary(
            when {
                !Updater.supported -> "Version de test ${BuildConfig.VERSION_NAME} : pas de mise à jour automatique"
                release != null -> "Nouvelle version ${release.version} disponible"
                store.updateCheckedAt != 0L -> "Version ${BuildConfig.VERSION_NAME} · à jour"
                else -> "Version ${BuildConfig.VERSION_NAME}"
            },
        )
    }

    /** Vérification discrète à l'ouverture (un échec, hors ligne…, ne s'affiche pas). */
    private fun checkUpdate() {
        if (UpdatesSettings.checking || UpdatesSettings.installing) return
        UpdatesSettings.checking = true
        updates.execute {
            runCatching { Updater.check(this, notify = false) }
            main.post {
                UpdatesSettings.checking = false
                if (!isDestroyed) refreshUpdate()
            }
        }
    }

    @Deprecated("API Activity simple, sans AndroidX")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        refreshStatus()
    }

    override fun onDestroy() {
        main.removeCallbacks(bubbleCheck)
        updates.shutdown()
        super.onDestroy()
    }

    private companion object {
        const val NOTIFICATIONS = 3
    }
}
