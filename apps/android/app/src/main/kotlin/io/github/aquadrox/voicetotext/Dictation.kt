package io.github.aquadrox.voicetotext

import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

/**
 * Moteur partagé par le clavier et la bulle : chargement du modèle quand les
 * réglages changent, puis dictées les unes après les autres sur un seul fil.
 */
object Dictation {
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val nextLiveGen = AtomicLong(0)
    private val cancelledLiveGen = AtomicLong(0)

    /** Version des réglages chargée dans le moteur (-1 : pas encore). */
    private var loaded = -1L
    private var loading = false
    private val waiting = mutableListOf<(String?) -> Unit>()

    /** Langue du mode traduction (réglages). */
    var language = "anglais"
        private set

    /** Lecture opt-in du contexte autour du curseur (réglages ; faux par défaut). */
    var screenContext = false
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
                screenContext = settings?.optBoolean("screen_context", false) ?: false
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
        liveCancel()
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
        context: String = "",
        done: (JSONObject) -> Unit,
    ) {
        // Modèle libéré ou réglages modifiés pendant l'enregistrement :
        // rechargé d'abord, sur la même file d'attente.
        if (stale(store)) prepare(store) {}
        worker.execute {
            val result = try {
                JSONObject(Native.dictate(samples, app, mode, selection, privacy, context))
            } catch (e: Throwable) {
                JSONObject().put("kind", "error").put("error", failure(e))
            }
            main.post { done(result) }
        }
    }

    /**
     * Démarre une session mains libres en continu sur le fil du moteur.
     * Renvoie l'identifiant de génération à passer à [liveFeed] et [liveFinish].
     */
    fun liveStart(
        store: Store,
        app: String,
        mode: Int,
        privacy: Int,
        context: String = "",
        onError: (Long, String) -> Unit = { _, _ -> },
    ): Long {
        val gen = nextLiveGen.incrementAndGet()
        if (stale(store)) prepare(store) {}
        worker.execute {
            if (cancelledLiveGen.get() >= gen) return@execute
            val error = try {
                LiveLogic.parseError(Native.liveStart(app, mode, privacy, context))
            } catch (e: Throwable) {
                failure(e)
            }
            if (!error.isNullOrEmpty() && cancelledLiveGen.get() < gen) {
                main.post {
                    if (cancelledLiveGen.get() < gen) onError(gen, error)
                }
            }
        }
        return gen
    }

    /**
     * Envoie une tranche audio incrémentale à la session mains libres ;
     * `onPreview` est rappelé sur le fil principal uniquement si l'aperçu a changé.
     */
    fun liveFeed(
        generation: Long,
        samples: FloatArray,
        onPreview: (String) -> Unit,
    ) {
        if (samples.isEmpty() || cancelledLiveGen.get() >= generation) return
        worker.execute {
            if (cancelledLiveGen.get() >= generation) return@execute
            val parsed = try {
                LiveLogic.parseFeed(Native.liveFeed(samples))
            } catch (e: Throwable) {
                Log.w("VoiceToText", "Erreur prévisualisation en continu", e)
                null
            }
            if (parsed != null && parsed.changed && parsed.preview.isNotEmpty() && cancelledLiveGen.get() < generation) {
                main.post {
                    if (cancelledLiveGen.get() < generation) onPreview(parsed.preview)
                }
            }
        }
    }

    /**
     * Finalise la session mains libres avec le reliquat audio sur le fil du
     * moteur ; réponse JSON (même forme que [run]) sur le fil principal.
     */
    fun liveFinish(
        generation: Long,
        tail: FloatArray,
        done: (JSONObject) -> Unit,
    ) {
        worker.execute {
            if (cancelledLiveGen.get() >= generation) return@execute
            val result = try {
                JSONObject(Native.liveFinish(tail))
            } catch (e: Throwable) {
                JSONObject().put("kind", "error").put("error", failure(e))
            }
            if (cancelledLiveGen.get() < generation) {
                main.post { done(result) }
            }
        }
    }

    /** Annule immédiatement toute session mains libres en cours sans bloquer. */
    fun liveCancel() {
        val latest = nextLiveGen.get()
        cancelledLiveGen.accumulateAndGet(latest) { current, candidate -> maxOf(current, candidate) }
        runCatching { Native.liveCancel() }
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
