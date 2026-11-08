package io.github.aquadrox.voicetotext

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.inputmethodservice.InputMethodService
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

/**
 * Clavier vocal : maintenir 🎤 pour dicter (relâcher pour insérer), ou un
 * appui court pour dicter mains libres (nouvel appui pour finir). 🌍 active
 * le mode traduction ; maintenir ✨ dicte une consigne appliquée au texte
 * sélectionné (Command Mode). Le texte s'insère dans le champ en cours.
 */
class VoiceKeyboard : InputMethodService() {
    private val store by lazy { Store(this) }
    private val recorder = Recorder()

    private lateinit var status: TextView
    private lateinit var mic: Button
    private lateinit var command: Button
    private lateinit var translate: Button

    private var pressedAt = 0L
    private var handsFree = false
    /** Mode de la dictée en cours (Native.DICTATE, TRANSLATE, COMMAND). */
    private var mode = Native.DICTATE
    /** Longueur de la dernière insertion (commande « annule la dernière dictée »). */
    private var lastInserted = 0

    override fun onCreate() {
        super.onCreate()
        // Vérification des mises à jour replanifiée (effacée par un arrêt forcé).
        runCatching { Updater.schedule(this) }
    }

    override fun onCreateInputView(): View {
        val dp = resources.displayMetrics.density
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor(BACKGROUND))
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
                setColor(Color.parseColor(ACCENT))
            }
            contentDescription = "Dicter"
            setOnTouchListener { view, event -> onHold(view, event, Native.DICTATE) }
        }
        fun key(label: String, description: String) = Button(this).apply {
            text = label
            textSize = 18f
            setTextColor(Color.WHITE)
            setBackgroundColor(Color.parseColor(KEY))
            contentDescription = description
        }
        command = key("✨", "Consigne sur le texte sélectionné (maintenir)").apply {
            setOnTouchListener { view, event -> onHold(view, event, Native.COMMAND) }
        }
        translate = key("🌍", "Mode traduction").apply {
            setOnClickListener {
                store.translate = !store.translate
                showTranslate()
                show(if (store.translate) "Traduction → ${Dictation.language}" else "Traduction coupée")
            }
        }
        val keyHeight = (56 * dp).toInt()
        val margin = (4 * dp).toInt()
        fun LinearLayout.side(view: View, weight: Float = 1f) = addView(
            view,
            LinearLayout.LayoutParams(0, keyHeight, weight).apply { setMargins(margin, margin, margin, margin) },
        )
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            side(key("🌐", "Clavier suivant").apply { setOnClickListener { switchToNextInputMethod(false) } })
            side(translate)
            addView(mic, LinearLayout.LayoutParams((88 * dp).toInt(), (88 * dp).toInt()).apply {
                setMargins((8 * dp).toInt(), 0, (8 * dp).toInt(), 0)
            })
            side(command)
            side(key("⌫", "Effacer").apply { setOnClickListener { sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL) } })
        }
        val bottom = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            side(key("espace", "Espace").apply { setOnClickListener { currentInputConnection?.commitText(" ", 1) } }, 3f)
            side(key("↵", "Entrée").apply { setOnClickListener { enter() } })
        }
        val wide = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        root.addView(status, wide)
        root.addView(top, wide)
        root.addView(bottom, wide)
        showTranslate()
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
        if (Dictation.stale(store)) loadEngine()
        else show(IDLE)
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        if (recorder.recording) recorder.stop()
        handsFree = false
        super.onFinishInputView(finishingInput)
    }

    private fun showTranslate() {
        if (!::translate.isInitialized) return
        translate.setBackgroundColor(Color.parseColor(if (store.translate) ACCENT else KEY))
    }

    private fun loadEngine() {
        show("Chargement du modèle…")
        Dictation.prepare(store) { error ->
            show(if (error == null) IDLE else "$error (ouvrez l'application VoiceToText)")
        }
    }

    /** 🎤 et ✨ : maintenir pour dicter ; appui court sur 🎤 : mains libres. */
    @SuppressLint("ClickableViewAccessibility")
    private fun onHold(view: View, event: MotionEvent, kind: Int): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                if (handsFree) {
                    // Mains libres en cours : cet appui la termine.
                    handsFree = false
                    finishDictation()
                } else {
                    pressedAt = SystemClock.uptimeMillis()
                    mode = when {
                        kind == Native.COMMAND -> Native.COMMAND
                        store.translate -> Native.TRANSLATE
                        else -> Native.DICTATE
                    }
                    startDictation(view)
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (!recorder.recording) return true
                if (kind == Native.DICTATE && SystemClock.uptimeMillis() - pressedAt < TAP_MS) {
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

    private fun startDictation(view: View) {
        if (!hasMicPermission()) {
            show("Autorisez le micro dans l'application VoiceToText")
            return
        }
        if (!Dictation.ready) {
            show("Modèle pas encore prêt")
            return
        }
        if (!recorder.start()) {
            show("Micro indisponible")
            return
        }
        view.alpha = 0.7f
        show(
            when (mode) {
                Native.COMMAND -> "Consigne… (appliquée au texte sélectionné)"
                Native.TRANSLATE -> "Écoute… → ${Dictation.language}"
                else -> "Écoute…"
            },
        )
    }

    private fun finishDictation() {
        val samples = recorder.stop()
        mic.alpha = 1f
        command.alpha = 1f
        show(if (mode == Native.COMMAND) "Application de la consigne…" else "Transcription…")
        val app = currentInputEditorInfo?.packageName.orEmpty()
        val selection = if (mode == Native.COMMAND) {
            currentInputConnection?.getSelectedText(0)?.toString().orEmpty()
        } else {
            ""
        }
        val current = mode
        val privacy = privacy(currentInputEditorInfo)
        Dictation.run(samples, app, current, selection, privacy) { apply(it, current) }
    }

    private fun apply(result: JSONObject, kind: Int) {
        val connection = currentInputConnection
        when (result.optString("kind")) {
            "text" -> {
                val text = result.optString("text")
                // Une espace entre deux dictées, pas en début de champ ; une
                // consigne remplace la sélection telle quelle.
                val before = connection?.getTextBeforeCursor(1, 0)?.toString().orEmpty()
                val spaced = if (kind != Native.COMMAND && before.isNotEmpty() && !before.last().isWhitespace()) {
                    " $text"
                } else {
                    text
                }
                connection?.commitText(spaced, 1)
                lastInserted = spaced.length
                val summary = Dictation.summary(result)
                show(if (summary.startsWith("✓")) "$summary · $IDLE_SHORT" else summary)
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

    /**
     * Champ de mot de passe : ni LLM ni historique. Champ « ne pas apprendre »
     * (navigation privée, application sensible) : pas d'historique.
     */
    private fun privacy(editor: EditorInfo?): Int {
        val info = editor ?: return Native.NORMAL
        return when {
            Native.isPassword(info.inputType) -> Native.PASSWORD
            info.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING != 0 -> Native.NO_HISTORY
            else -> Native.NORMAL
        }
    }

    private fun hasMicPermission() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun show(text: String) {
        if (::status.isInitialized) status.text = text
    }

    override fun onDestroy() {
        if (recorder.recording) recorder.stop()
        super.onDestroy()
    }

    private companion object {
        /** En dessous : appui court (mains libres). */
        const val TAP_MS = 300L
        const val IDLE = "Maintenez 🎤 pour dicter · ✨ consigne sur la sélection"
        const val IDLE_SHORT = "maintenez 🎤 pour dicter"
        const val BACKGROUND = "#1E1E26"
        const val KEY = "#2C2C38"
        const val ACCENT = "#4F46E5"
    }
}
