package io.github.aquadrox.voicetotext

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import kotlin.math.hypot

/**
 * Bulle de dictée flottante, sans changer de clavier : elle apparaît quand un
 * champ de texte est actif. Toucher : dicter (toucher encore pour finir) ;
 * maintenir : consigne appliquée au texte sélectionné ; glisser : déplacer.
 *
 * Service d'accessibilité : c'est le seul moyen offert par Android pour
 * savoir quel champ est actif et y écrire. Il regarde seulement si le champ
 * actif accepte du texte, n'en lit le contenu qu'au moment de la dictée et
 * n'enregistre rien. Jamais de bulle sur un champ de mot de passe.
 */
class BubbleService : AccessibilityService() {
    private val store by lazy { Store(this) }
    private val recorder = Recorder()
    private val main = Handler(Looper.getMainLooper())
    private lateinit var manager: WindowManager

    private var bubble: TextView? = null
    private var params: WindowManager.LayoutParams? = null
    /** Taille de la bulle affichée (Store.bubbleSize). */
    private var shownSize = -1

    private var state = IDLE
    /** Mode de la dictée en cours (Native.DICTATE, TRANSLATE, COMMAND). */
    private var mode = Native.DICTATE
    /** Consigne en cours : se termine au relâchement. */
    private var holding = false
    /** Champ actif au début de la dictée. */
    private var target: AccessibilityNodeInfo? = null
    /** Longueur de la dernière insertion (commande « annule la dernière dictée »). */
    private var lastInserted = 0

    private var downX = 0f
    private var downY = 0f
    private var startX = 0
    private var startY = 0
    private var dragging = false

    private val refresh = Runnable { update() }
    private val longPress = Runnable { startCommand() }
    private val pulse = object : Runnable {
        override fun run() {
            val view = bubble ?: return
            val scale = if (state == LISTENING) 0.8f + minOf(recorder.level * 2f, 0.2f) else 1f
            view.scaleX = scale
            view.scaleY = scale
            if (state == LISTENING) main.postDelayed(this, 80)
        }
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        manager = getSystemService(WindowManager::class.java)
        runCatching { Updater.schedule(this) }
        update()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // Les événements arrivent en rafale : une seule mise à jour après.
        main.removeCallbacks(refresh)
        main.postDelayed(refresh, 120)
    }

    override fun onInterrupt() {
        if (recorder.recording) recorder.stop()
        state = IDLE
        holding = false
        paint()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        if (recorder.recording) recorder.stop()
        hide()
        super.onDestroy()
    }

    /**
     * Bulle affichée seulement sur un champ de texte ordinaire. Une exception
     * ici (fenêtre disparue entre-temps…) tuerait le service, qu'Android
     * marquerait « Ne fonctionne pas » sans le relancer : on l'ignore, le
     * prochain événement réessaie.
     */
    private fun update() {
        if (!::manager.isInitialized || state != IDLE) return
        runCatching { if (field() == null) hide() else show() }
            .onFailure { Log.w("VoiceToText", "Bulle non mise à jour", it) }
    }

    private fun field(): AccessibilityNodeInfo? {
        val node = findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return null
        if (!writable(node)) return null
        if (store.bubbleWithKeyboard && windows.none { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }) return null
        // Le clavier VoiceToText a déjà son micro.
        val keyboard = Settings.Secure.getString(contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD).orEmpty()
        if (keyboard.startsWith("$packageName/") && windows.any { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }) {
            return null
        }
        return node
    }

    /** Champ de texte qui n'est pas un mot de passe (drapeau ou type de saisie). */
    private fun writable(node: AccessibilityNodeInfo) =
        node.isEditable && !node.isPassword && !Native.isPassword(node.inputType)

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    @SuppressLint("ClickableViewAccessibility")
    private fun show() {
        val size = store.bubbleSize
        if (bubble != null && size == shownSize) return
        hide()
        shownSize = size
        val px = dp(SIZES[size])
        val screen = resources.displayMetrics
        val view = TextView(this).apply {
            gravity = Gravity.CENTER
            textSize = SIZES[size] * 0.4f
            setTextColor(Color.WHITE)
            elevation = dp(4).toFloat()
            contentDescription = "Dicter avec VoiceToText"
            setOnTouchListener { view, event -> onTouch(view, event) }
            // Fenêtre retirée par Android (et non par hide()) : la bulle est
            // oubliée puis remise au prochain passage, au lieu de rester
            // invisible jusqu'à un changement de taille.
            addOnAttachStateChangeListener(object : View.OnAttachStateChangeListener {
                override fun onViewAttachedToWindow(view: View) {}

                override fun onViewDetachedFromWindow(view: View) {
                    if (bubble !== view) return
                    bubble = null
                    params = null
                    if (state != IDLE) {
                        if (recorder.recording) recorder.stop()
                        holding = false
                        state = IDLE
                    }
                    main.removeCallbacks(refresh)
                    main.postDelayed(refresh, 120)
                }
            })
        }
        val layout = WindowManager.LayoutParams(
            px,
            px,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            // Jamais le focus : le champ en cours le garde, le clavier reste ouvert.
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            val savedX = store.bubbleX
            val savedY = store.bubbleY
            x = (if (savedX >= 0) savedX else screen.widthPixels - px - dp(8)).coerceIn(0, maxOf(0, screen.widthPixels - px))
            y = (if (savedY >= 0) savedY else (screen.heightPixels * 0.35f).toInt()).coerceIn(0, maxOf(0, screen.heightPixels - px))
        }
        runCatching { manager.addView(view, layout) }.onFailure { return }
        bubble = view
        params = layout
        paint()
        // Modèle chargé dès l'apparition : la première dictée part sans attendre.
        if (Dictation.stale(store)) Dictation.prepare(store) {}
    }

    private fun hide() {
        // Oubliée avant le retrait : le détachement qui suit vient de nous.
        val view = bubble ?: return
        bubble = null
        params = null
        runCatching { manager.removeView(view) }
    }

    private fun paint(done: Boolean = false) {
        val view = bubble ?: return
        val (color, label) = when {
            done -> SUCCESS to "✓"
            state == LISTENING -> RECORDING to if (mode == Native.COMMAND) "✨" else "🎤"
            state == WORKING -> WAITING to "…"
            else -> ACCENT to "🎤"
        }
        view.text = label
        view.background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(Color.parseColor(color))
        }
        view.alpha = if (state == IDLE && !done) 0.85f else 1f
        main.removeCallbacks(pulse)
        main.post(pulse)
    }

    private fun onTouch(view: View, event: MotionEvent): Boolean {
        val layout = params ?: return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.rawX
                downY = event.rawY
                startX = layout.x
                startY = layout.y
                dragging = false
                if (state == IDLE) main.postDelayed(longPress, LONG_PRESS_MS)
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = event.rawX - downX
                val dy = event.rawY - downY
                if (!dragging && !holding && hypot(dx, dy) > ViewConfiguration.get(this).scaledTouchSlop) {
                    dragging = true
                    main.removeCallbacks(longPress)
                }
                if (dragging) {
                    layout.x = startX + dx.toInt()
                    layout.y = startY + dy.toInt()
                    runCatching { manager.updateViewLayout(view, layout) }
                }
            }
            MotionEvent.ACTION_UP -> {
                main.removeCallbacks(longPress)
                when {
                    dragging -> {
                        store.bubbleX = layout.x
                        store.bubbleY = layout.y
                    }
                    holding -> {
                        holding = false
                        finish()
                    }
                    state == LISTENING -> finish()
                    state == IDLE -> start(if (store.translate) Native.TRANSLATE else Native.DICTATE)
                }
            }
            MotionEvent.ACTION_CANCEL -> {
                main.removeCallbacks(longPress)
                if (holding) {
                    holding = false
                    finish()
                }
            }
        }
        return true
    }

    private fun startCommand() {
        if (state != IDLE || dragging) return
        holding = true
        bubble?.performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
        start(Native.COMMAND)
    }

    private fun start(kind: Int) {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            say("Autorisez le micro dans l'application VoiceToText")
            return
        }
        target = field() ?: findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { writable(it) }
        if (target == null) {
            say("Touchez d'abord un champ de texte")
            return
        }
        // Réglages modifiés : rechargés avant la dictée (même file d'attente).
        if (Dictation.stale(store)) Dictation.prepare(store) { error -> error?.let { say(it) } }
        if (!recorder.start()) {
            say("Micro indisponible")
            return
        }
        mode = kind
        state = LISTENING
        paint()
    }

    private fun finish() {
        if (state != LISTENING) return
        val samples = recorder.stop()
        if (samples.isNotEmpty() && recorder.loudest == 0f) {
            state = IDLE
            paint()
            say("Micro coupé par Android : voir « Bulle » dans l'application VoiceToText")
            return
        }
        state = WORKING
        paint()
        val node = target
        val app = node?.packageName?.toString().orEmpty()
        val selection = if (mode == Native.COMMAND && node != null) selected(node) else ""
        val current = mode
        // Pas de bulle sur les mots de passe. La navigation privée est
        // invisible d'ici : historique de la bulle coupable à part.
        val privacy = if (store.bubbleHistory) Native.NORMAL else Native.NO_HISTORY
        Dictation.run(samples, app, current, selection, privacy) { result ->
            state = IDLE
            val ok = apply(result, current)
            paint(done = ok)
            if (ok) main.postDelayed({ paint() }, 800)
            update()
        }
    }

    /** Insère le résultat ; vrai si du texte a été écrit. */
    private fun apply(result: JSONObject, kind: Int): Boolean {
        // Le champ de départ, sinon le champ actif de la même fenêtre : jamais
        // un autre champ ouvert entre-temps (autre application, mot de passe).
        val origin = target
        target = null
        val node = origin?.takeIf { it.refresh() && writable(it) }
            ?: findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf {
                origin != null && writable(it) && it.windowId == origin.windowId && it.packageName == origin.packageName
            }
        when (result.optString("kind")) {
            "text" -> {
                if (node == null) {
                    say("Champ de texte introuvable")
                    return false
                }
                val inserted = insert(node, result.optString("text"), spaced = kind != Native.COMMAND)
                if (inserted < 0) {
                    say("Cette application refuse l'écriture : essayez « Insérer en collant »")
                    return false
                }
                lastInserted = inserted
                val summary = Dictation.summary(result)
                if (!summary.startsWith("✓")) say(summary)
                return true
            }
            "undo" -> {
                if (node != null && lastInserted > 0) undo(node)
                lastInserted = 0
                say("Dernière dictée annulée")
            }
            "skip" -> say("Rien entendu")
            else -> say(result.optString("error").ifEmpty { "Erreur de dictée" })
        }
        return false
    }

    /** Texte réel du champ (sans l'indication grisée d'un champ vide). */
    private fun content(node: AccessibilityNodeInfo): String =
        if (node.isShowingHintText) "" else node.text?.toString().orEmpty()

    /** Sélection (ou curseur) ; fin du champ si l'application ne la donne pas. */
    private fun range(node: AccessibilityNodeInfo, text: String): Pair<Int, Int> {
        var start = node.textSelectionStart
        var end = node.textSelectionEnd
        if (start < 0 || end < 0) {
            start = text.length
            end = text.length
        }
        if (start > end) start = end.also { end = start }
        return start.coerceIn(0, text.length) to end.coerceIn(0, text.length)
    }

    private fun selected(node: AccessibilityNodeInfo): String {
        val text = content(node)
        val (start, end) = range(node, text)
        return text.substring(start, end)
    }

    /**
     * Écrit `text` à la place de la sélection : directement dans les champs
     * Android ordinaires, sinon en collant (pages web, éditeurs riches, où
     * réécrire le champ perdrait la mise en forme). Renvoie la longueur écrite
     * ou -1.
     */
    private fun insert(node: AccessibilityNodeInfo, text: String, spaced: Boolean): Int {
        val current = content(node)
        val (start, end) = range(node, current)
        val before = current.substring(0, start)
        val piece = if (spaced && before.isNotEmpty() && !before.last().isWhitespace()) " $text" else text
        val plain = node.className?.toString() == "android.widget.EditText"
        if (plain && !store.bubblePaste && setText(node, before + piece + current.substring(end), start + piece.length)) {
            return piece.length
        }
        return if (paste(node, piece)) piece.length else -1
    }

    private fun undo(node: AccessibilityNodeInfo) {
        val current = content(node)
        val (cursor, _) = range(node, current)
        if (cursor < lastInserted) return
        setText(node, current.removeRange(cursor - lastInserted, cursor), cursor - lastInserted)
    }

    private fun setText(node: AccessibilityNodeInfo, value: String, cursor: Int): Boolean {
        val text = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, value)
        }
        if (!node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, text)) return false
        val selection = Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor)
        }
        node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selection)
        return true
    }

    /** Coller remplace le contenu du presse-papier (Android interdit de le relire). */
    private fun paste(node: AccessibilityNodeInfo, text: String): Boolean {
        val clipboard = getSystemService(ClipboardManager::class.java)
        if (runCatching { clipboard.setPrimaryClip(ClipData.newPlainText("VoiceToText", text)) }.isFailure) return false
        return node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
    }

    private fun say(text: String) {
        Toast.makeText(this, text, Toast.LENGTH_SHORT).show()
    }

    private companion object {
        const val IDLE = 0
        const val LISTENING = 1
        const val WORKING = 2
        /** Au-delà : appui long (consigne). */
        const val LONG_PRESS_MS = 450L
        /** Diamètre de la bulle (dp) : petite, moyenne, grande. */
        val SIZES = intArrayOf(44, 56, 68)
        const val ACCENT = "#4F46E5"
        const val RECORDING = "#DC2626"
        const val WAITING = "#52525B"
        const val SUCCESS = "#16A34A"
    }
}
