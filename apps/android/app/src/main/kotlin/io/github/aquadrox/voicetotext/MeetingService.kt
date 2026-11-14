package io.github.aquadrox.voicetotext

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaRecorder
import android.net.Uri
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import org.json.JSONObject
import java.nio.ByteOrder
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * Réunions : enregistrement au micro, lecture d'un fichier importé, puis
 * transcription et compte rendu une fois l'audio reçu. Service au premier
 * plan (notification permanente) : le téléphone ne l'arrête pas écran éteint,
 * et Android exige ce statut pour garder le micro en arrière-plan.
 *
 * L'audio va au moteur Rust au fil de l'eau (une seconde à la fois) ; rien
 * n'est gardé ici.
 */
class MeetingService : Service() {
    private val store by lazy { Store(this) }
    private val main = Handler(Looper.getMainLooper())
    /** Lectures de fichiers et transcriptions, l'une après l'autre. */
    private val worker = Executors.newSingleThreadExecutor()
    private val queued = mutableSetOf<String>()
    private var recorder: AudioRecord? = null
    private val ticker = object : Runnable {
        override fun run() {
            refreshNotification()
            main.postDelayed(this, TICK_MS)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        channels(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Android exige le passage au premier plan dès le démarrage.
        foreground()
        val id = intent?.getStringExtra(EXTRA_ID).orEmpty()
        when (intent?.action) {
            ACTION_RECORD -> record(id)
            ACTION_MARK -> mark()
            ACTION_STOP -> stopRecording()
            ACTION_IMPORT -> intent.data?.let { importFile(id, it) }
            ACTION_PROCESS -> process(id)
            ACTION_IDENTIFY -> identify(id)
        }
        main.removeCallbacks(ticker)
        main.post(ticker)
        stopIfIdle()
        return START_NOT_STICKY
    }

    /** Android 15 : temps maximal d'un service de traitement atteint. Reprise à la prochaine ouverture. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "Service des réunions arrêté par Android (durée maximale)")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        main.removeCallbacks(ticker)
        recordingId = null
        super.onDestroy()
    }

    // ---- Enregistrement ----

    @SuppressLint("MissingPermission")
    private fun record(id: String) {
        if (id.isEmpty() || recordingId != null) return
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            fail(id, "Micro non autorisé")
            return
        }
        val size = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        // Micro général (gain automatique d'Android) : voix éloignées autour d'une table.
        val audio = try {
            AudioRecord(MediaRecorder.AudioSource.MIC, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(size, RATE * 2))
        } catch (e: Exception) {
            null
        }
        if (audio == null || audio.state != AudioRecord.STATE_INITIALIZED) {
            audio?.release()
            fail(id, "Micro indisponible (pris par une autre application ?)")
            return
        }
        try {
            audio.startRecording()
        } catch (e: Exception) {
            audio.release()
            fail(id, "Micro indisponible : ${e.message}")
            return
        }
        recorder = audio
        recordingId = id
        startedAt = SystemClock.elapsedRealtime()
        receivedMs = 0
        marks = 0
        level = 0f
        foreground()
        val root = store.meetingsDir
        thread(name = "meeting-mic") {
            val buffer = ShortArray(RATE / 10)
            val second = ShortArray(RATE)
            var filled = 0
            while (recordingId == id) {
                val n = audio.read(buffer, 0, buffer.size)
                if (n <= 0) continue
                var peak = 0
                for (i in 0 until n) peak = maxOf(peak, kotlin.math.abs(buffer[i].toInt()))
                level = peak / 32768f
                var offset = 0
                while (offset < n) {
                    val count = minOf(n - offset, second.size - filled)
                    buffer.copyInto(second, filled, offset, offset + count)
                    filled += count
                    offset += count
                    if (filled == second.size) {
                        feed(root, id, second, filled)
                        filled = 0
                    }
                }
            }
            feed(root, id, second, filled)
            runCatching { audio.stop() }
            audio.release()
            level = 0f
            val stopped = JSONObject(Native.meetingStop(root, id))
            main.post {
                if (stopped.has("error")) notifyError(id, stopped.optString("error")) else process(id)
                stopIfIdle()
            }
        }
    }

    private fun feed(root: String, id: String, pcm: ShortArray, count: Int) {
        if (count <= 0) return
        val reply = JSONObject(Native.meetingFeed(root, id, pcm, count, RATE, 1))
        if (reply.has("ok")) receivedMs = reply.getLong("ok") else Log.w(TAG, "réunion $id : ${reply.optString("error")}")
    }

    private fun mark() {
        val id = recordingId ?: return
        val reply = JSONObject(Native.meetingMark(store.meetingsDir, id))
        if (reply.has("ok")) marks++
        refreshNotification()
    }

    private fun stopRecording() {
        recordingId ?: return
        recorder = null
        // Le fil du micro voit le changement, envoie la dernière seconde et termine.
        recordingId = null
        foreground()
    }

    private fun fail(id: String, message: String) {
        Native.meetingStop(store.meetingsDir, id)
        notifyError(id, message)
    }

    // ---- Import ----

    private fun importFile(id: String, uri: Uri) {
        if (id.isEmpty() || !queued.add(id)) return
        importing = id
        importPercent = 0
        foreground()
        val root = store.meetingsDir
        worker.execute {
            val error = try {
                decode(root, id, uri)
                null
            } catch (e: Throwable) {
                Log.e(TAG, "Import illisible", e)
                e.message ?: e.javaClass.simpleName
            }
            val stopped = JSONObject(Native.meetingStop(root, id))
            importing = null
            main.post {
                queued.remove(id)
                when {
                    error != null -> notifyError(id, "Fichier illisible : $error")
                    stopped.has("error") -> notifyError(id, stopped.optString("error"))
                    else -> process(id)
                }
                stopIfIdle()
            }
        }
    }

    /**
     * Décode la piste audio d'un fichier (MP3, M4A, Opus, OGG, AMR, WAV, FLAC,
     * vidéo…) avec les décodeurs d'Android et l'envoie au moteur, une seconde
     * environ à la fois, avec le pourcentage lu.
     */
    private fun decode(root: String, id: String, uri: Uri) {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(this, uri, null)
            val track = (0 until extractor.trackCount).firstOrNull {
                extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: throw IllegalArgumentException("aucune piste audio")
            extractor.selectTrack(track)
            val format = extractor.getTrackFormat(track)
            val duration = if (format.containsKey(MediaFormat.KEY_DURATION)) format.getLong(MediaFormat.KEY_DURATION) else 0L
            val codec = MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME)!!)
            try {
                codec.configure(format, null, null, 0)
                codec.start()
                pump(codec, extractor, root, id, format, duration)
            } finally {
                runCatching { codec.stop() }
                codec.release()
            }
        } finally {
            extractor.release()
        }
    }

    private fun pump(codec: MediaCodec, extractor: MediaExtractor, root: String, id: String, input: MediaFormat, duration: Long) {
        var rate = input.getInteger(MediaFormat.KEY_SAMPLE_RATE)
        var channels = input.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
        var float = false
        val info = MediaCodec.BufferInfo()
        var pending = ShortArray(0)
        var filled = 0
        var inputDone = false
        var percent = 0
        val flush = {
            if (filled > 0) {
                val reply = JSONObject(Native.meetingFeed(root, id, pending, filled, rate, channels))
                if (reply.has("error")) throw IllegalStateException(reply.getString("error"))
                filled = 0
            }
        }
        while (true) {
            if (!inputDone) {
                val index = codec.dequeueInputBuffer(TIMEOUT_US)
                if (index >= 0) {
                    val buffer = codec.getInputBuffer(index)!!
                    val size = extractor.readSampleData(buffer, 0)
                    if (size < 0) {
                        codec.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                        inputDone = true
                    } else {
                        codec.queueInputBuffer(index, 0, size, extractor.sampleTime, 0)
                        if (duration > 0) {
                            val now = (extractor.sampleTime * 100 / duration).toInt().coerceIn(0, 100)
                            if (now > percent) {
                                percent = now
                                importPercent = now
                                Native.meetingProgress(root, id, now)
                            }
                        }
                        extractor.advance()
                    }
                }
            }
            val index = codec.dequeueOutputBuffer(info, TIMEOUT_US)
            when {
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    flush()
                    val output = codec.outputFormat
                    rate = output.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                    channels = output.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                    float = output.containsKey(MediaFormat.KEY_PCM_ENCODING) &&
                        output.getInteger(MediaFormat.KEY_PCM_ENCODING) == AudioFormat.ENCODING_PCM_FLOAT
                }
                index >= 0 -> {
                    val buffer = codec.getOutputBuffer(index)!!
                    buffer.position(info.offset)
                    buffer.limit(info.offset + info.size)
                    buffer.order(ByteOrder.LITTLE_ENDIAN)
                    val count = if (float) info.size / 4 else info.size / 2
                    if (pending.size < filled + count) pending = pending.copyOf(maxOf(filled + count, rate * channels))
                    if (float) {
                        val floats = buffer.asFloatBuffer()
                        for (i in 0 until count) pending[filled + i] = (floats.get(i).coerceIn(-1f, 1f) * 32767).toInt().toShort()
                    } else {
                        buffer.asShortBuffer().get(pending, filled, count)
                    }
                    filled += count
                    codec.releaseOutputBuffer(index, false)
                    // Une seconde environ par envoi.
                    if (filled >= rate * channels) flush()
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                        flush()
                        return
                    }
                }
            }
        }
    }

    // ---- Transcription ----

    private fun process(id: String) {
        if (id.isEmpty() || !queued.add(id)) return
        foreground()
        val root = store.meetingsDir
        worker.execute {
            processing = id
            main.post { foreground() }
            var reply = run(root, id)
            // Modèle libéré ou réglages changés entre-temps : rechargé, puis reprise.
            if (reply.optString("error").startsWith("moteur non démarré")) reply = run(root, id)
            processing = null
            main.post {
                queued.remove(id)
                val error = reply.optString("error")
                if (error.isNotEmpty()) notifyError(id, error) else notifyDone(id)
                stopIfIdle()
            }
        }
    }

    /** Intervenants d'une réunion déjà transcrite (audio gardé). */
    private fun identify(id: String) {
        if (id.isEmpty() || !queued.add(id)) return
        foreground()
        val root = store.meetingsDir
        val models = store.modelsDir
        worker.execute {
            processing = id
            main.post { foreground() }
            val reply = try {
                JSONObject(Native.meetingIdentify(root, id, models))
            } catch (e: Throwable) {
                JSONObject().put("error", e.message ?: e.javaClass.simpleName)
            }
            processing = null
            main.post {
                queued.remove(id)
                val error = reply.optString("error")
                if (error.isNotEmpty()) notifyError(id, "Intervenants : $error") else done(id, "Intervenants distingués", "Donnez-leur un nom dans la transcription.")
                stopIfIdle()
            }
        }
    }

    /** Charge le modèle (fil principal), puis transcrit. */
    private fun run(root: String, id: String): JSONObject {
        val latch = CountDownLatch(1)
        var error: String? = null
        main.post {
            Dictation.prepare(store) {
                error = it
                latch.countDown()
            }
        }
        latch.await(10, TimeUnit.MINUTES)
        error?.let { return JSONObject().put("error", it) }
        return try {
            JSONObject(Native.meetingProcess(root, id, store.config, store.modelsDir))
        } catch (e: Throwable) {
            Log.e(TAG, "Transcription", e)
            JSONObject().put("error", e.message ?: e.javaClass.simpleName)
        }
    }

    // ---- Notifications ----

    /** Premier plan, avec le type qu'exige Android pour ce qui tourne. */
    private fun foreground() {
        val notification = notification()
        val type = if (recordingId != null) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        } else {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        }
        try {
            startForeground(NOTIFY_ONGOING, notification, type)
        } catch (e: Exception) {
            // Android refuse (application en arrière-plan) : rien ne tourne.
            Log.e(TAG, "Premier plan refusé", e)
            recordingId = null
            stopSelf()
        }
    }

    private fun refreshNotification() {
        if (recordingId == null && processing == null && importing == null) return
        getSystemService(NotificationManager::class.java).notify(NOTIFY_ONGOING, notification())
    }

    private fun notification(): Notification {
        val id = recordingId
        val builder = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_SERVICE)
        if (id != null) {
            val open = PendingIntent.getActivity(
                this, 10, Intent(this, RecordingActivity::class.java).putExtra(EXTRA_ID, id),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val marked = if (marks > 0) " · $marks moment(s) marqué(s)" else ""
            return builder
                .setContentTitle("Réunion en cours")
                .setContentText("Enregistrement du micro$marked")
                .setUsesChronometer(true)
                .setWhen(System.currentTimeMillis() - (SystemClock.elapsedRealtime() - startedAt))
                .setContentIntent(open)
                .addAction(Notification.Action.Builder(null, "Marquer un moment", action(ACTION_MARK, 11)).build())
                .addAction(Notification.Action.Builder(null, "Arrêter", action(ACTION_STOP, 12)).build())
                .build()
        }
        val busy = importing ?: processing
        val open = PendingIntent.getActivity(
            this, 13,
            if (busy != null) MeetingActivity.intent(this, busy) else Intent(this, MeetingsActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        builder.setContentIntent(open)
        if (importing != null) {
            return builder.setContentTitle("Import d'un enregistrement")
                .setContentText("Lecture du fichier : $importPercent %")
                .setProgress(100, importPercent, false)
                .build()
        }
        if (busy == null) return builder.setContentTitle("Réunions").setContentText("Préparation…").build()
        val detail = JSONObject(Native.meeting(store.meetingsDir, busy)).optJSONObject("ok")
        val title = detail?.optString("title").orEmpty()
        if (detail?.optString("step") == "speakers") {
            val percent = detail.optInt("progress", 0)
            return builder.setContentTitle("Intervenants : $percent %").setContentText(title).setProgress(100, percent, false).build()
        }
        if (detail?.optString("step") == "summary") {
            return builder.setContentTitle("Compte rendu en cours").setContentText(title).setProgress(0, 0, true).build()
        }
        val percent = detail?.optInt("progress", 0) ?: 0
        return builder.setContentTitle("Transcription : $percent %")
            .setContentText(title)
            .setProgress(100, percent, false)
            .build()
    }

    private fun action(name: String, code: Int): PendingIntent = PendingIntent.getService(
        this, code, Intent(this, MeetingService::class.java).setAction(name),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun notifyDone(id: String) {
        val detail = JSONObject(Native.meeting(store.meetingsDir, id)).optJSONObject("ok") ?: return
        val summary = detail.optBoolean("summary")
        val error = detail.optString("summary_error").takeIf { it.isNotEmpty() && it != "null" }
        val text = when {
            summary -> "Compte rendu prêt"
            error != null -> "Transcription prête ; compte rendu en échec"
            else -> "Transcription prête"
        }
        done(id, text, detail.optString("title"))
    }

    private fun notifyError(id: String, message: String) {
        Log.w(TAG, "réunion $id : $message")
        done(id, "Réunion : échec", message)
    }

    private fun done(id: String, title: String, text: String) {
        val manager = getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return
        val open = PendingIntent.getActivity(
            this, id.hashCode(), MeetingActivity.intent(this, id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(this, CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        manager.notify(NOTIFY_DONE + (id.hashCode() and 0xFFFF), notification)
    }

    /** Plus rien à faire : la notification permanente disparaît. */
    private fun stopIfIdle() {
        if (recordingId != null || queued.isNotEmpty()) return
        main.removeCallbacks(ticker)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    companion object {
        private const val TAG = "VoiceToText"
        const val ACTION_RECORD = "io.github.aquadrox.voicetotext.meeting.RECORD"
        const val ACTION_MARK = "io.github.aquadrox.voicetotext.meeting.MARK"
        const val ACTION_STOP = "io.github.aquadrox.voicetotext.meeting.STOP"
        const val ACTION_IMPORT = "io.github.aquadrox.voicetotext.meeting.IMPORT"
        const val ACTION_PROCESS = "io.github.aquadrox.voicetotext.meeting.PROCESS"
        const val ACTION_IDENTIFY = "io.github.aquadrox.voicetotext.meeting.IDENTIFY"
        const val EXTRA_ID = "id"
        private const val CHANNEL = "meetings"
        private const val CHANNEL_DONE = "meetings_done"
        private const val NOTIFY_ONGOING = 4200
        private const val NOTIFY_DONE = 0x10000
        private const val RATE = 16_000
        private const val TICK_MS = 2_000L
        private const val TIMEOUT_US = 10_000L

        /** Réunion en cours d'enregistrement (null : aucune). */
        @Volatile
        var recordingId: String? = null
            private set

        /** Début de l'enregistrement (horloge depuis le démarrage du téléphone). */
        @Volatile
        var startedAt = 0L
            private set

        /** Audio reçu par le moteur (ms). */
        @Volatile
        var receivedMs = 0L
            private set

        /** Niveau du micro (0–1). */
        @Volatile
        var level = 0f
            private set

        @Volatile
        var marks = 0
            private set

        /** Réunion en cours de transcription. */
        @Volatile
        var processing: String? = null
            private set

        /** Fichier en cours de lecture, et pourcentage lu. */
        @Volatile
        var importing: String? = null
            private set

        @Volatile
        var importPercent = 0
            private set

        /** Le micro est pris par une réunion : pas de dictée en même temps. */
        val recording: Boolean
            get() = recordingId != null

        /** Quelque chose tourne (enregistrement, import, transcription). */
        val busy: Boolean
            get() = recordingId != null || processing != null || importing != null

        fun start(context: Context, action: String, id: String = "", data: Uri? = null) {
            val intent = Intent(context, MeetingService::class.java).setAction(action).putExtra(EXTRA_ID, id)
            if (data != null) {
                intent.data = data
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            context.startForegroundService(intent)
        }

        fun channels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "Réunion en cours", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Enregistrement, import et transcription d'une réunion"
                },
            )
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_DONE, "Réunion terminée", NotificationManager.IMPORTANCE_DEFAULT).apply {
                    description = "Transcription ou compte rendu prêt"
                },
            )
        }
    }
}
