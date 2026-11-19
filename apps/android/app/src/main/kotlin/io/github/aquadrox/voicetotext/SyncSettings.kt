package io.github.aquadrox.voicetotext

import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Synchronisation locale Wi-Fi (pair-à-pair chiffrée, sans cloud) :
 * - appairage en scannant le QR code du PC (lien `vttsync://v1?...`) ou en le collant ;
 * - découverte automatique sur le Wi-Fi via Android [NsdDiscovery] ;
 * - synchronisation en un appui (dictionnaire, snippets, styles, prompts,
 *   profils LLM et réunions ; jamais les clés API) avec secours manuel `IP:port`.
 */
class SyncSettings : SettingsPage() {
    override val section = Section.SYNC
    override val subtitle = "Wi-Fi local chiffré · sans cloud ni compte"

    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var discovery: NsdDiscovery

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
    private var busy = false

    override fun LinearLayout.build() {
        note(
            "Synchronise dictionnaire, snippets, styles, prompts, profils LLM et réunions " +
                "avec votre PC ou Steam Deck sur le même Wi-Fi. Les clés API ne quittent jamais ce téléphone.",
        )

        card("Appairer un appareil", R.drawable.ic_wifi).anchor("sync_pair").apply {
            note(
                "Sur votre PC, ouvrez Réglages → Synchronisation → Afficher un QR code, " +
                    "puis scannez-le avec l'appareil photo du téléphone (il s'ouvrira ici) ou collez le lien ci-dessous.",
            )
            pairInput = field("vttsync://v1?id=...")
            row {
                secondary("Coller") { pastePairingUri() }
                pairButton = primary("Appairer") { pairFromInput() }
            }
        }

        card("Appareils appairés", R.drawable.ic_monitor).anchor("sync_peers").apply {
            peersEmpty = note("Aucun appareil appairé pour le moment.")
            peersContainer = LinearLayout(this@SyncSettings).apply { orientation = LinearLayout.VERTICAL }
            addView(peersContainer)
        }

        card("Détectés sur le Wi-Fi", R.drawable.ic_refresh_cw).anchor("sync_nsd").apply {
            discoveryStatus = note("Recherche des appareils VoiceToText sur le réseau local…")
            discoveredEmpty = note("Aucun appareil détecté pour l'instant.")
            discoveredContainer = LinearLayout(this@SyncSettings).apply { orientation = LinearLayout.VERTICAL }
            addView(discoveredContainer)
            secondary("Actualiser la découverte") {
                discoveryStatus.text = "Recherche en cours…"
                discovery.restart()
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
        discovery.start()
        loadView()
    }

    override fun onPause() {
        discovery.stop()
        super.onPause()
    }

    override fun onDestroy() {
        discovery.destroy()
        worker.shutdownNow()
        super.onDestroy()
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
            .setMessage("Sa clé de synchronisation sera révoquée sur ce téléphone. Vous pourrez le réappairer plus tard avec un nouveau QR code.")
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
            if (index > 0) discoveredContainer.divider(6)
            val state = if (device.deviceId in pairedIds) "Déjà appairé" else "Non appairé — scannez son QR code"
            discoveredContainer.text(device.deviceName, 15f, bold = true)
            discoveredContainer.note("${device.addr} · $state")
        }
    }

    companion object {
        fun defaultPhoneName(): String {
            val model = Build.MODEL?.trim().orEmpty()
            return if (model.isNotEmpty()) model else "Android"
        }
    }
}
