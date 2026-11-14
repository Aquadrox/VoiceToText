package io.github.aquadrox.voicetotext

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Bundle
import android.text.InputType
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.PopupMenu
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import io.github.aquadrox.voicetotext.Meetings.markdown
import io.github.aquadrox.voicetotext.Meetings.objects
import io.github.aquadrox.voicetotext.Meetings.text
import org.json.JSONObject
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.Executors

/**
 * Une réunion : suivi de la transcription (étapes cochées), puis deux onglets
 * « Compte rendu » et « Transcription », « Partager » et « Copier » en bas,
 * menu ⋯ pour le reste (renommer, modèle de compte rendu, refaire le compte
 * rendu, supprimer).
 */
class MeetingActivity : Activity() {
    private val store by lazy { Store(this) }
    private val worker = Executors.newSingleThreadExecutor()
    private var meetingId = ""
    private lateinit var title: TextView
    private lateinit var subtitle: TextView
    private lateinit var message: TextView
    private lateinit var tabs: LinearLayout
    private lateinit var summaryTab: TextView
    private lateinit var transcriptTab: TextView
    private lateinit var body: LinearLayout
    private lateinit var share: Button
    private lateinit var copy: Button
    private var detail: JSONObject? = null
    private var showTranscript = false
    /** Signature du dernier affichage : rien n'est redessiné si rien n'a changé. */
    private var shown = ""
    private var confirmDelete = false
    private var player: AudioTrack? = null
    private var playing = -1L
    private val poll = object : Runnable {
        override fun run() = load()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        meetingId = intent.getStringExtra(MeetingService.EXTRA_ID).orEmpty()
        screen(bar = {
            share = primary("Partager") { share() }.apply {
                setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_share, 0, 0, 0)
                compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.accent_fg))
            }
            copy = secondary("Copier") { copy() }.apply {
                setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_copy, 0, 0, 0)
                compoundDrawableTintList = ColorStateList.valueOf(getColor(R.color.fg))
            }
        }) {
            val head = LinearLayout(this@MeetingActivity).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.TOP
            }
            val texts = LinearLayout(this@MeetingActivity).apply { orientation = LinearLayout.VERTICAL }
            subtitle = texts.header("", "", onBack = { finish() })
            title = (subtitle.parent as LinearLayout).getChildAt(0) as TextView
            head.addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            lateinit var more: View
            more = iconButton(R.drawable.ic_ellipsis, "Plus d'actions") { menu(more) }.apply {
                imageTintList = ColorStateList.valueOf(getColor(R.color.fg))
            }
            head.addView(
                more,
                LinearLayout.LayoutParams(dp(44), dp(44)).apply { topMargin = dp(8) },
            )
            addView(head)
            message = notice()
            tabs = LinearLayout(this@MeetingActivity).apply {
                orientation = LinearLayout.HORIZONTAL
                background = rounded(getColor(R.color.border), 12f)
                setPadding(dp(4), dp(4), dp(4), dp(4))
            }
            summaryTab = tabs.tab("Compte rendu") { showTranscript = false; render(true) }
            transcriptTab = tabs.tab("Transcription") { showTranscript = true; render(true) }
            addView(tabs, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(8) })
            body = LinearLayout(this@MeetingActivity).apply { orientation = LinearLayout.VERTICAL }
            addView(body)
        }
    }

    override fun onResume() {
        super.onResume()
        load()
    }

    override fun onPause() {
        body.removeCallbacks(poll)
        stopPlayer()
        super.onPause()
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    private fun load() {
        body.removeCallbacks(poll)
        val reply = JSONObject(Native.meeting(store.meetingsDir, meetingId))
        val detail = reply.optJSONObject("ok")
        if (detail == null) {
            message.say(reply.optString("error"))
            return
        }
        // Première ouverture sans compte rendu (transcription seule) : la transcription.
        if (this.detail == null && detail.optJSONObject("summary_text") == null &&
            detail.text("summary_error").isEmpty() && detail.optString("status") == "terminee"
        ) {
            showTranscript = true
        }
        this.detail = detail
        render(false)
        if (MeetingService.busy || detail.optString("status") in listOf("enregistrement", "transcription") ||
            detail.text("step").isNotEmpty()
        ) {
            body.postDelayed(poll, 1_000)
        }
    }

    private fun render(force: Boolean) {
        val detail = detail ?: return
        val signature = "$showTranscript|$playing|$detail"
        if (!force && signature == shown) return
        shown = signature
        title.text = detail.optString("title")
        subtitle.text = "${Meetings.date(detail.getLong("started_at_ms"))} · ${Meetings.clock(detail.optLong("duration_ms"))}"
        subtitle.visibility = View.VISIBLE
        body.removeAllViews()
        val status = detail.optString("status")
        val working = status == "enregistrement" || status == "transcription" || detail.text("step") == "transcription"
        tabs.visibility = if (working) View.GONE else View.VISIBLE
        share.isEnabled = !working
        copy.isEnabled = !working
        if (working) {
            progress(detail)
            return
        }
        summaryTab.select(!showTranscript)
        transcriptTab.select(showTranscript)
        if (showTranscript) transcript(detail) else summary(detail)
    }

    // ---- Pendant le traitement ----

    private fun progress(detail: JSONObject) {
        val status = detail.optString("status")
        val imported = detail.optBoolean("imported")
        if (status == "enregistrement" && MeetingService.recordingId == meetingId) {
            body.card().apply {
                note("Enregistrement en cours.")
                primary("Revenir à l'enregistrement") { startActivity(RecordingActivity.intent(this@MeetingActivity, meetingId)) }
            }
            return
        }
        val card = body.card()
        card.step(if (imported) "Lecture du fichier" else "Enregistrement").apply {
            val reading = status == "enregistrement"
            set(
                !reading,
                if (reading) "${detail.optInt("progress")} %"
                else Meetings.clock(detail.optLong("duration_ms")) + marksText(detail),
            )
        }
        val transcribing = detail.text("step") == "transcription"
        card.step("Transcription").apply {
            set(
                false,
                when {
                    transcribing -> "${detail.optInt("progress")} % · modèle de la dictée, sur le téléphone"
                    detail.text("step") == "speakers" -> "Terminée"
                    status == "transcription" -> "En attente…"
                    else -> ""
                },
            )
        }
        if (transcribing) {
            card.addView(
                ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
                    max = 100
                    progress = detail.optInt("progress")
                    progressTintList = ColorStateList.valueOf(getColor(R.color.accent))
                },
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT),
            )
        }
        if (detail.optBoolean("speakers_pending") || detail.text("step") == "speakers") {
            val analyzing = detail.text("step") == "speakers"
            card.step("Intervenants").set(false, if (analyzing) "${detail.optInt("progress")} % · voix analysées sur le téléphone" else "Après la transcription")
        }
        card.step("Compte rendu").set(false, if (detail.optBoolean("conference")) "Modèle cours ou conférence" else "Modèle réunion")
        body.note(
            "Vous pouvez quitter l'appli : une notification vous préviendra. Le clavier et la bulle restent utilisables ; " +
                "la transcription s'interrompt le temps de vos dictées.",
        )
        val replies = detail.optJSONArray("replies")?.objects().orEmpty()
        if (replies.isNotEmpty()) {
            body.card("Premières phrases").apply {
                for (reply in replies.take(3)) {
                    text("${Meetings.clock(reply.getLong("start_ms"))}  ${reply.optString("text")}", 15f)
                        .setPadding(0, dp(4), 0, dp(4))
                }
            }
        }
    }

    private fun marksText(detail: JSONObject): String {
        val count = detail.optJSONArray("marks")?.length() ?: 0
        return if (count > 0) " · $count moment(s) marqué(s)" else ""
    }

    // ---- Compte rendu ----

    private fun summary(detail: JSONObject) {
        val summary = detail.optJSONObject("summary_text")
        val error = detail.text("summary_error")
        val step = detail.text("step")
        notes(detail)
        when {
            step == "summary" -> body.card().apply {
                note("Compte rendu en cours de rédaction par le LLM…")
                addView(ProgressBar(this@MeetingActivity, null, android.R.attr.progressBarStyleHorizontal).apply {
                    isIndeterminate = true
                })
            }
            summary != null -> {
                body.card().apply { markdown(summary.optString("text")) }
                val cost = if (summary.isNull("cost")) "" else " · " + Meetings.cost(summary.optDouble("cost"), summary.optString("currency"))
                body.note("Rédigé par ${summary.optString("model")}$cost")
            }
            else -> body.card().apply {
                if (error.isNotEmpty()) note("Le compte rendu a échoué : $error")
                else if (detail.optString("status") == "interrompue") note("Enregistrement interrompu.")
                else note("Pas encore de compte rendu.")
                if (detail.optJSONArray("replies")?.length() ?: 0 > 0) {
                    primary(if (error.isNotEmpty()) "Réessayer" else "Rédiger le compte rendu") { summarize() }
                }
            }
        }
    }

    private fun notes(detail: JSONObject) {
        val notes = detail.optJSONArray("notes")?.let { a -> List(a.length()) { a.getString(it) } }.orEmpty()
        val failed = detail.optInt("failed_chunks")
        val lines = notes + if (failed > 0) listOf("$failed passage(s) non transcrit(s).") else emptyList()
        if (lines.isNotEmpty()) {
            body.text(lines.joinToString("\n"), 13f, R.color.warning_fg).apply {
                background = rounded(getColor(R.color.warning_bg), 12f)
                setPadding(dp(12), dp(10), dp(12), dp(10))
                (layoutParams as LinearLayout.LayoutParams).topMargin = dp(12)
            }
        }
    }

    private fun summarize() {
        if (detail?.text("step")?.isNotEmpty() == true) return
        val root = store.meetingsDir
        val config = store.config
        message.say("")
        worker.execute {
            // Clé API du fournisseur chargée dans le moteur (comme pour une dictée).
            val latch = java.util.concurrent.CountDownLatch(1)
            runOnUiThread { Dictation.prepare(store) { latch.countDown() } }
            latch.await(2, java.util.concurrent.TimeUnit.MINUTES)
            val reply = JSONObject(Native.meetingSummarize(root, meetingId, config, ""))
            runOnUiThread {
                if (isDestroyed) return@runOnUiThread
                reply.text("error").takeIf { it.isNotEmpty() }?.let { message.say("Compte rendu : $it") }
                load()
            }
        }
        body.postDelayed({ load() }, 300)
    }

    // ---- Transcription ----

    private fun transcript(detail: JSONObject) {
        val replies = detail.optJSONArray("replies")?.objects().orEmpty()
        val marks = detail.optJSONArray("marks")?.let { a -> List(a.length()) { a.getLong(it) } }.orEmpty()
        val replay = detail.optBoolean("replay")
        notes(detail)
        if (replies.isEmpty()) {
            body.card().note("Rien n'a été transcrit (silence, ou audio illisible).")
            return
        }
        speakers(detail)
        body.note(if (replay) "Touchez une phrase pour la copier, ▶ pour la réécouter." else "Touchez une phrase pour la copier.")
        val card = body.card().apply {
            setPadding(0, dp(2), 0, dp(2))
            clipToOutline = true
        }
        var markIndex = 0
        replies.forEachIndexed { index, reply ->
            val start = reply.getLong("start_ms")
            while (markIndex < marks.size && marks[markIndex] <= start) {
                card.mark(marks[markIndex++])
            }
            if (card.childCount > 0) card.divider(0)
            val end = replies.getOrNull(index + 1)?.getLong("start_ms") ?: (start + CLIP_MS)
            card.addView(reply(reply, start, minOf(end, start + CLIP_MS), replay))
        }
        while (markIndex < marks.size) card.mark(marks[markIndex++])
    }

    private fun reply(reply: JSONObject, start: Long, end: Long, replay: Boolean): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(16), dp(10), dp(4), dp(10))
        }
        val texts = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = pressable(null, 8f)
            setOnClickListener { Meetings.copy(this@MeetingActivity, "Réplique", reply.optString("text")) }
        }
        val label = reply.text("label")
        val speaker = reply.optInt("speaker", -1)
        val head = (if (label.isEmpty()) "" else "$label · ") + Meetings.clock(start)
        texts.text(head, 13f, if (label.isEmpty()) R.color.muted else R.color.fg, bold = label.isNotEmpty()).apply {
            if (speaker >= 0) setTextColor(speakerColor(speaker))
        }
        texts.text(reply.optString("text"), 15f)
        row.addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        if (replay) {
            val playingThis = playing == start
            row.addView(
                ImageButton(this).apply {
                    setImageResource(if (playingThis) R.drawable.ic_pause else R.drawable.ic_play)
                    imageTintList = ColorStateList.valueOf(getColor(if (playingThis) R.color.signal else R.color.muted))
                    background = pressable(null, 22f)
                    contentDescription = if (playingThis) "Arrêter la lecture" else "Réécouter à ${Meetings.clock(start)}"
                    setPadding(dp(12), dp(12), dp(12), dp(12))
                    setOnClickListener { if (playingThis) stopPlayer() else play(start, end) }
                },
                LinearLayout.LayoutParams(dp(44), dp(44)),
            )
        }
        return row
    }

    /** Couleur d'un intervenant, lisible en clair comme en sombre. */
    private fun speakerColor(speaker: Int): Int {
        val night = resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        val colors = if (night) SPEAKERS_NIGHT else SPEAKERS
        return colors[speaker % colors.size].toInt()
    }

    private fun LinearLayout.mark(at: Long) {
        if (childCount > 0) divider(0)
        text("⚑  Moment marqué · ${Meetings.clock(at)}", 13f, R.color.signal, bold = true).apply {
            setPadding(dp(16), dp(8), dp(16), dp(8))
        }
    }

    // ---- Intervenants ----

    /** Pastilles des intervenants (toucher : les nommer), ou analyse en cours. */
    private fun speakers(detail: JSONObject) {
        if (detail.text("step") == "speakers") {
            body.card().apply {
                note("Intervenants : ${detail.optInt("progress")} % (voix analysées sur le téléphone)…")
                addView(ProgressBar(this@MeetingActivity, null, android.R.attr.progressBarStyleHorizontal).apply {
                    max = 100
                    progress = detail.optInt("progress")
                    progressTintList = ColorStateList.valueOf(getColor(R.color.accent))
                })
            }
            return
        }
        val list = detail.optJSONArray("speaker_list")?.objects().orEmpty()
        if (list.isEmpty()) return
        val flow = FlowLayout(this).apply { setPadding(0, dp(10), 0, 0) }
        for (speaker in list) {
            val name = speaker.text("name").ifEmpty { speaker.optString("label") }
            flow.addView(TextView(this).apply {
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                val dot = speakerColor(speaker.optInt("index"))
                text = android.text.SpannableString("●  $name").apply {
                    setSpan(android.text.style.ForegroundColorSpan(dot), 0, 1, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
                }
                background = pressable(rounded(getColor(R.color.card), 16f, getColor(R.color.border_strong)), 16f)
                setPadding(dp(12), dp(8), dp(12), dp(8))
                minHeight = dp(36)
                gravity = Gravity.CENTER_VERTICAL
                setOnClickListener { speakersDialog() }
            })
        }
        flow.addView(TextView(this).apply {
            text = "Nommer…"
            setTextColor(getColor(R.color.fg))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
            typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
            background = pressable(null, 16f)
            setPadding(dp(12), dp(8), dp(12), dp(8))
            minHeight = dp(36)
            gravity = Gravity.CENTER_VERTICAL
            setOnClickListener { speakersDialog() }
        })
        body.addView(flow)
    }

    /** Fenêtre Intervenants : nom, temps de parole, première phrase, écoute ; nombre imposé. */
    private fun speakersDialog() {
        val detail = detail ?: return
        val list = detail.optJSONArray("speaker_list")?.objects().orEmpty()
        val dialog = android.app.Dialog(this)
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(10), dp(16), dp(20))
            background = android.graphics.drawable.GradientDrawable().apply {
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
        column.text("Intervenants", 20f, bold = true)
        column.note("Qui se présente (« je m'appelle Marie ») est nommé d'office. Donnez ou corrigez un nom : il remplace « Intervenant N » partout (transcription, partage, prochain compte rendu).")
        val fields = mutableListOf<Triple<Int, EditText, String?>>()
        for (speaker in list) {
            val index = speaker.optInt("index")
            column.divider(6)
            val head = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
            head.addView(
                TextView(this).apply {
                    text = speaker.optString("label")
                    setTextColor(speakerColor(index))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                    typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
                },
                LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f),
            )
            head.text(Meetings.clock(speaker.optLong("talk_ms")) + " de parole", 13f, R.color.muted)
            column.addView(head)
            column.text("« ${speaker.optString("first")} »", 14f, R.color.muted).setTypeface(null, Typeface.ITALIC)
            val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
            column.addView(row)
            val field = row.field("Nom").apply { setText(speaker.text("name")) }
            val found = speaker.text("found")
            // Nom trouvé dans sa présentation : seulement s'il est changé, il devient un nom donné.
            fields += Triple(index, field, speaker.text("name").takeIf { found.isNotEmpty() })
            if (detail.optBoolean("replay")) {
                val start = speaker.optLong("first_ms")
                row.addView(
                    ImageButton(this).apply {
                        setImageResource(R.drawable.ic_play)
                        imageTintList = ColorStateList.valueOf(getColor(R.color.fg))
                        background = pressable(rounded(getColor(R.color.card), 10f, getColor(R.color.border_strong)), 10f)
                        contentDescription = "Écouter sa voix"
                        setOnClickListener { play(start, start + VOICE_SAMPLE_MS, redraw = false) }
                    },
                    LinearLayout.LayoutParams(dp(48), dp(48)).apply {
                        marginStart = dp(8)
                        topMargin = dp(8)
                    },
                )
            }
            if (found.isNotEmpty()) {
                column.text("Présentation à ${Meetings.clock(speaker.optLong("found_ms"))} : « $found »", 13f, R.color.muted)
                    .setTypeface(null, Typeface.ITALIC)
            }
        }
        column.divider(6)
        column.text("Nombre d'intervenants", 15f, bold = true)
        column.note("À imposer si deux voix sont confondues ou une voix coupée en deux.")
        val choices = listOf("Selon les voix") + (1..8).map { it.toString() }
        val count = android.widget.Spinner(this).apply {
            adapter = android.widget.ArrayAdapter(this@MeetingActivity, android.R.layout.simple_spinner_dropdown_item, choices)
            setSelection(if (detail.isNull("speaker_count")) 0 else detail.optInt("speaker_count").coerceIn(0, 8))
            isEnabled = detail.optBoolean("voices_kept")
        }
        column.addView(count)
        if (!detail.optBoolean("voices_kept")) column.note("Analyse des voix non gardée (réunion en mémoire seulement) : nombre fixe.")
        val initialCount = count.selectedItemPosition
        column.primary("Enregistrer") {
            stopPlayer()
            for ((index, field, guess) in fields) {
                if (field.text.toString().trim() == guess) continue
                Native.meetingRenameSpeaker(store.meetingsDir, meetingId, index, field.text.toString())
            }
            if (count.selectedItemPosition != initialCount) {
                val reply = JSONObject(Native.meetingSpeakerCount(store.meetingsDir, meetingId, count.selectedItemPosition))
                message.say(reply.text("error"))
            }
            if (detail.optJSONObject("summary_text") != null) {
                message.say("Noms enregistrés. Menu ⋯ → « Refaire le compte rendu » pour les y reprendre.")
            }
            dialog.dismiss()
            load()
        }
        column.secondary("Ne plus distinguer") {
            stopPlayer()
            Native.meetingForgetSpeakers(store.meetingsDir, meetingId)
            dialog.dismiss()
            load()
        }.apply { setTextColor(getColor(R.color.danger)) }
        dialog.setContentView(android.widget.ScrollView(this).apply { addView(column) })
        dialog.setOnDismissListener { stopPlayer() }
        dialog.window?.apply {
            setBackgroundDrawable(android.graphics.drawable.ColorDrawable(android.graphics.Color.TRANSPARENT))
            setGravity(Gravity.BOTTOM)
            setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        }
        dialog.show()
    }

    /** Distinguer les intervenants après coup : modèles d'abord si besoin. */
    private fun identify() {
        val installed = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")?.optBoolean("voices") == true
        if (installed) {
            MeetingService.start(this, MeetingService.ACTION_IDENTIFY, meetingId)
            body.postDelayed({ load() }, 500)
            return
        }
        message.say("Téléchargement des modèles des intervenants…")
        Meetings.Voices.download(this, store) { error ->
            if (isDestroyed) return@download
            if (error != null) {
                message.say("Téléchargement échoué : $error")
            } else {
                message.say("")
                MeetingService.start(this, MeetingService.ACTION_IDENTIFY, meetingId)
                body.postDelayed({ load() }, 500)
            }
        }
    }

    // ---- Réécoute ----

    private fun play(start: Long, end: Long, redraw: Boolean = true) {
        stopPlayer()
        val pcm = Native.meetingClip(store.meetingsDir, meetingId, start, end)
        if (pcm == null || pcm.isEmpty()) {
            Toast.makeText(this, "Audio indisponible", Toast.LENGTH_SHORT).show()
            return
        }
        val track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            .setAudioFormat(AudioFormat.Builder().setSampleRate(16_000).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(pcm.size * 2)
            .build()
        track.write(pcm, 0, pcm.size)
        track.notificationMarkerPosition = pcm.size
        track.setPlaybackPositionUpdateListener(object : AudioTrack.OnPlaybackPositionUpdateListener {
            override fun onMarkerReached(t: AudioTrack?) = stopPlayer()
            override fun onPeriodicNotification(t: AudioTrack?) {}
        })
        track.play()
        player = track
        if (redraw) {
            playing = start
            render(true)
        }
    }

    private fun stopPlayer() {
        player?.let {
            runCatching { it.stop() }
            it.release()
        }
        player = null
        if (playing >= 0) {
            playing = -1
            if (::body.isInitialized) render(true)
        }
    }

    // ---- Actions ----

    private fun dateText(): String = detail?.let {
        DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(it.getLong("started_at_ms")))
    }.orEmpty()

    /** Ce qui est affiché : le compte rendu (s'il existe) ou la transcription. */
    private fun shownText(): Pair<String, String>? {
        val what = if (showTranscript || detail?.optJSONObject("summary_text") == null) "transcript" else "summary"
        val text = JSONObject(Native.meetingExport(store.meetingsDir, meetingId, what, dateText())).text("ok")
        if (text.isEmpty()) return null
        val label = if (what == "summary") "Compte rendu" else "Transcription"
        return label to (if (what == "transcript") "${detail?.optString("title")}\n\n$text" else text)
    }

    private fun share() {
        val (label, text) = shownText() ?: return
        Meetings.share(this, "$label : ${detail?.optString("title")}", text)
    }

    private fun copy() {
        val (label, text) = shownText() ?: return
        Meetings.copy(this, label, text)
    }

    private fun menu(anchor: View) {
        val detail = detail ?: return
        val busy = detail.optString("status") in listOf("enregistrement", "transcription") || detail.text("step").isNotEmpty()
        PopupMenu(this, anchor).apply {
            menu.add(0, 1, 0, "Renommer")
            menu.add(0, 2, 0, if (detail.optBoolean("conference")) "Compte rendu : passer au modèle réunion" else "Compte rendu : passer au modèle cours")
                .isEnabled = !busy
            menu.add(0, 3, 0, "Refaire le compte rendu").isEnabled = !busy && detail.optJSONArray("replies")?.length() ?: 0 > 0
            val speakers = detail.optJSONArray("speaker_list")?.length() ?: 0
            if (speakers > 0) {
                menu.add(0, 6, 0, "Intervenants…").isEnabled = !busy
            } else if (detail.optString("status") == "terminee" && detail.optBoolean("replay")) {
                menu.add(0, 7, 0, "Distinguer les intervenants").isEnabled = !busy
            }
            menu.add(0, 4, 0, "Partager tout (Markdown)").isEnabled = !busy
            menu.add(0, 5, 0, "Supprimer")
            setOnMenuItemClickListener {
                when (it.itemId) {
                    1 -> rename()
                    2 -> {
                        Native.meetingSetConference(store.meetingsDir, meetingId, !detail.optBoolean("conference"))
                        message.say("Modèle changé : « Refaire le compte rendu » pour l'appliquer.")
                        load()
                    }
                    3 -> summarize()
                    4 -> JSONObject(Native.meetingExport(store.meetingsDir, meetingId, "markdown", dateText())).text("ok")
                        .takeIf { t -> t.isNotEmpty() }?.let { t -> Meetings.share(this@MeetingActivity, detail.optString("title"), t) }
                    5 -> delete()
                    6 -> speakersDialog()
                    7 -> identify()
                }
                true
            }
        }.show()
    }

    private fun rename() {
        val field = EditText(this).apply {
            setText(detail?.optString("title"))
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            setSelection(text.length)
        }
        AlertDialog.Builder(this)
            .setTitle("Renommer")
            .setView(LinearLayout(this).apply {
                setPadding(dp(20), dp(8), dp(20), 0)
                addView(field, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            })
            .setPositiveButton("Enregistrer") { _, _ ->
                val reply = JSONObject(Native.meetingRename(store.meetingsDir, meetingId, field.text.toString()))
                message.say(reply.text("error"))
                load()
            }
            .setNegativeButton("Annuler", null)
            .show()
    }

    /** Suppression en deux appuis, comme l'historique. */
    private fun delete() {
        if (!confirmDelete) {
            confirmDelete = true
            message.say("Supprimer la réunion, sa transcription, son compte rendu et son audio ? Menu ⋯ → Supprimer de nouveau pour confirmer.")
            return
        }
        if (MeetingService.recordingId == meetingId) MeetingService.start(this, MeetingService.ACTION_STOP, meetingId)
        stopPlayer()
        Native.meetingDelete(store.meetingsDir, meetingId)
        Toast.makeText(this, "Réunion supprimée", Toast.LENGTH_SHORT).show()
        finish()
    }

    private fun LinearLayout.tab(label: String, action: () -> Unit): TextView = TextView(context).apply {
        text = label
        gravity = Gravity.CENTER
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        minHeight = dp(44)
        setOnClickListener { action() }
    }.also { addView(it, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)) }

    private fun TextView.select(selected: Boolean) {
        background = if (selected) rounded(getColor(R.color.card), 9f) else null
        setTextColor(getColor(if (selected) R.color.fg else R.color.muted))
        isSelected = selected
    }

    companion object {
        /** « Écouter sa voix » : début de sa première réplique. */
        private const val VOICE_SAMPLE_MS = 8_000L

        /** Réécoute : jusqu'à la réplique suivante, une minute au plus. */
        private const val CLIP_MS = 60_000L

        /** Couleurs des intervenants (contraste ≥ 4,5:1 sur les cartes claires). */
        private val SPEAKERS = longArrayOf(0xFF1F64C0, 0xFFC2410C, 0xFF23773A, 0xFF8A2BA3, 0xFFB0204F, 0xFF0B7285)
        private val SPEAKERS_NIGHT = longArrayOf(0xFF74A9F0, 0xFFF59E6B, 0xFF6CCB86, 0xFFD08BE3, 0xFFF07FA6, 0xFF5BC6D6)

        fun intent(context: Context, meetingId: String): Intent =
            Intent(context, MeetingActivity::class.java).putExtra(MeetingService.EXTRA_ID, meetingId)
    }
}
