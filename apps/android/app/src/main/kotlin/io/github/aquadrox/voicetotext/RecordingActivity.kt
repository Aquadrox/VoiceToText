package io.github.aquadrox.voicetotext

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Bundle
import android.os.SystemClock
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject

/**
 * Enregistrement en cours, lisible de loin (téléphone posé sur la table) :
 * gros chrono dans un anneau qui suit le niveau du micro, « Marquer un
 * moment », « Arrêter ». Quitter l'écran n'arrête rien : la notification
 * garde la main.
 */
class RecordingActivity : Activity() {
    private val store by lazy { Store(this) }
    private var meetingId = ""
    private val opened = SystemClock.elapsedRealtime()
    private lateinit var title: TextView
    private lateinit var clock: TextView
    private lateinit var ring: Ring
    private lateinit var marks: TextView
    private val tick = object : Runnable {
        override fun run() {
            refresh()
            clock.postDelayed(this, 200)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        meetingId = intent.getStringExtra(MeetingService.EXTRA_ID).orEmpty()
        build()
    }

    override fun onResume() {
        super.onResume()
        // Écran allumé tant qu'on le regarde (chrono visible sur la table).
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        clock.post(tick)
    }

    override fun onPause() {
        clock.removeCallbacks(tick)
        super.onPause()
    }

    private fun build() {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            fitsSystemWindows = true
            setBackgroundColor(getColor(R.color.bg))
        }
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(dp(16), dp(28), dp(16), 0)
        }
        top.addView(
            TextView(this).apply {
                text = "●  Enregistrement"
                setTextColor(getColor(R.color.signal))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
                background = rounded(getColor(R.color.signal) and 0x00FFFFFF or 0x1F000000, 14f)
                setPadding(dp(12), dp(5), dp(12), dp(5))
            },
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT),
        )
        title = TextView(this).apply {
            setTextColor(getColor(R.color.fg))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
            gravity = Gravity.CENTER
            setPadding(0, dp(8), 0, 0)
        }
        top.addView(title)
        root.addView(top)

        val middle = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
        }
        val dial = FrameLayout(this)
        ring = Ring(this)
        dial.addView(ring, FrameLayout.LayoutParams(dp(264), dp(264), Gravity.CENTER))
        val face = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            background = rounded(getColor(R.color.card), 96f, getColor(R.color.signal), 3)
        }
        clock = TextView(this).apply {
            setTextColor(getColor(R.color.fg))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 48f)
            fontFeatureSettings = "tnum"
            gravity = Gravity.CENTER
            text = "00:00"
        }
        face.addView(clock)
        face.addView(TextView(this).apply {
            text = "micro"
            gravity = Gravity.CENTER
            setTextColor(getColor(R.color.muted))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
        })
        dial.addView(face, FrameLayout.LayoutParams(dp(192), dp(192), Gravity.CENTER))
        middle.addView(dial, LinearLayout.LayoutParams(dp(264), dp(264)))
        val infos = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(24), dp(24), 0)
        }
        infos.info(R.drawable.ic_lock, audioText())
        infos.info(R.drawable.ic_audio_lines, "Transcription et compte rendu à la fin")
        marks = infos.info(R.drawable.ic_flag, "")
        middle.addView(infos, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        root.addView(middle, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))

        val warning = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), 0, dp(16), dp(12))
        }
        warning.text(
            "Prévenez les participants que la réunion est enregistrée. Vous pouvez éteindre l'écran : " +
                "l'enregistrement continue et s'arrête depuis la notification.",
            13f,
            R.color.warning_fg,
        ).apply {
            background = rounded(getColor(R.color.warning_bg), 12f)
            setPadding(dp(12), dp(10), dp(12), dp(10))
        }
        root.addView(warning)

        root.addView(View(this).apply { setBackgroundColor(getColor(R.color.border)) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setBackgroundColor(getColor(R.color.card))
            setPadding(dp(16), dp(4), dp(16), dp(16))
        }
        bar.secondary("Marquer un moment") {
            MeetingService.start(this, MeetingService.ACTION_MARK, meetingId)
        }
        bar.primary("Arrêter") {
            MeetingService.start(this, MeetingService.ACTION_STOP, meetingId)
            startActivity(MeetingActivity.intent(this, meetingId))
            finish()
        }.apply {
            background = pressable(rounded(getColor(R.color.signal), 10f), 10f)
            setTextColor(getColor(R.color.on_signal))
            setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_stop, 0, 0, 0)
            compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.on_signal))
        }
        root.addView(bar)
        setContentView(root)
    }

    private fun audioText(): String {
        val detail = JSONObject(Native.meeting(store.meetingsDir, meetingId)).optJSONObject("ok")
        title.text = detail?.optString("title").orEmpty()
        return when (detail?.optString("audio")) {
            "memoire" -> "Audio en mémoire seulement (perdu si le téléphone ferme l'appli)"
            "temporaire" -> "Audio effacé après la transcription"
            else -> "Audio gardé sur le téléphone (stockage privé, chiffré par Android)"
        }
    }

    private fun refresh() {
        if (MeetingService.recordingId != meetingId) {
            // Le service démarre peut-être encore : quelques secondes de grâce.
            if (SystemClock.elapsedRealtime() - opened < START_GRACE_MS) return
            // Arrêté depuis la notification : place au suivi de la transcription.
            startActivity(MeetingActivity.intent(this, meetingId))
            finish()
            return
        }
        val seconds = (SystemClock.elapsedRealtime() - MeetingService.startedAt) / 1000
        clock.text = Meetings.clock(seconds * 1000)
        ring.level = MeetingService.level
        val count = MeetingService.marks
        marks.text = if (count == 0) "« Marquer un moment » : repère dans la transcription" else "$count moment(s) marqué(s)"
    }

    /** Ligne d'information : icône grise et texte. */
    private fun LinearLayout.info(icon: Int, value: String): TextView {
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, dp(5), 0, dp(5))
        }
        row.addView(
            ImageView(context).apply {
                setImageResource(icon)
                imageTintList = ColorStateList.valueOf(getColor(R.color.muted))
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            },
            LinearLayout.LayoutParams(dp(18), dp(18)).apply { marginEnd = dp(10) },
        )
        val label = row.text(value, 14f, R.color.muted)
        addView(row)
        return label
    }

    /** Anneaux vermillon qui s'élargissent avec la voix. */
    private class Ring(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = context.getColor(R.color.signal) }
        var level = 0f
            set(value) {
                field = value.coerceIn(0f, 1f)
                invalidate()
            }

        override fun onDraw(canvas: Canvas) {
            val cx = width / 2f
            val cy = height / 2f
            val outer = minOf(cx, cy)
            paint.alpha = 18
            canvas.drawCircle(cx, cy, outer, paint)
            paint.alpha = 34
            val inner = outer * 0.73f
            canvas.drawCircle(cx, cy, inner + (outer - inner) * minOf(1f, level * 3f), paint)
        }
    }

    companion object {
        private const val START_GRACE_MS = 3_000L

        fun intent(context: Context, meetingId: String): Intent =
            Intent(context, RecordingActivity::class.java).putExtra(MeetingService.EXTRA_ID, meetingId)
    }
}
