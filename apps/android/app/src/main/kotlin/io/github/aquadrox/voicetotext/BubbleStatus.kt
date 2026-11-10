package io.github.aquadrox.voicetotext

import android.accessibilityservice.AccessibilityServiceInfo
import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.os.SystemClock
import android.provider.Settings
import android.view.accessibility.AccessibilityManager
import java.text.DateFormat
import java.util.Date

/**
 * État de la bulle vu de l'extérieur de son service (application, clavier).
 *
 * Quand le téléphone tue l'application et refuse de la relancer (Xiaomi,
 * Samsung… sans « démarrage automatique »), le service d'accessibilité reste
 * coché dans les réglages mais mort (« Ne fonctionne pas »). Android ne le
 * reconnecte alors jamais de lui-même, même si l'application redémarre pour
 * le clavier, et n'offre aucun moyen à l'application de le faire : seul
 * l'utilisateur le peut, en désactivant puis réactivant la bulle. On détecte
 * donc cet état, on le signale et on amène l'utilisateur au bon écran.
 */
object BubbleStatus {
    enum class State {
        /** Pas cochée dans les réglages d'accessibilité (ou édition sans bulle). */
        DISABLED,
        RUNNING,
        /** Processus tout juste lancé : Android n'a pas fini de reconnecter le service. */
        STARTING,
        /** Cochée mais morte : à relancer par l'utilisateur. */
        STOPPED,
    }

    /** Après le lancement du processus, délai avant de déclarer la bulle arrêtée. */
    const val SETTLE_MS = 4_000L

    private fun component(context: Context) = ComponentName(context, BubbleService::class.java)

    fun state(context: Context): State {
        if (!BuildConfig.BUBBLE) return State.DISABLED
        val bubble = component(context)
        val enabled = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
            .orEmpty().split(':').any { ComponentName.unflattenFromString(it) == bubble }
        if (!enabled) return State.DISABLED
        // Le réglage ci-dessus reste coché quand le service est mort ; cette
        // liste-ci ne contient que les services réellement connectés.
        val running = context.getSystemService(AccessibilityManager::class.java)
            .getEnabledAccessibilityServiceList(AccessibilityServiceInfo.FEEDBACK_ALL_MASK)
            .any { info -> info.resolveInfo.serviceInfo.let { ComponentName(it.packageName, it.name) } == bubble }
        return when {
            running -> State.RUNNING
            SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime() < SETTLE_MS -> State.STARTING
            else -> State.STOPPED
        }
    }

    /**
     * Ouvre la page de la bulle dans les réglages d'accessibilité, là où se
     * trouve son interrupteur ; à défaut (écran absent chez certains
     * constructeurs), la liste des services, bulle mise en évidence.
     */
    fun openSettings(context: Context) {
        val name = component(context).flattenToString()
        val page = Intent("android.settings.ACCESSIBILITY_DETAILS_SETTINGS")
            .putExtra(Intent.EXTRA_COMPONENT_NAME, name)
        val list = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
            .putExtra(":settings:fragment_args_key", name)
            .putExtra(":settings:show_fragment_args", Bundle().apply { putString(":settings:fragment_args_key", name) })
        for (intent in listOf(page, list)) {
            // Lancé aussi depuis le clavier (un service, sans écran à lui).
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (runCatching { context.startActivity(intent) }.isSuccess) return
        }
    }

    /** La bulle vient de se connecter : l'arrêt précédent est noté, puis une nouvelle période commence. */
    fun connected(context: Context, store: Store) {
        recordStop(context, store, recovered = true)
        store.bubbleStartedAt = System.currentTimeMillis()
    }

    /** Bulle désactivée par l'utilisateur : rien à expliquer. */
    fun disconnected(store: Store) {
        store.bubbleStartedAt = 0
    }

    /**
     * Note pourquoi la bulle s'est arrêtée : premier arrêt du processus
     * (donné par Android 11 et plus) depuis sa dernière connexion. Gardé dans
     * les réglages : toujours affiché après un arrêt forcé ou une relance.
     * `recovered` : la bulle est repartie d'elle-même (une mise à jour de
     * l'application n'est alors pas un arrêt à signaler).
     */
    fun recordStop(context: Context, store: Store, recovered: Boolean = false) {
        val since = store.bubbleStartedAt
        if (since == 0L || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
        // Du plus récent au plus ancien : le dernier de la liste après `since`
        // est l'arrêt qui a emporté la bulle.
        val exit = runCatching {
            context.getSystemService(ActivityManager::class.java)
                .getHistoricalProcessExitReasons(context.packageName, 0, 0)
        }.getOrNull()?.lastOrNull { it.timestamp >= since } ?: return
        store.bubbleStartedAt = 0
        if (recovered && exit.reason == ApplicationExitInfo.REASON_PACKAGE_UPDATED) return
        store.bubbleStop = describe(exit)
    }

    private fun describe(exit: ApplicationExitInfo): String {
        val reason = when (exit.reason) {
            ApplicationExitInfo.REASON_CRASH -> "plantage de l'application"
            ApplicationExitInfo.REASON_CRASH_NATIVE -> "plantage du moteur de dictée"
            ApplicationExitInfo.REASON_ANR -> "application bloquée (ANR)"
            ApplicationExitInfo.REASON_LOW_MEMORY -> "mémoire insuffisante"
            ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE -> "consommation jugée excessive"
            ApplicationExitInfo.REASON_SIGNALED -> "tuée par le système (signal ${exit.status})"
            ApplicationExitInfo.REASON_OTHER -> "arrêtée par le système"
            ApplicationExitInfo.REASON_USER_REQUESTED -> "arrêt forcé, ou application fermée depuis les applications récentes"
            ApplicationExitInfo.REASON_USER_STOPPED -> "arrêtée par l'utilisateur"
            ApplicationExitInfo.REASON_PERMISSION_CHANGE -> "permission modifiée"
            ApplicationExitInfo.REASON_DEPENDENCY_DIED -> "composant du système arrêté"
            ApplicationExitInfo.REASON_FREEZER -> "application figée par le système"
            ApplicationExitInfo.REASON_PACKAGE_STATE_CHANGE -> "application désactivée puis réactivée"
            ApplicationExitInfo.REASON_PACKAGE_UPDATED -> "mise à jour de l'application"
            else -> "code ${exit.reason}"
        }
        val date = DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(exit.timestamp))
        val detail = exit.description?.takeIf { it.isNotBlank() }?.let { " : $it" }.orEmpty()
        return "$reason$detail ($date)"
    }
}
