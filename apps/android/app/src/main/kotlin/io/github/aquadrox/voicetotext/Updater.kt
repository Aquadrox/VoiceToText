package io.github.aquadrox.voicetotext

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicInteger

/**
 * Mises à jour de l'application, comme sur PC : vérification au lancement
 * puis toutes les 6 h, notification une fois par version, installation sur
 * clic seulement (écran de confirmation d'Android, sauf si le téléphone
 * permet à l'application de se mettre à jour elle-même sans le demander).
 *
 * Sécurité, en plus des contrôles d'Android (une mise à jour doit être
 * signée avec la même clé que l'application en place, celle du projet, et
 * une version plus ancienne est refusée) : adresse limitée aux Releases du
 * dépôt, taille et SHA-256 publiés dans le manifeste vérifiés, paquet relu
 * avant l'installation (identifiant, version, signature, édition).
 */
object Updater {
    /** Manifeste publié par la CI à chaque tag (à part de celui du PC, lu par Tauri). */
    private const val MANIFEST =
        "https://github.com/Aquadrox/VoiceToText/releases/latest/download/latest-android.json"
    private const val DOWNLOADS = "https://github.com/Aquadrox/VoiceToText/releases/download/"
    private const val NOTES = "https://github.com/Aquadrox/VoiceToText/releases/"
    const val INTERVAL_MS = 6 * 60 * 60 * 1000L
    private const val MAX_BYTES = 1024L * 1024 * 1024
    /** Notes de version gardées avec la release (le manifeste fait moins de 64 Kio). */
    private const val MAX_CHANGELOG = 48 * 1024
    private const val JOB_ID = 6001
    private const val CHANNEL = "updates"
    private const val NOTIFY_AVAILABLE = 6002
    private const val NOTIFY_CONFIRM = 6003
    const val ACTION_INSTALL = "io.github.aquadrox.voicetotext.INSTALL_UPDATE"

    /** Version de test (PR) : autre identifiant et autre signature, pas de mise à jour. */
    val supported: Boolean
        get() = !BuildConfig.DEBUG

    /** Entrée du manifeste : on reste sur l'édition installée (la bulle garde sa bulle). */
    val platform: String
        get() = if (BuildConfig.BUBBLE) "android-bulle" else "android-clavier"

    val edition: String
        get() = if (BuildConfig.BUBBLE) "bulle" else "clavier"

    /** Avancement du téléchargement, en %, lu par l'écran. */
    val progress = AtomicInteger(0)

    /** Écran principal affiché : la confirmation d'Android peut s'ouvrir directement. */
    @Volatile
    var visible = false

    /** Prévenu (fil principal) quand l'installation échoue ou attend une confirmation. */
    var listener: (() -> Unit)? = null

    class Release(
        val version: String,
        val code: Long,
        val url: String,
        val sha256: String,
        val size: Long,
        /** Page de la release sur GitHub. */
        val notes: String,
        /** Notes de version (CHANGELOG.md) publiées avec le manifeste ; vide si absentes. */
        val changelog: String,
    ) {
        fun toJson(): String = JSONObject()
            .put("version", version)
            .put("url", url)
            .put("sha256", sha256)
            .put("size", size)
            .put("notes", notes)
            .put("changelog", changelog)
            .toString()
    }

    /** « 0.21.0 » → 2100, comme `versionCode` dans build.gradle.kts. */
    fun code(version: String): Long? {
        val parts = version.trim().split('.')
        if (parts.size != 3) return null
        val (major, minor, patch) = parts.map { it.toIntOrNull() ?: return null }
        if (major !in 0..20_000 || minor !in 0..99 || patch !in 0..99) return null
        return major * 10_000L + minor * 100L + patch
    }

    /** Release en cache (dernière vérification), relue avec les mêmes contrôles. */
    fun cached(store: Store): Release? {
        val text = store.updateRelease.takeIf { it.isNotEmpty() } ?: return null
        return runCatching {
            val json = JSONObject(text)
            release(json, json.getString("version"), json.optString("notes"), json.optString("changelog"))
        }.getOrNull()?.takeIf { it.code > BuildConfig.VERSION_CODE }
    }

    private fun release(entry: JSONObject, version: String, notes: String, changelog: String): Release {
        val code = code(version) ?: error("Numéro de version illisible : $version")
        val url = entry.getString("url")
        require(url.startsWith(DOWNLOADS) && !url.contains("..")) { "Adresse de téléchargement inattendue." }
        val sha256 = entry.getString("sha256").lowercase()
        require(sha256.length == 64 && sha256.all { it in '0'..'9' || it in 'a'..'f' }) { "Empreinte SHA-256 invalide." }
        val size = entry.getLong("size")
        require(size in 1..MAX_BYTES) { "Taille annoncée invalide." }
        return Release(
            version,
            code,
            url,
            sha256,
            size,
            notes.takeIf { it.startsWith(NOTES) }.orEmpty(),
            changelog.take(MAX_CHANGELOG),
        )
    }

    /**
     * Notes de version lisibles (« Version 0.22.0 — 8 novembre 2026 » puis une
     * ligne « • » par changement), lues par le moteur Rust comme sur PC.
     * `changelog` vide : notes embarquées dans l'application ; bornes vides :
     * sans limite. Vide s'il n'y a rien à montrer.
     */
    fun notesText(changelog: String, after: String, upTo: String): String {
        val releases = runCatching {
            JSONObject(Native.releaseNotes(changelog, after, upTo)).optJSONArray("ok")
        }.getOrNull() ?: return ""
        return (0 until releases.length()).joinToString("\n\n") { index ->
            val release = releases.getJSONObject(index)
            val date = release.optString("date").takeIf { it.isNotEmpty() }?.let { " — $it" }.orEmpty()
            val items = release.optJSONArray("items") ?: JSONArray()
            val lines = (0 until items.length()).joinToString("\n") { "• ${items.getString(it)}" }
            "Version ${release.getString("version")}$date\n$lines"
        }
    }

    private fun open(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = true
            setRequestProperty("User-Agent", "VoiceToText-Android/${BuildConfig.VERSION_NAME}")
        }

    /** Dernière version publiée pour cette édition, ou null si l'application est à jour. */
    fun fetch(): Release? {
        val connection = open(MANIFEST)
        val text = try {
            when (val status = connection.responseCode) {
                200 -> Unit
                404 -> error("Aucune version Android publiée avec mise à jour automatique.")
                else -> error("GitHub a répondu HTTP $status.")
            }
            check(connection.url.protocol == "https") { "Réponse reçue hors HTTPS : ignorée." }
            connection.inputStream.use { input ->
                val bytes = input.readBytes()
                check(bytes.size < 64 * 1024) { "Manifeste trop gros : ignoré." }
                bytes.toString(Charsets.UTF_8)
            }
        } finally {
            connection.disconnect()
        }
        val manifest = JSONObject(text)
        val version = manifest.getString("version")
        val entry = manifest.getJSONObject("platforms").optJSONObject(platform)
            ?: error("La version $version n'est pas publiée pour l'édition « $edition ».")
        return release(entry, version, manifest.optString("notes"), manifest.optString("changelog"))
            .takeIf { it.code > BuildConfig.VERSION_CODE }
    }

    /**
     * Vérification complète : résultat gardé (bandeau, bouton), notification
     * si demandée (une fois par version). Lance une exception en cas d'échec.
     */
    fun check(context: Context, notify: Boolean): Release? {
        val store = Store(context)
        store.updateCheckedAt = System.currentTimeMillis()
        val release = fetch()
        store.updateRelease = release?.toJson().orEmpty()
        if (release != null && notify && store.updateNotified != release.version && notifyAvailable(context, release)) {
            store.updateNotified = release.version
        }
        return release
    }

    /** Télécharge et vérifie l'APK (taille, SHA-256, contenu). */
    fun download(context: Context, release: Release): File {
        val dir = File(context.cacheDir, "update").apply {
            deleteRecursively()
            mkdirs()
        }
        val file = File(dir, "VoiceToText.apk")
        progress.set(0)
        try {
            val connection = open(release.url)
            try {
                val status = connection.responseCode
                check(status == 200) { "Téléchargement refusé (HTTP $status)." }
                check(connection.url.protocol == "https") { "Téléchargement redirigé hors HTTPS : refusé." }
                val digest = MessageDigest.getInstance("SHA-256")
                var total = 0L
                connection.inputStream.use { input ->
                    file.outputStream().use { output ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            val read = input.read(buffer)
                            if (read < 0) break
                            total += read
                            check(total <= release.size) { "Fichier plus gros qu'annoncé : refusé." }
                            digest.update(buffer, 0, read)
                            output.write(buffer, 0, read)
                            progress.set((total * 100 / release.size).toInt())
                        }
                    }
                }
                check(total == release.size) { "Téléchargement incomplet : réessayez." }
                val sha256 = digest.digest().joinToString("") { "%02x".format(it) }
                check(sha256 == release.sha256) { "Empreinte SHA-256 inattendue : fichier refusé." }
            } finally {
                connection.disconnect()
            }
            inspect(context, file, release)
            return file
        } catch (error: Exception) {
            dir.deleteRecursively()
            throw error
        }
    }

    /** Relit l'APK téléchargé : même application, version annoncée, même signature, même édition. */
    private fun inspect(context: Context, file: File, release: Release) {
        val pm = context.packageManager
        val flags = PackageManager.GET_SIGNING_CERTIFICATES or PackageManager.GET_SERVICES
        val archive = archiveInfo(pm, file, flags) ?: error("Paquet illisible : fichier refusé.")
        check(archive.packageName == context.packageName) { "Ce paquet n'est pas VoiceToText : refusé." }
        check(archive.longVersionCode == release.code) { "Version du paquet différente de celle annoncée : refusé." }
        val bubble = archive.services.orEmpty().any { it.name.endsWith(".BubbleService") }
        check(bubble == BuildConfig.BUBBLE) { "Paquet d'une autre édition que « $edition » : refusé." }
        // Android revérifie la signature à l'installation ; ce contrôle donne
        // seulement un message clair. Signature non lue : on laisse Android juger.
        val incoming = archive.signingInfo ?: return
        val installed = installedInfo(pm, context.packageName).signingInfo ?: return
        val mine = installed.apkContentsSigners.toSet()
        val theirs = incoming.apkContentsSigners.toSet()
        val rotated = incoming.signingCertificateHistory.orEmpty().toSet().containsAll(mine)
        check(theirs == mine || rotated) { "Paquet signé avec une autre clé que l'application installée : refusé." }
    }

    private fun archiveInfo(pm: PackageManager, file: File, flags: Int): PackageInfo? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            pm.getPackageArchiveInfo(file.path, PackageManager.PackageInfoFlags.of(flags.toLong()))
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageArchiveInfo(file.path, flags)
        }

    private fun installedInfo(pm: PackageManager, name: String): PackageInfo =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            pm.getPackageInfo(name, PackageManager.PackageInfoFlags.of(PackageManager.GET_SIGNING_CERTIFICATES.toLong()))
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageInfo(name, PackageManager.GET_SIGNING_CERTIFICATES)
        }

    /** Confie l'APK vérifié à l'installateur d'Android ; le résultat arrive à [UpdateReceiver]. */
    fun install(context: Context, file: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(file.length())
            // Android 12 et plus : une application qui se met à jour elle-même
            // peut le faire sans écran de confirmation (le clic « Installer »
            // a déjà eu lieu). Sinon, Android demande la confirmation.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                session.openWrite("VoiceToText.apk", 0, file.length()).use { output ->
                    file.inputStream().use { it.copyTo(output) }
                    session.fsync(output)
                }
                val intent = Intent(context, UpdateReceiver::class.java)
                val pending = PendingIntent.getBroadcast(
                    context,
                    id,
                    intent,
                    // Mutable : Android y ajoute le statut de l'installation.
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                )
                session.commit(pending.intentSender)
            }
        } catch (error: Exception) {
            runCatching { installer.abandonSession(id) }
            throw error
        } finally {
            // Android a copié le paquet dans la session.
            file.parentFile?.deleteRecursively()
        }
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFY_AVAILABLE)
    }

    /** Vérification périodique (toutes les 6 h, réseau requis), gardée après un redémarrage. */
    fun schedule(context: Context) {
        val scheduler = context.getSystemService(JobScheduler::class.java)
        if (!supported || !Store(context).updatesEnabled) {
            scheduler.cancel(JOB_ID)
            return
        }
        if (scheduler.getPendingJob(JOB_ID) != null) return
        scheduler.schedule(
            JobInfo.Builder(JOB_ID, ComponentName(context, UpdateJob::class.java))
                .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
                .setPeriodic(INTERVAL_MS)
                .setPersisted(true)
                .build(),
        )
    }

    private fun channel(context: Context): NotificationManager {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Mises à jour", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "Nouvelle version de VoiceToText disponible"
            },
        )
        return manager
    }

    /** Annonce une nouvelle version ; false si les notifications sont coupées. */
    private fun notifyAvailable(context: Context, release: Release): Boolean {
        val manager = channel(context)
        if (!manager.areNotificationsEnabled()) return false
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val install = PendingIntent.getActivity(
            context,
            1,
            Intent(context, MainActivity::class.java).setAction(ACTION_INSTALL),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("VoiceToText ${release.version} disponible")
            .setContentText("Version installée : ${BuildConfig.VERSION_NAME}. Nouveautés dans l'application ; rien n'est installé sans votre accord.")
            .setContentIntent(open)
            .setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Installer", install).build())
            .build()
        manager.notify(NOTIFY_AVAILABLE, notification)
        return true
    }

    /** Confirmation d'Android à afficher alors que l'application n'est pas à l'écran. */
    fun notifyConfirm(context: Context, confirm: Intent) {
        val manager = channel(context)
        val pending = PendingIntent.getActivity(
            context,
            2,
            confirm,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Mise à jour de VoiceToText prête")
            .setContentText("Touchez pour l'installer.")
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build()
        manager.notify(NOTIFY_CONFIRM, notification)
    }

    /** Après la mise à jour : Android a fermé l'application, la notification la rouvre. */
    fun notifyUpdated(context: Context) {
        val manager = channel(context)
        manager.cancel(NOTIFY_AVAILABLE)
        manager.cancel(NOTIFY_CONFIRM)
        if (!manager.areNotificationsEnabled()) return
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("VoiceToText mis à jour : version ${BuildConfig.VERSION_NAME}")
            .setContentText("Touchez pour voir les nouveautés. Clavier, bulle et réglages sont gardés.")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        manager.notify(NOTIFY_AVAILABLE, notification)
    }

    /** Message pour un statut d'échec de l'installateur d'Android. */
    fun failure(status: Int, detail: String?): String {
        val reason = when (status) {
            PackageInstaller.STATUS_FAILURE_ABORTED -> "installation annulée"
            PackageInstaller.STATUS_FAILURE_BLOCKED ->
                "installation bloquée par le téléphone (Play Protect pour l'édition bulle : voir le README)"
            PackageInstaller.STATUS_FAILURE_CONFLICT -> "conflit avec l'application installée (signature différente ?)"
            PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> "paquet incompatible avec ce téléphone"
            PackageInstaller.STATUS_FAILURE_INVALID -> "paquet invalide"
            PackageInstaller.STATUS_FAILURE_STORAGE -> "espace de stockage insuffisant"
            else -> "échec de l'installation (code $status)"
        }
        val extra = detail?.takeIf { it.isNotBlank() }?.let { " : $it" }.orEmpty()
        return "Mise à jour non installée : $reason$extra."
    }
}

/** Résultat de l'installation, envoyé par Android (récepteur non exporté). */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val store = Store(context)
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_INTENT)
                } ?: return
                confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                // Android interdit d'ouvrir un écran depuis l'arrière-plan :
                // application cachée, la confirmation passe par une notification.
                val shown = Updater.visible && runCatching { context.startActivity(confirm) }.isSuccess
                if (!shown) Updater.notifyConfirm(context, confirm)
                store.updateError = ""
            }
            // L'application est remplacée puis relancée : rien à faire.
            PackageInstaller.STATUS_SUCCESS -> store.updateError = ""
            else -> store.updateError =
                Updater.failure(status, intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE))
        }
        Updater.listener?.invoke()
    }
}

/** Application remplacée par une nouvelle version (envoyé par Android). */
class UpdatedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED || !Updater.supported) return
        val store = Store(context)
        store.updateError = ""
        store.updateRelease = ""
        runCatching { Updater.schedule(context) }
        runCatching { Updater.notifyUpdated(context) }
    }
}

/** Vérification périodique, lancée par Android (réseau disponible). */
class UpdateJob : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        Thread {
            runCatching { Updater.check(applicationContext, notify = true) }
            jobFinished(params, false)
        }.start()
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean = true
}
