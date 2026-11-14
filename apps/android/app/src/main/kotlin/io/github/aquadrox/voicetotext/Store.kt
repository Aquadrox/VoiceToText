package io.github.aquadrox.voicetotext

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Réglages (config.toml, le même format que sur PC), modèles et clés API.
 * Les clés sont chiffrées par une clé du Keystore Android, qui ne quitte
 * jamais le matériel sécurisé du téléphone ; elles ne sont jamais affichées.
 */
class Store(private val context: Context) {
    private val prefs = context.getSharedPreferences("voicetotext", Context.MODE_PRIVATE)
    private val configFile = File(context.filesDir, "config.toml")

    val modelsDir: String
        get() = File(context.filesDir, "models").apply { mkdirs() }.absolutePath

    /** Historique des dictées, sur le téléphone seulement. */
    val historyFile: String
        get() = File(context.filesDir, "history.jsonl").absolutePath

    /** Mode traduction du clavier et de la bulle (gardé d'une ouverture à l'autre). */
    var translate: Boolean
        get() = prefs.getBoolean("translate", false)
        set(value) = prefs.edit().putBoolean("translate", value).apply()

    /** Bulle : affichée seulement quand un clavier est ouvert. */
    var bubbleWithKeyboard: Boolean
        get() = prefs.getBoolean("bubble_with_keyboard", true)
        set(value) = prefs.edit().putBoolean("bubble_with_keyboard", value).apply()

    /** Bulle : 0 petite, 1 moyenne, 2 grande. */
    var bubbleSize: Int
        get() = prefs.getInt("bubble_size", 1).coerceIn(0, 2)
        set(value) = prefs.edit().putInt("bubble_size", value).apply()

    /** Bulle : insérer en collant (presse-papier) plutôt qu'en écrivant dans le champ. */
    var bubblePaste: Boolean
        get() = prefs.getBoolean("bubble_paste", false)
        set(value) = prefs.edit().putBoolean("bubble_paste", value).apply()

    /**
     * Bulle : dictées gardées dans l'historique (s'il est activé). La bulle
     * ne voit pas la navigation privée (le clavier, si) : réglage à part.
     */
    var bubbleHistory: Boolean
        get() = prefs.getBoolean("bubble_history", true)
        set(value) = prefs.edit().putBoolean("bubble_history", value).apply()

    /** Position de la bulle à l'écran (-1 : place par défaut). */
    var bubbleX: Int
        get() = prefs.getInt("bubble_x", -1)
        set(value) = prefs.edit().putInt("bubble_x", value).apply()

    var bubbleY: Int
        get() = prefs.getInt("bubble_y", -1)
        set(value) = prefs.edit().putInt("bubble_y", value).apply()

    /**
     * Bulle : heure (ms) de sa dernière connexion à Android, 0 si elle a été
     * désactivée proprement. Sert à retrouver l'arrêt qui l'a emportée.
     */
    var bubbleStartedAt: Long
        get() = prefs.getLong("bubble_started_at", 0)
        set(value) = prefs.edit().putLong("bubble_started_at", value).apply()

    /** Bulle : cause et date de son dernier arrêt par le téléphone (vide : jamais vu). */
    var bubbleStop: String
        get() = prefs.getString("bubble_stop", "").orEmpty()
        set(value) = prefs.edit().putString("bubble_stop", value).apply()

    /** Mises à jour : vérification automatique (au lancement puis toutes les 6 h). */
    var updatesEnabled: Boolean
        get() = prefs.getBoolean("updates_enabled", true)
        set(value) = prefs.edit().putBoolean("updates_enabled", value).apply()

    /** Heure de la dernière vérification (ms), réussie ou non. */
    var updateCheckedAt: Long
        get() = prefs.getLong("update_checked_at", 0)
        set(value) = prefs.edit().putLong("update_checked_at", value).apply()

    /** Version disponible trouvée à la dernière vérification (JSON, vide si à jour). */
    var updateRelease: String
        get() = prefs.getString("update_release", "").orEmpty()
        set(value) = prefs.edit().putString("update_release", value).apply()

    /** Dernière version annoncée par notification (une fois par version). */
    var updateNotified: String
        get() = prefs.getString("update_notified", "").orEmpty()
        set(value) = prefs.edit().putString("update_notified", value).apply()

    /** Échec de la dernière installation, affiché dans l'application. */
    var updateError: String
        get() = prefs.getString("update_error", "").orEmpty()
        set(value) = prefs.edit().putString("update_error", value).apply()

    /** Version au dernier lancement : « mis à jour en x.y.z » une fois. */
    var lastRunVersion: String
        get() = prefs.getString("last_run_version", "").orEmpty()
        set(value) = prefs.edit().putString("last_run_version", value).apply()

    /** Réunions : audio « conserve » (par défaut), « temporaire » ou « memoire ». */
    var meetingAudio: String
        get() = prefs.getString("meeting_audio", "conserve").orEmpty()
        set(value) = prefs.edit().putString("meeting_audio", value).apply()

    /** Réunions : compte rendu de cours ou conférence plutôt que de réunion. */
    var meetingConference: Boolean
        get() = prefs.getBoolean("meeting_conference", false)
        set(value) = prefs.edit().putBoolean("meeting_conference", value).apply()

    /** Réunions : compte rendu à la fin de la transcription. */
    var meetingSummary: Boolean
        get() = prefs.getBoolean("meeting_summary", true)
        set(value) = prefs.edit().putBoolean("meeting_summary", value).apply()

    /** Dossier des réunions (une par sous-dossier, comme sur PC). */
    val meetingsDir: String
        get() = File(context.filesDir, "meetings").apply { mkdirs() }.absolutePath

    /** Autorisation des notifications déjà demandée (Android 13 et plus). */
    var askedNotifications: Boolean
        get() = prefs.getBoolean("asked_notifications", false)
        set(value) = prefs.edit().putBoolean("asked_notifications", value).apply()

    var config: String
        get() = if (configFile.exists()) configFile.readText() else ""
        set(value) {
            configFile.writeText(value)
            // Le clavier recharge ses réglages au prochain affichage.
            prefs.edit().putLong("version", version + 1).apply()
        }

    /** Augmente à chaque changement de réglage ou de clé. */
    val version: Long
        get() = prefs.getLong("version", 0)

    fun hasKey(host: String): Boolean = prefs.contains("key:$host")

    fun key(host: String): String? {
        val stored = prefs.getString("key:$host", null) ?: return null
        return runCatching {
            val bytes = Base64.decode(stored, Base64.NO_WRAP)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, secret(), GCMParameterSpec(128, bytes, 0, IV_SIZE))
            String(cipher.doFinal(bytes, IV_SIZE, bytes.size - IV_SIZE), Charsets.UTF_8)
        }.getOrNull()
    }

    fun saveKey(host: String, key: String) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secret())
        val sealed = cipher.iv + cipher.doFinal(key.trim().toByteArray(Charsets.UTF_8))
        prefs.edit()
            .putString("key:$host", Base64.encodeToString(sealed, Base64.NO_WRAP))
            .putLong("version", version + 1)
            .apply()
    }

    fun deleteKey(host: String) {
        prefs.edit().remove("key:$host").putLong("version", version + 1).apply()
    }

    private fun secret(): SecretKey {
        val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (keyStore.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "voicetotext-api-keys"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_SIZE = 12
    }
}
