package io.github.aquadrox.voicetotext

import android.app.NotificationManager
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.Executors

/**
 * Mises à jour : vérification, téléchargement vérifié et installation (sur
 * appui seulement), nouveautés de chaque version.
 */
class UpdatesSettings : SettingsPage() {
    override val section = Section.UPDATES
    override val subtitle = "Version ${BuildConfig.VERSION_NAME} · édition ${Updater.edition}"

    private lateinit var status: TextView
    private lateinit var install: Button
    private lateinit var changelog: TextView
    private lateinit var notes: Button
    private lateinit var check: Button
    private lateinit var progress: ProgressBar
    private lateinit var notificationsButton: Button

    /** Téléchargement et vérification de la mise à jour. */
    private val updates = Executors.newSingleThreadExecutor()

    /**
     * Autorisation « applis inconnues » demandée : l'écran des réglages
     * d'Android s'ouvre (pause), l'installation reprend au retour (reprise).
     */
    private var askedInstallPermission = false
    private var pendingInstall = false

    override fun LinearLayout.build() {
        card("Version", R.drawable.ic_refresh_cw).anchor("updates_check").apply {
            status = note("")
            if (Updater.supported) {
                install = primary("Télécharger et installer") { installUpdate() }
                progress = ProgressBar(this@UpdatesSettings, null, android.R.attr.progressBarStyleHorizontal).apply {
                    max = 100
                    visibility = View.GONE
                }.also { addView(it) }
                changelog = note("").apply { visibility = View.GONE }
                notes = secondary("Page de la version sur GitHub") {
                    Updater.cached(store)?.notes?.takeIf { it.isNotEmpty() }?.let {
                        startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it)))
                    }
                }
                check = secondary("Vérifier maintenant") { checkUpdate(manual = true) }
            }
        }

        if (Updater.supported) {
            card("Vérification", R.drawable.ic_list_checks).anchor("updates_auto").apply {
                toggle(
                    "Vérifier automatiquement",
                    "À l'ouverture puis toutes les 6 h. Une notification prévient d'une nouvelle version ; rien n'est installé sans votre accord.",
                    store.updatesEnabled,
                ) {
                    store.updatesEnabled = it
                    Updater.schedule(this@UpdatesSettings)
                    refresh()
                    saved(if (it) "✓ Vérification automatique activée" else "✓ Vérification automatique coupée : pensez à « Vérifier maintenant »")
                }
                notificationsButton = secondary("Autoriser les notifications") {
                    startActivity(
                        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                            .putExtra(Settings.EXTRA_APP_PACKAGE, packageName),
                    )
                }
                more(
                    "Android vérifie que la mise à jour est signée avec la même clé que l'application installée, et refuse une version plus ancienne ; l'application vérifie en plus l'empreinte SHA-256 publiée avec la version. La première fois, Android demande d'autoriser VoiceToText à installer des applications. Vos réglages, modèles et clés sont gardés." +
                        if (BuildConfig.BUBBLE) "\n\nÉdition bulle : si Play Protect bloque la mise à jour, coupez-le le temps de l'installation puis réactivez-le, comme à la première installation." else "",
                    "Sécurité des mises à jour",
                )
            }
        }

        card("Nouveautés", R.drawable.ic_list_checks).anchor("updates_notes").apply {
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
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** Notification ou bandeau « Installer » : installation lancée aussitôt. */
    private fun handle(intent: Intent?) {
        if (intent?.action == Updater.ACTION_INSTALL) {
            intent.action = null
            installUpdate()
        }
    }

    override fun onResume() {
        super.onResume()
        Updater.visible = true
        Updater.listener = { refresh() }
        // Retour de l'autorisation « applis inconnues » : l'installation reprend.
        if (pendingInstall) {
            pendingInstall = false
            if (packageManager.canRequestPackageInstalls()) installUpdate()
            else store.updateError = "Mise à jour non installée : VoiceToText n'est pas autorisé à installer des applications."
        }
        refresh()
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

    /** État d'après la dernière vérification. */
    private fun refresh() {
        val installed = "Version installée : ${BuildConfig.VERSION_NAME} (édition ${Updater.edition})."
        if (!Updater.supported) {
            status.text = "$installed\nVersion de test : pas de mise à jour automatique (installez l'APK d'une release)."
            return
        }
        val release = Updater.cached(store)
        // Nouveautés de la version proposée (et des versions sautées).
        val text = release?.let { Updater.notesText(it.changelog, BuildConfig.VERSION_NAME, it.version) }.orEmpty()
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
        status.text = "$installed\nDernière vérification : $checked.\n$state$error".trimEnd()
        install.visibility = if (release != null) View.VISIBLE else View.GONE
        install.text = release?.let { "Télécharger et installer la ${it.version}" } ?: "Télécharger et installer"
        install.isEnabled = !installing
        changelog.text = if (text.isEmpty()) "" else "Nouveautés :\n\n$text"
        changelog.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
        notes.visibility = if (release?.notes?.isNotEmpty() == true) View.VISIBLE else View.GONE
        check.isEnabled = !checking && !installing
        progress.visibility = if (installing) View.VISIBLE else View.GONE
        val notifications = getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        notificationsButton.visibility = if (store.updatesEnabled && !notifications) View.VISIBLE else View.GONE
    }

    private fun checkUpdate(manual: Boolean) {
        if (checking || installing) return
        checking = true
        if (manual) store.updateError = ""
        refresh()
        updates.execute {
            val result = runCatching { Updater.check(this, notify = false) }
            main.post {
                checking = false
                result.exceptionOrNull()?.let { if (manual) store.updateError = it.message ?: it.toString() }
                if (!isDestroyed) refresh()
            }
        }
    }

    /** « Installer » : autorisation si besoin, téléchargement vérifié, installateur d'Android. */
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
        progress.progress = 0
        refresh()
        val poll = object : Runnable {
            override fun run() {
                if (isDestroyed) return
                progress.progress = Updater.progress.get()
                if (installing) main.postDelayed(this, 300)
            }
        }
        main.post(poll)
        updates.execute {
            val result = runCatching { Updater.install(this, Updater.download(this, release)) }
            main.post {
                installing = false
                result.exceptionOrNull()?.let { store.updateError = "Mise à jour non installée : ${it.message ?: it}" }
                if (isDestroyed) return@post
                if (result.isSuccess) say("Installation de la ${release.version} en cours : Android ferme VoiceToText le temps de la mise à jour ; une notification la rouvre ensuite.")
                refresh()
            }
        }
    }

    override fun onDestroy() {
        updates.shutdown()
        super.onDestroy()
    }

    companion object {
        /** Partagés avec l'accueil : une seule vérification ou installation à la fois. */
        @Volatile
        var checking = false

        @Volatile
        var installing = false
    }
}
