package io.github.aquadrox.voicetotext

import android.Manifest
import android.app.Activity
import android.app.Dialog
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.text.SpannableString
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.text.style.RelativeSizeSpan
import android.text.style.StyleSpan
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.TextView
import io.github.aquadrox.voicetotext.Meetings.objects
import org.json.JSONObject

/**
 * Réunions : liste (la plus récente d'abord), « Enregistrer » et « Importer »
 * en bas. Reçoit aussi les fichiers audio partagés depuis une autre
 * application (« Partager → VoiceToText »).
 */
class MeetingsActivity : Activity() {
    private val store by lazy { Store(this) }
    private lateinit var message: TextView
    private lateinit var list: LinearLayout
    private lateinit var footer: TextView
    /** Fichier choisi en attendant l'autorisation, ou ouverture demandée. */
    private var pendingRecord = false
    private val poll = object : Runnable {
        override fun run() {
            load()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        screen(bar = {
            primary("Enregistrer") { askRecord() }.apply {
                background = pressable(rounded(getColor(R.color.signal), 10f), 10f)
                setTextColor(getColor(R.color.on_signal))
                setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_dot, 0, 0, 0)
                compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.on_signal))
            }
            secondary("Importer") { pickFile() }.apply {
                setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_upload, 0, 0, 0)
                compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.fg))
            }
        }) {
            header("Réunions", "Enregistrer ou importer, puis transcription et compte rendu", onBack = { finish() })
            message = notice()
            list = LinearLayout(this@MeetingsActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(list)
            footer = note("")
        }
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onResume() {
        super.onResume()
        // Application arrêtée pendant un enregistrement ou une transcription : reprise.
        if (!MeetingService.busy) {
            val pending = JSONObject(Native.meetingRecover(store.meetingsDir)).optJSONArray("ok")
            for (i in 0 until (pending?.length() ?: 0)) {
                MeetingService.start(this, MeetingService.ACTION_PROCESS, pending!!.getString(i))
            }
        }
        load()
    }

    override fun onPause() {
        list.removeCallbacks(poll)
        super.onPause()
    }

    /** Fichier partagé, raccourci « Enregistrer une réunion » ou carte de l'accueil. */
    private fun handle(intent: Intent?) {
        intent ?: return
        val uri = when (intent.action) {
            Intent.ACTION_SEND -> if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                @Suppress("DEPRECATION")
                intent.getParcelableExtra(Intent.EXTRA_STREAM)
            }
            Intent.ACTION_VIEW -> intent.data
            else -> null
        }
        val record = intent.action == ACTION_NEW
        intent.action = null
        if (uri != null) sheet(uri) else if (record) askRecord()
    }

    private fun load() {
        list.removeCallbacks(poll)
        list.removeAllViews()
        val reply = JSONObject(Native.meetings(store.meetingsDir, store.config))
        val rows = reply.optJSONArray("ok")?.objects().orEmpty()
        message.say(reply.optString("error"))
        if (rows.isEmpty()) {
            list.card().apply {
                note("Aucune réunion pour l'instant.")
                note("« Enregistrer » : posez le téléphone sur la table ; transcription, intervenants et compte rendu à la fin. « Importer » : un mémo vocal, une note vocale, l'enregistrement d'un appel, un cours… (aussi depuis une autre application : Partager → VoiceToText).")
            }
        }
        var group = ""
        var card: LinearLayout? = null
        for (row in rows) {
            val name = Meetings.group(row.getLong("started_at_ms"))
            if (name != group || card == null) {
                group = name
                list.text(name, 13f, R.color.muted, bold = true).setPadding(dp(4), dp(16), 0, dp(2))
                card = list.card().apply {
                    setPadding(0, dp(2), 0, dp(2))
                    clipToOutline = true
                }
            } else {
                card.divider(0)
            }
            card.addView(item(row))
        }
        val settings = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")
        val keep = settings?.optInt("meetings_keep_days") ?: 0
        footer.text = "Réunions gardées sur le téléphone seulement" +
            if (keep > 0) ", effacées après $keep jours." else "."
        val active = rows.any { it.optString("status") in listOf("enregistrement", "transcription") || it.has("step") && !it.isNull("step") }
        if (active || MeetingService.busy) list.postDelayed(poll, 1_500)
    }

    private fun item(row: JSONObject): View {
        val id = row.getString("id")
        val item = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(12), dp(16), dp(12))
            background = pressable(null, 0f)
            setOnClickListener {
                if (MeetingService.recordingId == id) startActivity(RecordingActivity.intent(this@MeetingsActivity, id))
                else startActivity(MeetingActivity.intent(this@MeetingsActivity, id))
            }
        }
        val line = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        line.addView(
            TextView(this).apply {
                text = row.optString("title")
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
            },
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f),
        )
        val duration = if (MeetingService.recordingId == id) "● en cours" else Meetings.clock(row.optLong("duration_ms"))
        line.text(duration, 14f, if (MeetingService.recordingId == id) R.color.signal else R.color.muted).apply {
            fontFeatureSettings = "tnum"
        }
        item.addView(line)
        val status = row.optString("status")
        val percent = when {
            status == "enregistrement" && row.optBoolean("imported") -> row.optInt("progress")
            row.optString("step") == "transcription" -> row.optInt("progress")
            else -> -1
        }
        if (percent >= 0) {
            item.addView(
                ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
                    max = 100
                    progress = percent
                    progressTintList = ColorStateList.valueOf(getColor(R.color.accent))
                },
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT),
            )
        }
        item.text("${Meetings.date(row.getLong("started_at_ms"))} · ${Meetings.meta(row)}", 13f, R.color.muted)
        return item
    }

    // ---- Nouvelle réunion ----

    private fun askRecord() {
        if (MeetingService.recordingId != null) {
            startActivity(RecordingActivity.intent(this, MeetingService.recordingId!!))
            return
        }
        val wanted = buildList {
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= 33 && !store.askedNotifications &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                store.askedNotifications = true
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
        if (wanted.isEmpty()) {
            sheet(null)
        } else {
            pendingRecord = true
            requestPermissions(wanted.toTypedArray(), PERMISSIONS)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != PERMISSIONS || !pendingRecord) return
        pendingRecord = false
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) sheet(null)
        else message.say("Autorisez le micro pour enregistrer une réunion.")
    }

    private fun pickFile() {
        startActivityForResult(
            Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
                putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("audio/*", "video/*"))
            },
            PICK,
        )
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == PICK && resultCode == RESULT_OK) data?.data?.let { sheet(it) }
    }

    private fun displayName(uri: Uri): String = runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull() ?: uri.lastPathSegment?.substringAfterLast('/') ?: "Enregistrement importé"

    /**
     * Feuille « Nouvelle réunion » (enregistrement si `file` est nul, sinon
     * import) : titre, modèle de compte rendu, audio, compte rendu à la fin.
     */
    private fun sheet(file: Uri?) {
        val dialog = Dialog(this)
        val name = file?.let { displayName(it) }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(10), dp(16), dp(20))
            background = GradientDrawable().apply {
                setColor(getColor(R.color.card))
                cornerRadii = FloatArray(8) { if (it < 4) dp(20).toFloat() else 0f }
            }
        }
        column.addView(
            View(this).apply { background = rounded(getColor(R.color.border_strong), 2f) },
            LinearLayout.LayoutParams(dp(36), dp(4)).apply {
                gravity = Gravity.CENTER_HORIZONTAL
                bottomMargin = dp(10)
            },
        )
        column.text(if (file == null) "Nouvelle réunion" else "Importer un enregistrement", 20f, bold = true)
        if (name != null) column.note(name)
        val title = column.field(if (file == null) "Titre (facultatif)" else "Titre (par défaut : nom du fichier)")

        column.label("Type de compte rendu")
        var conference = store.meetingConference
        val types = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            background = rounded(getColor(R.color.bg), 12f)
            setPadding(dp(4), dp(4), dp(4), dp(4))
        }
        val meetingType = types.segment("Réunion")
        val courseType = types.segment("Cours ou conférence")
        val paint = {
            meetingType.select(!conference)
            courseType.select(conference)
        }
        meetingType.setOnClickListener { conference = false; paint() }
        courseType.setOnClickListener { conference = true; paint() }
        paint()
        column.addView(types, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6) })
        column.note("Décisions et actions, ou plan et idées clés. Modifiable après coup.")

        column.label("Audio")
        val audio = RadioGroup(this)
        val choices = listOf(
            Triple("conserve", "Garder sur le téléphone", "Stockage privé de l'appli, chiffré par Android. Réécouter un passage. Par défaut."),
            Triple("temporaire", "Effacer après la transcription", "Seul le texte reste."),
            Triple("memoire", "En mémoire seulement", "Rien sur le stockage ; perdu si le téléphone ferme l'appli."),
        )
        for ((key, label, detail) in choices) {
            audio.addView(RadioButton(this).apply {
                id = View.generateViewId()
                tag = key
                text = twoLines(label, detail)
                setTextColor(getColor(R.color.fg))
                buttonTintList = ColorStateList.valueOf(getColor(R.color.accent))
                setPadding(dp(6), dp(8), 0, dp(8))
                isChecked = key == store.meetingAudio
            })
        }
        column.addView(audio)

        val settings = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")
        val llm = settings?.optJSONObject("llm")?.optBoolean("enabled") == true
        val model = settings?.optString("model")
        val modelReady = settings?.optJSONArray("models")?.objects()
            ?.any { it.optString("id") == model && it.optBoolean("downloaded") } == true
        val summary = column.option("Compte rendu à la fin", store.meetingSummary && llm)
        if (!llm) {
            summary.isEnabled = false
            column.note("Nettoyage par LLM désactivé (accueil → Nettoyage du texte) : transcription seule.")
        } else {
            column.note("La transcription est envoyée au LLM choisi pour le compte rendu ; l'audio ne quitte jamais le téléphone.")
        }
        // Intervenants : modèles téléchargés une fois (sur accord : interrupteur).
        val voicesReady = settings?.optBoolean("voices") == true
        val voicesMb = settings?.optInt("voices_mb") ?: 33
        val speakers = column.option("Distinguer les intervenants", store.meetingSpeakers && voicesReady)
        val voicesNote = column.note(
            if (voicesReady) "« Intervenant 1, 2… » à la fin, à nommer ensuite. Voix analysées sur le téléphone."
            else "Télécharge une fois deux petits modèles ($voicesMb Mo). Voix analysées sur le téléphone.",
        )
        val voicesProgress = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
            max = 100
            visibility = View.GONE
            progressTintList = ColorStateList.valueOf(getColor(R.color.accent))
        }.also { column.addView(it) }
        val pollVoices = object : Runnable {
            override fun run() {
                if (!Meetings.Voices.downloading) return
                voicesProgress.progress = Native.voicesProgress()
                voicesProgress.postDelayed(this, 300)
            }
        }
        speakers.setOnCheckedChangeListener { _, checked ->
            store.meetingSpeakers = checked
            val installed = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")?.optBoolean("voices") == true
            if (!checked || installed) return@setOnCheckedChangeListener
            voicesProgress.visibility = View.VISIBLE
            voicesNote.text = "Téléchargement des modèles des intervenants ($voicesMb Mo)…"
            Meetings.Voices.download(this, store) { error ->
                voicesProgress.visibility = View.GONE
                if (error == null) {
                    voicesNote.text = "Modèles prêts : « Intervenant 1, 2… » à la fin, à nommer ensuite."
                } else {
                    voicesNote.text = "Téléchargement échoué : $error. Réessayez avec l'interrupteur."
                    speakers.isChecked = false
                }
            }
            voicesProgress.post(pollVoices)
        }
        column.more(
            "Méthode de pyannote.audio (MIT) ; modèles pyannote segmentation 3.0 (MIT) et WeSpeaker ResNet34-LM (CC BY 4.0), convertis en ONNX par sherpa-onnx. " +
                "Les empreintes vocales sont gardées avec la réunion (stockage privé de l'application) pour changer le nombre d'intervenants, jamais pour une réunion « en mémoire seulement », et effacées avec elle.",
            "À propos des intervenants",
        )
        if (file == null) {
            column.text("Prévenez les participants : leur accord est nécessaire pour enregistrer.", 13f, R.color.warning_fg).apply {
                background = rounded(getColor(R.color.warning_bg), 12f)
                setPadding(dp(12), dp(10), dp(12), dp(10))
                (layoutParams as LinearLayout.LayoutParams).topMargin = dp(10)
            }
        }
        val start: Button = column.primary(if (file == null) "Commencer l'enregistrement" else "Importer et transcrire") {
            val chosen = audio.findViewById<RadioButton>(audio.checkedRadioButtonId)?.tag as? String ?: "conserve"
            store.meetingAudio = chosen
            store.meetingConference = conference
            if (llm) store.meetingSummary = summary.isChecked
            val reply = JSONObject(
                Native.meetingCreate(
                    store.meetingsDir, title.text.toString(), conference, chosen, summary.isChecked, name.orEmpty(),
                    speakers.isChecked,
                ),
            )
            val id = reply.optString("ok")
            if (id.isEmpty()) {
                message.say(reply.optString("error"))
                dialog.dismiss()
                return@primary
            }
            dialog.dismiss()
            if (file == null) {
                MeetingService.start(this, MeetingService.ACTION_RECORD, id)
                startActivity(RecordingActivity.intent(this, id))
            } else {
                MeetingService.start(this, MeetingService.ACTION_IMPORT, id, file)
                startActivity(MeetingActivity.intent(this, id))
            }
        }
        if (file == null) {
            start.background = pressable(rounded(getColor(R.color.signal), 10f), 10f)
            start.setTextColor(getColor(R.color.on_signal))
        }
        if (!modelReady) {
            // Rien ne serait transcrit : on le dit avant d'enregistrer.
            start.isEnabled = false
            column.note("Téléchargez d'abord un modèle de transcription (accueil → Modèle de transcription).")
        }

        dialog.setContentView(ScrollView(this).apply { addView(column) })
        dialog.window?.apply {
            setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
            setGravity(Gravity.BOTTOM)
            setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        }
        dialog.show()
    }

    /** Titre en gras, explication en gris plus petite. */
    private fun twoLines(label: String, detail: String): CharSequence =
        SpannableString("$label\n$detail").apply {
            setSpan(StyleSpan(Typeface.BOLD), 0, label.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            setSpan(ForegroundColorSpan(getColor(R.color.muted)), label.length, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            setSpan(RelativeSizeSpan(0.87f), label.length, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }

    private fun LinearLayout.label(value: String) {
        text(value, 13f, R.color.muted, bold = true).setPadding(0, dp(14), 0, 0)
    }

    /** Choix d'un sélecteur à deux segments. */
    private class Segment(context: Context) : TextView(context) {
        fun select(selected: Boolean) {
            background = if (selected) context.rounded(context.getColor(R.color.card), 9f, context.getColor(R.color.border)) else null
            setTextColor(context.getColor(if (selected) R.color.fg else R.color.muted))
            isSelected = selected
        }
    }

    private fun LinearLayout.segment(label: String): Segment = Segment(context).apply {
        text = label
        gravity = Gravity.CENTER
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        minHeight = dp(44)
    }.also { addView(it, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)) }

    companion object {
        /** Ouvre directement la feuille « Nouvelle réunion ». */
        const val ACTION_NEW = "io.github.aquadrox.voicetotext.meeting.NEW"
        private const val PICK = 1
        private const val PERMISSIONS = 2
    }
}
