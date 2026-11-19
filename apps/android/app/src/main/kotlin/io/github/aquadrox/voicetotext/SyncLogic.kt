package io.github.aquadrox.voicetotext

import org.json.JSONObject
import java.text.DateFormat
import java.util.Date
import java.util.Locale

data class DiscoveredDevice(
    val deviceId: String,
    val deviceName: String,
    val addr: String,
)

data class PairedDevice(
    val deviceId: String,
    val deviceName: String,
    val fingerprint: String,
    val pairedAtMs: Long,
    val lastSyncMs: Long?,
    val lastAddr: String?,
    val syncSettings: Boolean,
    val syncMeetings: Boolean,
)

data class PendingDirectPairing(
    val sessionId: String,
    val remoteDeviceId: String,
    val remoteDeviceName: String,
    val code: String,
    val formattedCode: String,
    val expiresAtMs: Long,
    val incoming: Boolean,
    val localApproved: Boolean,
)

data class DirectStatusEvent(
    val seq: Long,
    val kind: String,
    val message: String,
    val configChanged: Boolean,
)

data class DirectPairingState(
    val listeningPort: Int?,
    val pending: PendingDirectPairing?,
    val lastEvent: DirectStatusEvent?,
)

/**
 * Logique pure de synchronisation Wi-Fi sur Android (sans dépendance SDK,
 * testée directement sur la JVM).
 */
object SyncLogic {
    /** Type de service DNS-SD annoncé par les appareils VoiceToText sur le Wi-Fi. */
    const val SERVICE_TYPE = "_voicetotext._tcp."

    private val URI_REGEX = Regex("vttsync://v1\\?[^\\s]+", RegexOption.IGNORE_CASE)

    /** Vrai si le texte est une URI d'appairage VoiceToText (`vttsync://v1?...`). */
    fun isPairingUri(raw: String): Boolean {
        val trimmed = raw.trim()
        return trimmed.startsWith("vttsync://v1?", ignoreCase = true) && trimmed.length > "vttsync://v1?".length
    }

    /** Extrait un lien `vttsync://v1?...` d'un texte ou presse-papier, ou null. */
    fun extractPairingUri(raw: String?): String? {
        if (raw.isNullOrBlank()) return null
        val match = URI_REGEX.find(raw.trim()) ?: return null
        return match.value.trimEnd('.', ',', ';', ')', ']')
    }

    /** Formate un code à 6 chiffres en deux groupes lisibles (`"482917"` → `"482 917"`). */
    fun formatPairingCode(code: String): String {
        val trimmed = code.trim()
        return if (trimmed.length == 6 && trimmed.all { it in '0'..'9' }) {
            "${trimmed.substring(0, 3)} ${trimmed.substring(3)}"
        } else {
            trimmed
        }
    }

    /** Filtre cet appareil lui-même de la liste mDNS/NSD lorsqu'il s'annonce sur le Wi-Fi. */
    fun filterSelf(devices: List<DiscoveredDevice>, selfDeviceId: String?): List<DiscoveredDevice> {
        val own = selfDeviceId?.trim()?.lowercase(Locale.ROOT).orEmpty()
        if (own.isEmpty()) return devices
        return devices.filterNot { it.deviceId.equals(own, ignoreCase = true) }
    }

    /**
     * Décode les attributs TXT d'un service mDNS/NSD (`id`, `name`) et construit
     * l'entrée découverte, ou null si le service n'est pas valide.
     */
    fun parseNsdAttributes(
        attributes: Map<String, ByteArray?>,
        host: String?,
        port: Int,
        fallbackName: String,
    ): DiscoveredDevice? {
        val cleanHost = host?.trim().orEmpty()
        if (cleanHost.isEmpty() || port !in 1..65535) return null
        val normalized = attributes.entries.associate { (k, v) ->
            k.lowercase(Locale.ROOT) to v?.toString(Charsets.UTF_8)?.trim().orEmpty()
        }
        val id = normalized["id"].orEmpty().lowercase(Locale.ROOT)
        if (id.length != 32 || !id.all { it in '0'..'9' || it in 'a'..'f' }) return null
        val name = normalized["name"]?.takeIf { it.isNotEmpty() }
            ?: fallbackName.trim().ifEmpty { "Appareil VoiceToText" }
        val addr = if (cleanHost.contains(':') && !cleanHost.startsWith('[')) {
            "[$cleanHost]:$port"
        } else {
            "$cleanHost:$port"
        }
        return DiscoveredDevice(deviceId = id, deviceName = name, addr = addr)
    }

    /** Lit la liste des pairs depuis la réponse JSON `{"ok": {"peers": [...]}}`. */
    fun parsePeers(viewOk: JSONObject): List<PairedDevice> {
        val arr = viewOk.optJSONArray("peers") ?: return emptyList()
        return List(arr.length()) { index ->
            val p = arr.getJSONObject(index)
            PairedDevice(
                deviceId = p.getString("device_id"),
                deviceName = p.optString("device_name", "Appareil"),
                fingerprint = p.optString("fingerprint", ""),
                pairedAtMs = p.optLong("paired_at_ms", 0L),
                lastSyncMs = if (p.has("last_sync_ms") && !p.isNull("last_sync_ms")) p.getLong("last_sync_ms") else null,
                lastAddr = p.optString("last_addr", "").takeIf { it.isNotEmpty() },
                syncSettings = p.optBoolean("sync_settings", true),
                syncMeetings = p.optBoolean("sync_meetings", true),
            )
        }
    }

    /** Lit un objet JSON de demande d'appairage direct en attente. */
    fun parsePendingDirect(obj: JSONObject?): PendingDirectPairing? {
        if (obj == null) return null
        val sessionId = obj.optString("session_id", "").trim()
        val code = obj.optString("code", "").trim()
        if (sessionId.isEmpty() || code.isEmpty()) return null
        val formatted = obj.optString("formatted_code", "").ifEmpty { formatPairingCode(code) }
        return PendingDirectPairing(
            sessionId = sessionId,
            remoteDeviceId = obj.optString("remote_device_id", ""),
            remoteDeviceName = obj.optString("remote_device_name", "Appareil"),
            code = code,
            formattedCode = formatted,
            expiresAtMs = obj.optLong("expires_at_ms", 0L),
            incoming = obj.optBoolean("incoming", false),
            localApproved = obj.optBoolean("local_approved", false),
        )
    }

    /** Lit la réponse JSON `ok` de `Native.syncDirectState()` ou `Native.syncRespondDirectPairing()`. */
    fun parseDirectState(ok: JSONObject): DirectPairingState {
        val port = if (ok.has("listening_port") && !ok.isNull("listening_port")) ok.optInt("listening_port") else null
        val pending = parsePendingDirect(ok.optJSONObject("pending"))
        val evObj = ok.optJSONObject("last_event")
        val lastEvent = if (evObj != null && evObj.optLong("seq", 0L) > 0L) {
            DirectStatusEvent(
                seq = evObj.getLong("seq"),
                kind = evObj.optString("kind", ""),
                message = evObj.optString("message", ""),
                configChanged = evObj.optBoolean("config_changed", false),
            )
        } else {
            null
        }
        return DirectPairingState(listeningPort = port, pending = pending, lastEvent = lastEvent)
    }

    /**
     * Choisit l'adresse à passer à `Native.syncNow` :
     * 1. saisie manuelle `IP:port` (secours) ;
     * 2. adresse en direct découverte par `NsdManager` ;
     * 3. chaîne vide → le moteur Rust utilise `peer.last_addr`.
     */
    fun resolveSyncAddress(
        peer: PairedDevice,
        discovered: List<DiscoveredDevice>,
        manualOverride: String,
    ): String {
        val manual = manualOverride.trim()
        if (manual.isNotEmpty()) return manual
        return discovered.firstOrNull { it.deviceId == peer.deviceId }?.addr.orEmpty()
    }

    /** Résumé affiché sur l'écran d'accueil sous « Synchronisation Wi-Fi ». */
    fun sectionSummary(peerCount: Int): String = when (peerCount) {
        0 -> "Aucun appareil appairé · PC, Steam Deck ou Android"
        1 -> "1 appareil appairé · Wi-Fi local chiffré"
        else -> "$peerCount appareils appairés · Wi-Fi local chiffré"
    }

    fun formatDate(ms: Long?): String {
        if (ms == null || ms <= 0L) return "jamais"
        return DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT, Locale.FRANCE)
            .format(Date(ms))
    }
}
