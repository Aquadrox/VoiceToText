package io.github.aquadrox.voicetotext

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.AudioRecord
import android.os.Build
import android.util.Log

/**
 * Micros du téléphone et micros branchés (prise jack, USB, Bluetooth) : la
 * liste de Réglages → Dictée, et le micro imposé aux enregistrements
 * (dictée du clavier et de la bulle, réunions, test).
 *
 * Le choix est gardé sous forme de clé (type, nom, adresse) : le numéro
 * qu'Android donne à un micro change à chaque branchement. Micro choisi
 * absent (débranché) : Android choisit, comme en « Automatique ».
 */
object Microphones {
    /** Micro proposé : clé enregistrée, nom affiché, appareil d'Android. */
    class Mic(val key: String, val label: String, val device: AudioDeviceInfo)

    /** Micros utilisables en ce moment, ceux du téléphone d'abord. */
    fun list(context: Context): List<Mic> {
        val devices = context.getSystemService(AudioManager::class.java)
            .getDevices(AudioManager.GET_DEVICES_INPUTS)
            .filter { rank(it.type) != null }
            .sortedBy { rank(it.type) }
            .distinctBy { key(it) }
        // Plusieurs micros dans le téléphone (bas, arrière…) : on les distingue.
        val builtins = devices.filter { it.type == AudioDeviceInfo.TYPE_BUILTIN_MIC }
        return devices.map { device ->
            val position = if (builtins.size > 1) builtins.indexOf(device) else -1
            Mic(key(device), label(device, position), device)
        }
    }

    /**
     * Le micro de cette clé parmi `mics`, ou, s'il a changé d'adresse (micro
     * USB rebranché sur un autre port), un micro du même type et du même nom.
     */
    fun match(mics: List<Mic>, key: String): Mic? {
        if (key.isEmpty()) return null
        mics.firstOrNull { it.key == key }?.let { return it }
        val parts = key.split(SEPARATOR)
        val type = parts.getOrNull(0)?.toIntOrNull() ?: return null
        // Les micros du téléphone ne se distinguent que par leur adresse.
        if (type == AudioDeviceInfo.TYPE_BUILTIN_MIC) return null
        return mics.firstOrNull { it.device.type == type && name(it.device) == parts.getOrNull(1) }
    }

    /** Nom d'un micro pour l'affichage (celui qui enregistre réellement, par exemple). */
    fun describe(context: Context, device: AudioDeviceInfo): String =
        list(context).firstOrNull { it.key == key(device) }?.label ?: label(device, -1)

    /**
     * Impose le micro choisi dans les réglages à un enregistrement, avant son
     * démarrage. Renvoie de quoi refermer la liaison Bluetooth ouverte pour
     * lui, à appeler à la fin (null : rien à refermer).
     */
    fun use(context: Context, store: Store, record: AudioRecord): (() -> Unit)? {
        val chosen = match(list(context), store.microphone)?.device ?: return null
        if (!bluetooth(chosen.type)) {
            record.setPreferredDevice(chosen)
            return null
        }
        // Liaison refusée : Android choisit, plutôt qu'un micro muet.
        val close = openBluetooth(context, chosen) ?: return null
        record.setPreferredDevice(chosen)
        return close
    }

    /** Micro qui capte réellement (connu une fois l'enregistrement lancé), ou null. */
    fun routed(context: Context, record: AudioRecord): String? =
        record.routedDevice?.let { describe(context, it) }

    /**
     * Un casque Bluetooth n'envoie sa voix qu'en mode appel (liaison « SCO ») :
     * elle est ouverte le temps de l'enregistrement. Elle met environ une
     * seconde à s'établir, et le son du casque baisse en qualité pendant ce
     * temps. Renvoie de quoi la refermer, ou null si elle est refusée.
     */
    private fun openBluetooth(context: Context, device: AudioDeviceInfo): (() -> Unit)? {
        val manager = context.getSystemService(AudioManager::class.java)
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                val target = manager.availableCommunicationDevices.firstOrNull {
                    it.type == device.type && (it.address == device.address || name(it) == name(device))
                }
                if (target == null || !manager.setCommunicationDevice(target)) return null
                val close: () -> Unit = { runCatching { manager.clearCommunicationDevice() } }
                close
            } else {
                @Suppress("DEPRECATION")
                manager.startBluetoothSco()
                @Suppress("DEPRECATION")
                manager.isBluetoothScoOn = true
                val close: () -> Unit = {
                    runCatching {
                        @Suppress("DEPRECATION")
                        manager.isBluetoothScoOn = false
                        @Suppress("DEPRECATION")
                        manager.stopBluetoothSco()
                    }
                }
                close
            }
        } catch (e: Exception) {
            Log.w("VoiceToText", "Liaison Bluetooth du micro refusée", e)
            null
        }
    }

    internal fun bluetooth(type: Int) =
        type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO || type == AudioDeviceInfo.TYPE_BLE_HEADSET

    private fun key(device: AudioDeviceInfo): String =
        listOf(device.type.toString(), name(device), device.address.orEmpty()).joinToString(SEPARATOR)

    private fun name(device: AudioDeviceInfo): String = device.productName?.toString()?.trim().orEmpty()

    /** `position` : rang parmi les micros du téléphone, -1 s'il n'y en a qu'un. */
    private fun label(device: AudioDeviceInfo, position: Int): String {
        val name = name(device)
        return when (device.type) {
            AudioDeviceInfo.TYPE_BUILTIN_MIC ->
                if (position < 0) "Micro du téléphone" else "Micro du téléphone (${place(device.address.orEmpty(), position)})"
            AudioDeviceInfo.TYPE_WIRED_HEADSET -> "Casque ou micro filaire (prise jack)"
            AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_ACCESSORY ->
                named("Micro USB", name)
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO, AudioDeviceInfo.TYPE_BLE_HEADSET -> named("Bluetooth", name)
            AudioDeviceInfo.TYPE_DOCK -> "Station d'accueil"
            else -> "Entrée ligne"
        }
    }

    internal fun named(kind: String, name: String) = if (name.isEmpty()) kind else "$kind : $name"

    /** Place d'un micro du téléphone, d'après l'adresse que lui donne le fabricant. */
    internal fun place(address: String, position: Int) = when (address.lowercase()) {
        "bottom" -> "bas"
        "back" -> "arrière"
        "top" -> "haut"
        "front" -> "avant"
        else -> "n° ${position + 1}"
    }

    /** Ordre d'affichage ; null : pas un micro (téléphonie, tuner radio, flux internes). */
    internal fun rank(type: Int): Int? = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_MIC -> 0
        AudioDeviceInfo.TYPE_WIRED_HEADSET -> 1
        AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_ACCESSORY -> 2
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO, AudioDeviceInfo.TYPE_BLE_HEADSET -> 3
        AudioDeviceInfo.TYPE_LINE_ANALOG, AudioDeviceInfo.TYPE_LINE_DIGITAL, AudioDeviceInfo.TYPE_DOCK -> 4
        else -> null
    }

    /** Sépare les parties de la clé (caractère absent des noms d'appareils). */
    private const val SEPARATOR = "\u001F"
}
