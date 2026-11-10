package io.github.aquadrox.voicetotext

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.inputmethodservice.InputMethodService
import android.os.Build
import android.os.SystemClock
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject

/**
 * Clavier vocal : maintenir le micro pour dicter (relâcher pour insérer), ou
 * un appui court pour dicter mains libres (nouvel appui pour finir). La touche
 * à gauche du micro active le mode traduction ; maintenir celle de droite
 * dicte une consigne appliquée au texte sélectionné (Command Mode). Le texte
 * s'insère dans le champ en cours.
 */
class VoiceKeyboard : InputMethodService() {
    private val store by lazy { Store(this) }
    private val recorder = Recorder()

    private lateinit var status: TextView
    /** Édition bulle : bulle activée mais arrêtée par le téléphone. */
    private lateinit var bubbleAlert: TextView
    private lateinit var mic: ImageButton
    private lateinit var command: ImageButton
    private lateinit var translate: ImageButton
    private lateinit var ring: LevelRing

    /** Anneau du micro : suit le niveau de la voix tant que l'enregistrement dure. */
    private val meter = object : Runnable {
        override fun run() {
            if (recorder.recording) {
                ring.level = maxOf(kotlin.math.sqrt(recorder.level), ring.level * 0.8f)
                ring.postDelayed(this, 50)
            } else {
                ring.level = 0f
            }
        }
    }

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
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor(BACKGROUND))
            setPadding(dp(12), dp(8), dp(12), dp(12))
        }
        status = TextView(this).apply {
            setTextColor(Color.parseColor(MUTED))
            textSize = 14f
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, dp(4))
        }
        bubbleAlert = TextView(this).apply {
            text = "Bulle arrêtée par le téléphone · touchez ici pour la relancer"
            setTextColor(Color.parseColor(ACCENT))
            textSize = 14f
            gravity = Gravity.CENTER
            setPadding(dp(8), dp(4), dp(8), dp(8))
            visibility = View.GONE
            setOnClickListener {
                Toast.makeText(context, "Désactivez la bulle puis réactivez-la : elle repart", Toast.LENGTH_LONG).show()
                BubbleStatus.openSettings(context)
            }
        }
        mic = ImageButton(this).apply {
            setImageResource(R.drawable.ic_mic)
            imageTintList = ColorStateList.valueOf(Color.WHITE)
            scaleType = ImageView.ScaleType.FIT_CENTER
            setPadding(dp(20), dp(20), dp(20), dp(20))
            background = keyBackground(ACCENT, oval = true)
            contentDescription = "Dicter (maintenir ; appui court : mains libres)"
            setOnTouchListener { view, event -> onHold(view, event, Native.DICTATE) }
        }
        ring = LevelRing(this, Color.parseColor(ACCENT)).apply { inner = dp(36).toFloat() }
        command = key(R.drawable.ic_text_select, "Consigne sur le texte sélectionné (maintenir)").apply {
            setOnTouchListener { view, event -> onHold(view, event, Native.COMMAND) }
        }
        translate = key(R.drawable.ic_languages, "Mode traduction").apply {
            setOnClickListener {
                store.translate = !store.translate
                showTranslate()
                show(if (store.translate) "Traduction → ${Dictation.language}" else "Traduction coupée")
            }
        }
        val keyHeight = dp(56)
        val margin = dp(4)
        fun LinearLayout.side(view: View, weight: Float = 1f) = addView(
            view,
            LinearLayout.LayoutParams(0, keyHeight, weight).apply { setMargins(margin, margin, margin, margin) },
        )
        // Micro au centre, dans son anneau de niveau.
        val micFrame = FrameLayout(this).apply {
            addView(ring, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
            addView(mic, FrameLayout.LayoutParams(dp(72), dp(72), Gravity.CENTER))
        }
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            side(key(R.drawable.ic_keyboard, "Clavier suivant").apply { setOnClickListener { switchToNextInputMethod(false) } })
            side(translate)
            addView(micFrame, LinearLayout.LayoutParams(dp(96), dp(96)))
            side(command)
            side(key(R.drawable.ic_delete, "Effacer").apply { setOnClickListener { sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL) } })
        }
        val bottom = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            side(key(R.drawable.ic_space, "Espace").apply { setOnClickListener { currentInputConnection?.commitText(" ", 1) } }, 3f)
            side(key(R.drawable.ic_corner_down_left, "Entrée").apply { setOnClickListener { enter() } })
        }
        val wide = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        root.addView(bubbleAlert, wide)
        root.addView(status, wide)
        root.addView(top, wide)
        root.addView(bottom, wide)
        avoidNavigationBar(root)
        showTranslate()
        return root
    }

    /**
     * Android 15 et plus étendent la fenêtre du clavier sous la barre de
     * navigation (boutons retour, accueil, applications) : sans marge, Espace
     * et Entrée passent dessous. La marge du bas suit la hauteur de la barre,
     * calculée comme Android le fait pour la sienne (`captionBar`, comprise
     * dans `systemBars`, quand le clavier dessine lui-même la barre en
     * navigation par gestes). La barre reste transparente sur le fond d'encre,
     * avec des icônes claires. Avant Android 15, Android cale déjà le clavier
     * au-dessus de la barre : rien à faire.
     */
    private fun avoidNavigationBar(root: View) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return
        root.setOnApplyWindowInsetsListener { view, insets ->
            val bar = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout()).bottom
            view.setPadding(dp(12), dp(8), dp(12), dp(12) + bar)
            insets
        }
        window?.window?.let { window ->
            // Sans voile gris clair par-dessus le fond (barre à trois boutons).
            window.isNavigationBarContrastEnforced = false
            window.insetsController?.setSystemBarsAppearance(0, WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS)
        }
    }

    /** Touche : icône claire sur fond d'encre arrondi, ondulation à l'appui. */
    private fun key(icon: Int, description: String) = ImageButton(this).apply {
        setImageResource(icon)
        imageTintList = ColorStateList.valueOf(Color.parseColor(PAPER))
        scaleType = ImageView.ScaleType.CENTER
        background = keyBackground(KEY)
        contentDescription = description
    }

    private fun keyBackground(color: String, oval: Boolean = false): Drawable {
        fun filled(fill: Int) = GradientDrawable().apply {
            if (oval) shape = GradientDrawable.OVAL else cornerRadius = dp(12).toFloat()
            setColor(fill)
        }
        // Le second fond (blanc) sert de masque : l'ondulation reste dans la forme.
        return RippleDrawable(ColorStateList.valueOf(Color.parseColor(RIPPLE)), filled(Color.parseColor(color)), filled(Color.WHITE))
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        lastInserted = 0
        checkBubble()
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

    /**
     * La bulle a été arrêtée par le téléphone : on se rabat alors sur ce
     * clavier, qui le dit et mène à l'écran où la relancer (Android ne la
     * relance pas seul, même si ce clavier vient de redémarrer l'application).
     */
    private fun checkBubble() {
        if (!::bubbleAlert.isInitialized) return
        bubbleAlert.removeCallbacks(bubbleCheck)
        val state = runCatching { BubbleStatus.state(this) }.getOrDefault(BubbleStatus.State.DISABLED)
        bubbleAlert.visibility = if (state == BubbleStatus.State.STOPPED) View.VISIBLE else View.GONE
        if (state == BubbleStatus.State.STOPPED) BubbleStatus.recordStop(this, store)
        // Application tout juste lancée : Android reconnecte peut-être encore la bulle.
        if (state == BubbleStatus.State.STARTING) bubbleAlert.postDelayed(bubbleCheck, BubbleStatus.SETTLE_MS)
    }

    private val bubbleCheck = Runnable { checkBubble() }

    /** Mode traduction allumé : touche inversée (papier, icône d'encre). */
    private fun showTranslate() {
        if (!::translate.isInitialized) return
        translate.background = keyBackground(if (store.translate) PAPER else KEY)
        translate.imageTintList = ColorStateList.valueOf(Color.parseColor(if (store.translate) BACKGROUND else PAPER))
    }

    private fun loadEngine() {
        show("Chargement du modèle…")
        Dictation.prepare(store) { error ->
            show(if (error == null) IDLE else "$error (ouvrez l'application VoiceToText)")
        }
    }

    /** Micro et consigne : maintenir pour dicter ; appui court sur le micro : mains libres. */
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
            // Modèle libéré entre-temps (bulle, écran éteint) : rechargé.
            if (Dictation.stale(store)) loadEngine() else show("Modèle pas encore prêt")
            return
        }
        if (!recorder.start()) {
            show("Micro indisponible")
            return
        }
        // Consigne : sa touche passe au vermillon ; l'anneau du micro suit la voix.
        if (view === command) command.background = keyBackground(ACCENT)
        ring.removeCallbacks(meter)
        ring.post(meter)
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
        command.background = keyBackground(KEY)
        show(if (mode == Native.COMMAND) "Application de la consigne…" else "Transcription…")
        val app = currentInputEditorInfo?.packageName.orEmpty()
        val selection = if (mode == Native.COMMAND) {
            currentInputConnection?.getSelectedText(0)?.toString().orEmpty()
        } else {
            ""
        }
        val current = mode
        val privacy = privacy(currentInputEditorInfo)
        Dictation.run(store, samples, app, current, selection, privacy) { apply(it, current) }
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
        const val IDLE = "Maintenez le micro pour dicter · touche de droite : consigne"
        const val IDLE_SHORT = "maintenez le micro pour dicter"
        // Couleurs du logo : encre, papier, et vermillon (curseur) pour le micro.
        const val BACKGROUND = "#17171C"
        const val KEY = "#2A2A30"
        const val ACCENT = "#FF5B36"
        const val PAPER = "#F1EFE8"
        const val MUTED = "#C9C7C0"
        const val RIPPLE = "#33FFFFFF"
    }
}

/** Disque autour du micro : s'élargit avec le niveau de la voix (0 : rien). */
class LevelRing(context: Context, tint: Int) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = tint
        alpha = 80
    }

    var level = 0f
        set(value) {
            field = value.coerceIn(0f, 1f)
            invalidate()
        }

    /** Rayon du bouton dessiné par-dessus (px). */
    var inner = 0f

    override fun onDraw(canvas: Canvas) {
        if (level <= 0.01f) return
        val cx = width / 2f
        val cy = height / 2f
        canvas.drawCircle(cx, cy, inner + (minOf(cx, cy) - inner) * level, paint)
    }
}
