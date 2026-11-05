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
     * Bulle : dictées gardées dans l'historique. Coupé par défaut, car la
     * bulle ne voit pas la navigation privée (le clavier, si).
     */
    var bubbleHistory: Boolean
        get() = prefs.getBoolean("bubble_history", false)
        set(value) = prefs.edit().putBoolean("bubble_history", value).apply()

    /** Position de la bulle à l'écran (-1 : place par défaut). */
    var bubbleX: Int
        get() = prefs.getInt("bubble_x", -1)
        set(value) = prefs.edit().putInt("bubble_x", value).apply()

    var bubbleY: Int
        get() = prefs.getInt("bubble_y", -1)
        set(value) = prefs.edit().putInt("bubble_y", value).apply()

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
