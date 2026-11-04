package io.github.aquadrox.voicetotext

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Clavier vocal : maintenir le micro pour dicter (relâcher pour insérer), ou
 * un appui court pour dicter mains libres (nouvel appui pour finir). Le texte
 * nettoyé est inséré dans le champ en cours, comme sur PC.
 */
class VoiceKeyboard : InputMethodService() {
    private val store by lazy { Store(this) }
    private val recorder = Recorder()
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    private lateinit var status: TextView
    private lateinit var mic: Button

    /** Version des réglages chargée dans le moteur (-1 : pas encore). */
    private var loaded = -1L
    private var ready = false
    private var pressedAt = 0L
    private var handsFree = false
    /** Longueur de la dernière insertion (commande « annule la dernière dictée »). */
    private var lastInserted = 0

    override fun onCreateInputView(): View {
        val dp = resources.displayMetrics.density
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#1E1E26"))
            setPadding((12 * dp).toInt(), (8 * dp).toInt(), (12 * dp).toInt(), (12 * dp).toInt())
        }
        status = TextView(this).apply {
            setTextColor(Color.parseColor("#C8C8D2"))
            textSize = 14f
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, (8 * dp).toInt())
        }
        mic = Button(this).apply {
            text = "🎤"
            textSize = 30f
            setTextColor(Color.WHITE)
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(Color.parseColor("#4F46E5"))
            }
            setOnTouchListener(::onMicTouch)
            contentDescription = "Dicter"
        }
        fun key(label: String, description: String, action: () -> Unit) = Button(this).apply {
            text = label
            textSize = 18f
            setTextColor(Color.WHITE)
            setBackgroundColor(Color.parseColor("#2C2C38"))
            contentDescription = description
            setOnClickListener { action() }
        }
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        val keyHeight = (56 * dp).toInt()
        fun side(view: View) = row.addView(
            view,
            LinearLayout.LayoutParams(0, keyHeight, 1f).apply { setMargins((4 * dp).toInt(), 0, (4 * dp).toInt(), 0) },
        )
        side(key("🌐", "Clavier suivant") { switchToNextInputMethod(false) })
        side(key("⌫", "Effacer") { sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL) })
        row.addView(mic, LinearLayout.LayoutParams((88 * dp).toInt(), (88 * dp).toInt()).apply {
            setMargins((8 * dp).toInt(), 0, (8 * dp).toInt(), 0)
        })
        side(key("␣", "Espace") { currentInputConnection?.commitText(" ", 1) })
        side(key("↵", "Entrée") { enter() })
        root.addView(status, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        root.addView(row, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        return root
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        lastInserted = 0
        if (!hasMicPermission()) {
            show("Autorisez le micro dans l'application VoiceToText")
            return
        }
        // Réglages ou clé modifiés depuis le dernier affichage : moteur mis à jour.
        if (store.version != loaded) loadEngine()
        else if (ready) show(IDLE)
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        if (recorder.recording) recorder.stop()
        handsFree = false
        super.onFinishInputView(finishingInput)
    }

    private fun loadEngine() {
        val version = store.version
        val config = store.config
        ready = false
        show("Chargement du modèle…")
        worker.execute {
            val host = JSONObject(Native.settings(config, store.modelsDir)).optJSONObject("ok")
                ?.optJSONObject("llm")?.optString("host").orEmpty()
            Native.setApiKey(store.key(host).orEmpty())
            val error = JSONObject(Native.start(store.modelsDir, config)).optString("error")
            main.post {
                if (error.isEmpty()) {
                    loaded = version
                    ready = true
                    show(IDLE)
                } else {
                    show("$error (ouvrez l'application VoiceToText)")
                }
            }
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun onMicTouch(view: View, event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                if (handsFree) {
                    // Mains libres en cours : cet appui la termine.
                    handsFree = false
                    finishDictation()
                } else {
                    pressedAt = SystemClock.uptimeMillis()
                    startDictation()
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (!recorder.recording) return true
                if (SystemClock.uptimeMillis() - pressedAt < TAP_MS) {
                    // Appui court : mains libres jusqu'au prochain appui.
                    handsFree = true
                    show("Mains libres : appuyez pour terminer")
                } else {
                    finishDictation()
                }
            }
        }
        view.performClick()
        return true
    }

    private fun startDictation() {
        if (!hasMicPermission()) {
            show("Autorisez le micro dans l'application VoiceToText")
            return
        }
        if (!ready) {
            show("Modèle pas encore prêt")
            return
        }
        if (!recorder.start()) {
            show("Micro indisponible")
            return
        }
        mic.alpha = 0.7f
        show("Écoute…")
    }

    private fun finishDictation() {
        val samples = recorder.stop()
        mic.alpha = 1f
        show("Transcription…")
        val app = currentInputEditorInfo?.packageName.orEmpty()
        worker.execute {
            val result = JSONObject(Native.dictate(samples, app))
            main.post { apply(result) }
        }
    }

    private fun apply(result: JSONObject) {
        val connection = currentInputConnection
        when (result.optString("kind")) {
            "text" -> {
                val text = result.optString("text")
                // Une espace entre deux dictées, pas en début de champ.
                val before = connection?.getTextBeforeCursor(1, 0)?.toString().orEmpty()
                val spaced = if (before.isNotEmpty() && !before.last().isWhitespace()) " $text" else text
                connection?.commitText(spaced, 1)
                lastInserted = spaced.length
                val error = result.optString("error")
                show(if (error.isEmpty() || error == "null") IDLE else "Texte brut inséré : $error")
            }
            "undo" -> {
                if (lastInserted > 0) connection?.deleteSurroundingText(lastInserted, 0)
                lastInserted = 0
                show("Dernière dictée annulée")
            }
            "skip" -> show("Rien entendu")
            else -> show(result.optString("error").ifEmpty { "Erreur de dictée" })
        }
    }

    private fun enter() {
        val info = currentInputEditorInfo
        val action = info?.imeOptions?.and(EditorInfo.IME_MASK_ACTION) ?: EditorInfo.IME_ACTION_NONE
        if (action != EditorInfo.IME_ACTION_NONE && action != EditorInfo.IME_ACTION_UNSPECIFIED &&
            info?.imeOptions?.and(EditorInfo.IME_FLAG_NO_ENTER_ACTION) == 0
        ) {
            currentInputConnection?.performEditorAction(action)
        } else {
            currentInputConnection?.commitText("\n", 1)
        }
    }

    private fun hasMicPermission() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun show(text: String) {
        if (::status.isInitialized) status.text = text
    }

    override fun onDestroy() {
        if (recorder.recording) recorder.stop()
        worker.shutdown()
        super.onDestroy()
    }

    private companion object {
        /** En dessous : appui court (mains libres). */
        const val TAP_MS = 300L
        const val IDLE = "Maintenez 🎤 pour dicter · appui court : mains libres"
    }
}
