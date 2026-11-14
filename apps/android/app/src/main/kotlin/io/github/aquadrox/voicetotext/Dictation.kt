package io.github.aquadrox.voicetotext

import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Moteur partagé par le clavier et la bulle : chargement du modèle quand les
 * réglages changent, puis dictées les unes après les autres sur un seul fil.
 */
object Dictation {
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    /** Version des réglages chargée dans le moteur (-1 : pas encore). */
    private var loaded = -1L
    private var loading = false
    private val waiting = mutableListOf<(String?) -> Unit>()

    /** Langue du mode traduction (réglages). */
    var language = "anglais"
        private set

    val ready: Boolean
        get() = loaded >= 0 && !loading

    /** Le moteur doit être rechargé (réglages ou clé modifiés). */
    fun stale(store: Store) = store.version != loaded

    /**
     * Charge le modèle si besoin ; `done` reçoit null si tout va bien, sinon
     * l'erreur. Appelé sur le fil principal, rappel sur le fil principal.
     */
    fun prepare(store: Store, done: (String?) -> Unit) {
        if (!stale(store) && !loading) {
            done(null)
            return
        }
        waiting += done
        if (loading) return
        loading = true
        val version = store.version
        val config = store.config
        val modelsDir = store.modelsDir
        val historyFile = store.historyFile
        worker.execute {
            // Une exception ici tuerait tout le processus (et la bulle avec) :
            // elle devient l'erreur rendue à l'appelant.
            val (settings, error) = try {
                val settings = JSONObject(Native.settings(config, modelsDir)).optJSONObject("ok")
                val host = settings?.optJSONObject("llm")?.optString("host").orEmpty()
                Native.setApiKey(store.key(host).orEmpty())
                settings to JSONObject(Native.start(modelsDir, historyFile, config)).optString("error")
            } catch (e: Throwable) {
                null to failure(e)
            }
            main.post {
                loading = false
                language = settings?.optString("translation")?.ifEmpty { null } ?: language
                if (error.isEmpty()) loaded = version
                val callbacks = waiting.toList()
                waiting.clear()
                callbacks.forEach { it(error.ifEmpty { null }) }
            }
        }
    }

    /**
     * Libère le modèle (plusieurs centaines de Mo de mémoire vive) : le
     * prochain [prepare] le recharge. La bulle le demande écran éteint, pour
     * que le téléphone ait moins de raisons d'arrêter l'application. Fil
     * principal.
     */
    fun release() {
        // Réunion en cours de transcription : le modèle reste chargé.
        if (loaded < 0 || loading || MeetingService.processing != null) return
        loaded = -1
        worker.execute { runCatching { Native.release() } }
    }

    /** Dicte en arrière-plan ; réponse JSON de [Native.dictate] sur le fil principal. */
    fun run(
        store: Store,
        samples: FloatArray,
        app: String,
        mode: Int,
        selection: String,
        privacy: Int,
        done: (JSONObject) -> Unit,
    ) {
        // Modèle libéré ou réglages modifiés pendant l'enregistrement :
        // rechargé d'abord, sur la même file d'attente.
        if (stale(store)) prepare(store) {}
        worker.execute {
            val result = try {
                JSONObject(Native.dictate(samples, app, mode, selection, privacy))
            } catch (e: Throwable) {
                JSONObject().put("kind", "error").put("error", failure(e))
            }
            main.post { done(result) }
        }
    }

    /** Message d'une exception inattendue, gardé aussi dans le journal Android. */
    private fun failure(e: Throwable): String {
        Log.e("VoiceToText", "Erreur du moteur", e)
        return "Erreur interne : ${e.message ?: e.javaClass.simpleName}"
    }

    /** « ✓ 1,2 s » ou l'erreur du LLM si le texte brut a été inséré. */
    fun summary(result: JSONObject): String {
        val error = result.optString("error")
        val seconds = "%.1f".format(result.optLong("ms") / 1000.0).replace('.', ',')
        return if (error.isEmpty() || error == "null") "✓ $seconds s" else "Texte brut inséré : $error"
    }
}
