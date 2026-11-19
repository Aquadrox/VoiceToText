package io.github.aquadrox.voicetotext

import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Synchronisation locale Wi-Fi (pair-à-pair chiffrée, sans cloud) :
 * - appairage direct par clic sur un appareil détecté + code à 6 chiffres
 *   (dans les deux sens PC ↔ téléphone, sans scanner de QR code) ;
 * - appairage par QR code du PC (`vttsync://v1?...`) ou collage du lien ;
 * - découverte et annonce temporaire sur le Wi-Fi via Android [NsdDiscovery] ;
 * - synchronisation en un appui (dictionnaire, snippets, styles, prompts,
 *   profils LLM et réunions ; jamais les clés API) avec secours manuel `IP:port`.
 */
class SyncSettings : SettingsPage() {
    override val section = Section.SYNC
    override val subtitle = "Wi-Fi local chiffré · sans cloud ni compte"

    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var discovery: NsdDiscovery

    private lateinit var directCard: LinearLayout
    private lateinit var directTitle: TextView
    private lateinit var directCode: TextView
    private lateinit var directExpiry: TextView
    private lateinit var directApprove: Button
    private lateinit var directReject: Button

    private lateinit var pairInput: EditText
    private lateinit var pairButton: Button
    private lateinit var peersContainer: LinearLayout
    private lateinit var peersEmpty: TextView
    private lateinit var discoveredContainer: LinearLayout
    private lateinit var discoveredEmpty: TextView
    private lateinit var discoveryStatus: TextView
    private lateinit var deviceNameInput: EditText

    private var peers: List<PairedDevice> = emptyList()
    private var discovered: List<DiscoveredDevice> = emptyList()
    private val manualOverrides = mutableMapOf<String, String>()
    private var localDeviceName: String = defaultPhoneName()
    private var localDeviceId: String = ""
    private var listeningPort: Int? = null
    private var pendingDirect: PendingDirectPairing? = null
    private var lastShownSessionId: String? = null
    private var lastEventSeq: Long = 0L
    private var busy = false
    private var active = false

    private val directPoller = object : Runnable {
        override fun run() {
            if (!active || isDestroyed) return
            pollDirectState()
            main.postDelayed(this, POLL_DIRECT_MS)
        }
    }

    override fun LinearLayout.build() {
        note(
            "Synchronise dictionnaire, snippets, styles, prompts, profils LLM et réunions " +
                "avec votre PC ou Steam Deck sur le même Wi-Fi. Les clés API ne quittent jamais ce téléphone.",
        )

        directCard = card("", 0).apply {
            background = rounded(getColor(R.color.card), 14f, getColor(R.color.signal), 2)
            visibility = View.GONE
            directTitle = text("", 16f, bold = true)
            note("Vérifiez que ce code à 6 chiffres est identique sur l'autre appareil, puis appuyez sur Appairer (OK) des deux côtés.")
            directCode = text("", 28f, bold = true).apply {
                typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
                gravity = Gravity.CENTER_HORIZONTAL
                letterSpacing = 0.16f
                background = rounded(getColor(R.color.bg), 10f, getColor(R.color.border_strong))
                setPadding(dp(12), dp(12), dp(12), dp(12))
                (layoutParams as LinearLayout.LayoutParams).apply {
                    topMargin = dp(8)
                    bottomMargin = dp(6)
                }
            }
            directExpiry = note("")
            row {
                directApprove = primary("Appairer (OK)") { respondDirect(true) }
                directReject = secondary("Refuser") { respondDirect(false) }
            }
        }

        card("Détectés sur le Wi-Fi", R.drawable.ic_refresh_cw).anchor("sync_nsd").apply {
            discoveryStatus = note("Recherche des appareils VoiceToText sur le réseau local…")
            discoveredEmpty = note("Aucun autre appareil détecté pour l'instant. Ouvrez Réglages → Synchronisation sur votre PC.")
            discoveredContainer = LinearLayout(this@SyncSettings).apply { orientation = LinearLayout.VERTICAL }
            addView(discoveredContainer)
            secondary("Actualiser la découverte") {
                discoveryStatus.text = "Recherche en cours…"
                discovery.restart()
                val port = listeningPort
                if (port != null && localDeviceId.isNotEmpty()) {
                    discovery.advertise(localDeviceId, localDeviceName, port)
                }
            }
        }

        card("Appareils appairés", R.drawable.ic_monitor).anchor("sync_peers").apply {
            peersEmpty = note("Aucun appareil appairé pour le moment.")
            peersContainer = LinearLayout(this@SyncSettings).apply { orientation = LinearLayout.VERTICAL }
            addView(peersContainer)
        }

        card("Appairer par QR code ou lien", R.drawable.ic_wifi).anchor("sync_pair").apply {
            note(
                "En alternative à l'appairage direct ci-dessus : sur votre PC, ouvrez Réglages → Synchronisation → Afficher un QR code, " +
                    "puis scannez-le avec l'appareil photo du téléphone ou collez le lien ci-dessous.",
            )
            pairInput = field("vttsync://v1?id=...")
            row {
                secondary("Coller") { pastePairingUri() }
                pairButton = primary("Appairer") { pairFromInput() }
            }
        }

        card("Ce téléphone", R.drawable.ic_list_checks).anchor("sync_device").apply {
            note("Nom affiché sur le PC lors de l'appairage et de la synchronisation.")
            row {
                deviceNameInput = field("Nom de cet appareil")
                secondary("Renommer") { renameLocalDevice() }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        discovery = NsdDiscovery(
            context = this,
            onChanged = { list ->
                discovered = list
                discoveryStatus.text = if (list.isEmpty()) {
                    "Découverte Wi-Fi active (_voicetotext._tcp.)."
                } else {
                    "${list.size} appareil${if (list.size > 1) "s" else ""} détecté${if (list.size > 1) "s" else ""} sur le Wi-Fi."
                }
                renderDiscovered()
                renderPeers()
            },
            onError = { err ->
                discoveryStatus.text = err
            },
        )
        loadView()
        handlePairingIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairingIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        active = true
        discovery.start()
        startDirectListener()
        loadView()
        main.removeCallbacks(directPoller)
        main.postDelayed(directPoller, POLL_DIRECT_MS)
    }

    override fun onPause() {
        active = false
        main.removeCallbacks(directPoller)
        discovery.stop()
        listeningPort = null
        worker.execute { runCatching { Native.syncStopListener() } }
        super.onPause()
    }

    override fun onDestroy() {
        active = false
        main.removeCallbacks(directPoller)
        discovery.destroy()
        runCatching { Native.syncStopListener() }
        worker.shutdownNow()
        super.onDestroy()
    }

    /** Démarre le serveur TCP temporaire Android et annonce ce téléphone via NSD. */
    private fun startDirectListener() {
        worker.execute {
            val reply = JSONObject(
                Native.syncStartListener(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                ),
            )
            main.post {
                if (!active || isDestroyed) return@post
                val ok = reply.optJSONObject("ok") ?: return@post
                val port = ok.optInt("port", 0)
                val id = ok.optString("device_id", "")
                val name = ok.optString("device_name", defaultPhoneName())
                if (port > 0 && id.isNotEmpty()) {
                    listeningPort = port
                    localDeviceId = id
                    localDeviceName = name
                    discovery.advertise(id, name, port)
                }
            }
        }
    }

    private fun pollDirectState() {
        val raw = runCatching { Native.syncDirectState() }.getOrNull() ?: return
        val ok = runCatching { JSONObject(raw).optJSONObject("ok") }.getOrNull() ?: return
        applyDirectState(SyncLogic.parseDirectState(ok))
    }

    private fun applyDirectState(state: DirectPairingState) {
        val prevPending = pendingDirect
        pendingDirect = state.pending
        renderPendingDirect(state.pending)
        if ((prevPending == null) != (state.pending == null)) {
            renderDiscovered()
        }

        val ev = state.lastEvent
        if (ev != null && ev.seq > lastEventSeq) {
            lastEventSeq = ev.seq
            when (ev.kind) {
                "paired" -> {
                    saved("✓ ${ev.message}")
                    loadView()
                }
                "synced" -> {
                    if (ev.configChanged) store.notifyConfigChanged()
                    saved("✓ ${ev.message}")
                    loadView()
                }
                "ended" -> {
                    if (ev.message.isNotEmpty()) say(ev.message)
                }
            }
        }
    }

    private fun renderPendingDirect(pending: PendingDirectPairing?) {
        if (pending == null) {
            lastShownSessionId = null
            directCard.visibility = View.GONE
            return
        }
        directTitle.text = if (pending.incoming) {
            "Demande d'appairage reçue de « ${pending.remoteDeviceName} »"
        } else {
            "Appairage direct avec « ${pending.remoteDeviceName} »"
        }
        directCode.text = pending.formattedCode
        directApprove.isEnabled = !pending.localApproved
        directApprove.text = if (pending.localApproved) "En attente de l'autre appareil…" else "Appairer (OK)"
        directReject.isEnabled = true
        directReject.text = if (pending.incoming && !pending.localApproved) "Refuser" else "Annuler"

        val remSec = ((pending.expiresAtMs - System.currentTimeMillis()) / 1000L).coerceAtLeast(0L)
        directExpiry.text = if (remSec > 0L) {
            "Expire dans 0:${remSec.toString().padStart(2, '0')}."
        } else {
            "Demande expirée."
        }

        directCard.visibility = View.VISIBLE
        if (lastShownSessionId != pending.sessionId) {
            lastShownSessionId = pending.sessionId
            directCard.reveal()
        }
    }

    private fun startDirectPairing(device: DiscoveredDevice, button: Button) {
        if (busy || pendingDirect != null) return
        busy = true
        button.isEnabled = false
        button.text = "Connexion…"
        say("Demande d'appairage envoyée à ${device.deviceName}…")
        worker.execute {
            val reply = JSONObject(
                Native.syncStartDirectPairing(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                    device.deviceId,
                    device.addr,
                ),
            )
            main.post {
                busy = false
                if (reply.has("error")) {
                    button.isEnabled = true
                    button.text = "Appairer"
                    say("Appairage direct impossible : ${reply.getString("error")}")
                } else {
                    val pending = SyncLogic.parsePendingDirect(reply.optJSONObject("ok"))
                    pendingDirect = pending
                    renderPendingDirect(pending)
                    renderDiscovered()
                    if (pending != null) {
                        say("Comparez le code ${pending.formattedCode} sur les deux écrans puis appuyez sur Appairer (OK).")
                    }
                }
            }
        }
    }

    private fun respondDirect(approve: Boolean) {
        val current = pendingDirect ?: return
        directApprove.isEnabled = false
        directReject.isEnabled = false
        if (approve) directApprove.text = "En attente de l'autre appareil…"
        worker.execute {
            val reply = JSONObject(Native.syncRespondDirectPairing(current.sessionId, approve))
            main.post {
                val ok = reply.optJSONObject("ok")
                if (ok == null) {
                    say(reply.optString("error", "Impossible de répondre à l'appairage."))
                    pollDirectState()
                } else {
                    applyDirectState(SyncLogic.parseDirectState(ok))
                    if (!approve) say("Appairage direct annulé.")
                }
            }
        }
    }

    /** Lien `vttsync://v1?...` ouvert depuis l'appareil photo après scan du QR code du PC. */
    private fun handlePairingIntent(intent: Intent?) {
        val raw = intent?.dataString ?: return
        val uri = SyncLogic.extractPairingUri(raw) ?: return
        intent.data = null
        pairInput.setText(uri)
        AlertDialog.Builder(this)
            .setTitle("Appairer cet appareil ?")
            .setMessage("Un QR code d'appairage VoiceToText a été scanné. Voulez-vous appairer cet appareil sur le réseau local ?")
            .setPositiveButton("Appairer") { _, _ -> pairUri(uri) }
            .setNegativeButton("Annuler", null)
            .show()
    }

    private fun pastePairingUri() {
        val clipboard = getSystemService(ClipboardManager::class.java)
        val clip = clipboard?.primaryClip?.getItemAt(0)?.coerceToText(this)?.toString()
        val extracted = SyncLogic.extractPairingUri(clip)
        if (extracted == null) {
            say("Aucun lien vttsync://v1?... trouvé dans le presse-papier.")
            return
        }
        pairInput.setText(extracted)
        say("")
    }

    private fun pairFromInput() {
        val extracted = SyncLogic.extractPairingUri(pairInput.text.toString())
        if (extracted == null) {
            say("Collez un lien d'appairage valide (vttsync://v1?...).")
            return
        }
        pairUri(extracted)
    }

    private fun pairUri(uri: String) {
        if (busy) return
        busy = true
        pairButton.isEnabled = false
        say("Appairage chiffré en cours…")
        worker.execute {
            val reply = JSONObject(
                Native.syncPairUri(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                    uri,
                ),
            )
            main.post {
                busy = false
                pairButton.isEnabled = true
                if (reply.has("error")) {
                    say("Appairage échoué : ${reply.getString("error")}")
                } else {
                    val peer = reply.getJSONObject("ok")
                    pairInput.setText("")
                    saved("✓ Appareil « ${peer.optString("device_name")} » appairé")
                    loadView()
                }
            }
        }
    }

    private fun loadView() {
        worker.execute {
            val reply = JSONObject(
                Native.syncView(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                ),
            )
            main.post {
                val ok = reply.optJSONObject("ok")
                if (ok == null) {
                    say(reply.optString("error", "Impossible de lire les appareils appairés."))
                    return@post
                }
                localDeviceId = ok.optString("device_id", localDeviceId)
                localDeviceName = ok.optString("device_name", defaultPhoneName())
                if (!deviceNameInput.hasFocus()) {
                    deviceNameInput.setText(localDeviceName)
                }
                peers = SyncLogic.parsePeers(ok)
                renderPeers()
                renderDiscovered()
            }
        }
    }

    private fun renameLocalDevice() {
        val name = deviceNameInput.text.toString().trim()
        if (name.isEmpty()) return
        worker.execute {
            val reply = JSONObject(
                Native.syncRenameDevice(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                    name,
                ),
            )
            main.post {
                val ok = reply.optJSONObject("ok")
                if (ok == null) {
                    say(reply.optString("error", "Renommage impossible."))
                } else {
                    localDeviceName = ok.optString("device_name", name)
                    deviceNameInput.setText(localDeviceName)
                    val port = listeningPort
                    if (active && port != null && localDeviceId.isNotEmpty()) {
                        discovery.advertise(localDeviceId, localDeviceName, port)
                    }
                    saved("✓ Nom enregistré : $localDeviceName")
                }
            }
        }
    }

    private fun syncPeer(peer: PairedDevice, button: Button) {
        if (busy) return
        val addrOverride = SyncLogic.resolveSyncAddress(
            peer = peer,
            discovered = discovered,
            manualOverride = manualOverrides[peer.deviceId].orEmpty(),
        )
        busy = true
        button.isEnabled = false
        button.text = "Synchronisation…"
        say("Synchronisation avec ${peer.deviceName}…")
        worker.execute {
            val reply = JSONObject(
                Native.syncNow(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                    peer.deviceId,
                    addrOverride,
                ),
            )
            main.post {
                busy = false
                button.isEnabled = true
                button.text = "Synchroniser"
                val ok = reply.optJSONObject("ok")
                if (ok == null) {
                    say("Synchronisation échouée : ${reply.optString("error")}")
                } else {
                    if (ok.optBoolean("config_changed", false)) {
                        store.notifyConfigChanged()
                    }
                    val msg = ok.optString("message", "Synchronisation terminée.")
                    say("✓ $msg")
                    saved("✓ $msg")
                    loadView()
                }
            }
        }
    }

    private fun updatePeerPrefs(peer: PairedDevice, syncSettings: Boolean, syncMeetings: Boolean) {
        worker.execute {
            val reply = JSONObject(
                Native.syncSetPeerPrefs(
                    store.configPath,
                    store.syncPeersFile,
                    store.meetingsDir,
                    defaultPhoneName(),
                    peer.deviceId,
                    syncSettings,
                    syncMeetings,
                ),
            )
            main.post {
                if (reply.has("error")) {
                    say(reply.getString("error"))
                    loadView()
                } else {
                    saved("✓ Préférences enregistrées")
                }
            }
        }
    }

    private fun confirmRemovePeer(peer: PairedDevice) {
        AlertDialog.Builder(this)
            .setTitle("Oublier « ${peer.deviceName} » ?")
            .setMessage("Sa clé de synchronisation sera révoquée sur ce téléphone. Vous pourrez le réappairer plus tard directement ou avec un QR code.")
            .setPositiveButton("Oublier") { _, _ ->
                worker.execute {
                    val reply = JSONObject(
                        Native.syncRemovePeer(
                            store.configPath,
                            store.syncPeersFile,
                            store.meetingsDir,
                            defaultPhoneName(),
                            peer.deviceId,
                        ),
                    )
                    main.post {
                        if (reply.has("error")) {
                            say(reply.getString("error"))
                        } else {
                            manualOverrides.remove(peer.deviceId)
                            saved("Appareil « ${peer.deviceName} » oublié")
                            loadView()
                        }
                    }
                }
            }
            .setNegativeButton("Annuler", null)
            .show()
    }

    private fun renderPeers() {
        peersContainer.removeAllViews()
        peersEmpty.visibility = if (peers.isEmpty()) View.VISIBLE else View.GONE
        val onlineById = discovered.associateBy { it.deviceId }

        peers.forEachIndexed { index, peer ->
            if (index > 0) peersContainer.divider(10)
            val online = onlineById[peer.deviceId]
            val statusLine = if (online != null) {
                "En ligne · ${online.addr}"
            } else if (!peer.lastAddr.isNullOrEmpty()) {
                "Hors ligne · dernière adresse ${peer.lastAddr}"
            } else {
                "Adresse inconnue (saisissez IP:port ci-dessous)"
            }
            peersContainer.text(peer.deviceName, 16f, bold = true)
            peersContainer.note("$statusLine\nEmpreinte : ${peer.fingerprint} · Dernière synchro : ${SyncLogic.formatDate(peer.lastSyncMs)}")

            var settingsEnabled = peer.syncSettings
            var meetingsEnabled = peer.syncMeetings
            peersContainer.option("Synchroniser les réglages", settingsEnabled) { checked ->
                settingsEnabled = checked
                updatePeerPrefs(peer, settingsEnabled, meetingsEnabled)
            }
            peersContainer.option("Synchroniser les réunions", meetingsEnabled) { checked ->
                meetingsEnabled = checked
                updatePeerPrefs(peer, settingsEnabled, meetingsEnabled)
            }

            val overrideField = peersContainer.field(
                "Secours IP:port (ex. ${online?.addr ?: peer.lastAddr ?: "192.168.1.10:41739"})",
            ).apply {
                setText(manualOverrides[peer.deviceId].orEmpty())
                setOnFocusChangeListener { _, _ ->
                    manualOverrides[peer.deviceId] = text.toString().trim()
                }
            }

            peersContainer.row {
                lateinit var syncBtn: Button
                syncBtn = primary("Synchroniser") {
                    manualOverrides[peer.deviceId] = overrideField.text.toString().trim()
                    syncPeer(peer, syncBtn)
                }
                secondary("Oublier") { confirmRemovePeer(peer) }
            }
        }
    }

    private fun renderDiscovered() {
        discoveredContainer.removeAllViews()
        discoveredEmpty.visibility = if (discovered.isEmpty()) View.VISIBLE else View.GONE
        val pairedIds = peers.map { it.deviceId }.toSet()
        discovered.forEachIndexed { index, device ->
            if (index > 0) discoveredContainer.divider(8)
            val alreadyPaired = device.deviceId in pairedIds
            val state = if (alreadyPaired) "Déjà appairé" else "Non appairé"
            discoveredContainer.text(device.deviceName, 15f, bold = true)
            discoveredContainer.note("${device.addr} · $state")
            if (!alreadyPaired) {
                lateinit var pairDirectBtn: Button
                pairDirectBtn = discoveredContainer.primary("Appairer") {
                    startDirectPairing(device, pairDirectBtn)
                }.apply {
                    isEnabled = pendingDirect == null && !busy
                }
            }
        }
    }

    companion object {
        private const val POLL_DIRECT_MS = 450L

        fun defaultPhoneName(): String {
            val model = Build.MODEL?.trim().orEmpty()
            return if (model.isNotEmpty()) model else "Android"
        }
    }
}
