package io.github.aquadrox.voicetotext

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import kotlin.concurrent.thread

/**
 * Micro en 16 kHz mono, en mémoire seulement (jamais écrit sur le stockage).
 * Ouvert uniquement pendant la dictée.
 */
class Recorder {
    private var record: AudioRecord? = null
    private var reader: Thread? = null
    private val lock = Any()
    private var samples = ShortArray(RATE * 10)
    private var count = 0

    @Volatile
    var level = 0f
        private set

    /**
     * Son le plus fort de la dernière prise : 0 si le micro n'a rendu que du
     * silence (Android coupe le micro d'une appli qu'il juge en arrière-plan).
     */
    @Volatile
    var loudest = 0f
        private set

    /** La permission micro doit avoir été accordée (vérifiée par l'appelant). */
    @SuppressLint("MissingPermission")
    fun start(): Boolean {
        if (record != null) return true
        val size = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val audio = AudioRecord(
            MediaRecorder.AudioSource.VOICE_RECOGNITION,
            RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            maxOf(size, RATE / 5 * 2),
        )
        if (audio.state != AudioRecord.STATE_INITIALIZED) {
            audio.release()
            return false
        }
        synchronized(lock) { count = 0 }
        loudest = 0f
        audio.startRecording()
        record = audio
        reader = thread(name = "micro") {
            val buffer = ShortArray(RATE / 20)
            while (record === audio) {
                val n = audio.read(buffer, 0, buffer.size)
                if (n <= 0) continue
                var peak = 0
                synchronized(lock) {
                    if (count + n > samples.size) samples = samples.copyOf(samples.size * 2)
                    buffer.copyInto(samples, count, 0, n)
                    count += n
                }
                for (i in 0 until n) peak = maxOf(peak, kotlin.math.abs(buffer[i].toInt()))
                level = peak / 32768f
                if (level > loudest) loudest = level
            }
        }
        return true
    }

    /** Arrête et renvoie l'audio (flottants entre -1 et 1). */
    fun stop(): FloatArray {
        val audio = record ?: return FloatArray(0)
        record = null
        reader?.join(500)
        reader = null
        audio.stop()
        audio.release()
        level = 0f
        return synchronized(lock) {
            FloatArray(count) { samples[it] / 32768f }.also { count = 0 }
        }
    }

    val recording: Boolean
        get() = record != null

    private companion object {
        const val RATE = 16_000
    }
}
