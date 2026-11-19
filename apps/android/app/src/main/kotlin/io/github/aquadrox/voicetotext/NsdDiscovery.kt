package io.github.aquadrox.voicetotext

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.net.Inet4Address
import java.util.ArrayDeque
import java.util.concurrent.Executors

/**
 * Découverte Wi-Fi locale via l'API native Android [NsdManager] sur le service
 * `_voicetotext._tcp.`. Active uniquement pendant que l'écran Synchronisation
 * est affiché (aucune consommation en arrière-plan).
 */
class NsdDiscovery(
    context: Context,
    private val onChanged: (List<DiscoveredDevice>) -> Unit,
    private val onError: (String) -> Unit,
) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as? NsdManager
    private val main = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()

    private val discoveredByService = LinkedHashMap<String, DiscoveredDevice>()
    private val pendingResolve = ArrayDeque<NsdServiceInfo>()
    private val infoCallbacks = LinkedHashMap<String, Any>()
    private var resolving = false
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var registrationListener: NsdManager.RegistrationListener? = null
    @Volatile
    private var selfDeviceId: String? = null

    /**
     * Annonce temporairement cet appareil sur `_voicetotext._tcp.` pour qu'un PC
     * puisse le détecter et lancer un appairage direct par clic + code à 6 chiffres.
     */
    fun advertise(deviceId: String, deviceName: String, port: Int) {
        val manager = nsd ?: return
        if (port !in 1..65535 || deviceId.isBlank()) return
        selfDeviceId = deviceId
        stopAdvertising()

        val info = NsdServiceInfo().apply {
            serviceName = "vtt-${deviceId.take(8)}"
            serviceType = SyncLogic.SERVICE_TYPE
            setPort(port)
            setAttribute("id", deviceId)
            setAttribute("name", deviceName.trim().ifEmpty { "Android" })
            setAttribute("v", "1")
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(serviceInfo: NsdServiceInfo) {}
            override fun onRegistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                registrationListener = null
            }

            override fun onServiceUnregistered(serviceInfo: NsdServiceInfo) {}
            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                registrationListener = null
            }
        }
        registrationListener = listener
        runCatching {
            manager.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
        }.onFailure {
            registrationListener = null
        }
        synchronized(discoveredByService) {
            publishLocked()
        }
    }

    fun stopAdvertising() {
        val manager = nsd ?: return
        val listener = registrationListener ?: return
        registrationListener = null
        runCatching { manager.unregisterService(listener) }
    }

    fun start() {
        val manager = nsd ?: run {
            main.post { onError("Découverte réseau indisponible sur cet appareil.") }
            return
        }
        if (discoveryListener != null) return

        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}

            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                if (!serviceInfo.serviceType.contains("_voicetotext")) return
                resolve(manager, serviceInfo)
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo) {
                synchronized(discoveredByService) {
                    discoveredByService.remove(serviceInfo.serviceName)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                        (infoCallbacks.remove(serviceInfo.serviceName) as? NsdManager.ServiceInfoCallback)?.let { cb ->
                            runCatching { manager.unregisterServiceInfoCallback(cb) }
                        }
                    }
                    publishLocked()
                }
            }

            override fun onDiscoveryStopped(serviceType: String) {}

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                discoveryListener = null
                main.post { onError("Découverte Wi-Fi indisponible (code $errorCode). Utilisez le QR code ou IP:port.") }
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                discoveryListener = null
            }
        }
        discoveryListener = listener
        runCatching {
            manager.discoverServices(SyncLogic.SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        }.onFailure { err ->
            discoveryListener = null
            main.post { onError(err.message ?: "Découverte Wi-Fi indisponible.") }
        }
    }

    fun restart() {
        stop()
        synchronized(discoveredByService) {
            discoveredByService.clear()
            publishLocked()
        }
        start()
    }

    fun stop() {
        stopAdvertising()
        val manager = nsd ?: return
        discoveryListener?.let { listener ->
            discoveryListener = null
            runCatching { manager.stopServiceDiscovery(listener) }
        }
        synchronized(discoveredByService) {
            pendingResolve.clear()
            resolving = false
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                for (cb in infoCallbacks.values) {
                    (cb as? NsdManager.ServiceInfoCallback)?.let {
                        runCatching { manager.unregisterServiceInfoCallback(it) }
                    }
                }
                infoCallbacks.clear()
            }
        }
    }

    fun destroy() {
        stop()
        executor.shutdownNow()
    }

    private fun resolve(manager: NsdManager, info: NsdServiceInfo) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val cb = object : NsdManager.ServiceInfoCallback {
                override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {}
                override fun onServiceUpdated(updated: NsdServiceInfo) {
                    val host = updated.hostAddresses
                        .firstOrNull { it is Inet4Address }?.hostAddress
                        ?: updated.hostAddresses.firstOrNull()?.hostAddress
                    recordResolved(updated.serviceName, updated.attributes, host, updated.port)
                }

                override fun onServiceLost() {
                    synchronized(discoveredByService) {
                        discoveredByService.remove(info.serviceName)
                        publishLocked()
                    }
                }

                override fun onServiceInfoCallbackUnregistered() {}
            }
            synchronized(discoveredByService) {
                (infoCallbacks.remove(info.serviceName) as? NsdManager.ServiceInfoCallback)?.let { old ->
                    runCatching { manager.unregisterServiceInfoCallback(old) }
                }
                infoCallbacks[info.serviceName] = cb
            }
            runCatching { manager.registerServiceInfoCallback(info, executor, cb) }
        } else {
            synchronized(discoveredByService) {
                pendingResolve.addLast(info)
                drainResolveQueueLocked(manager)
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun drainResolveQueueLocked(manager: NsdManager) {
        if (resolving) return
        val next = pendingResolve.pollFirst() ?: return
        resolving = true
        runCatching {
            manager.resolveService(
                next,
                object : NsdManager.ResolveListener {
                    override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                        synchronized(discoveredByService) {
                            resolving = false
                            drainResolveQueueLocked(manager)
                        }
                    }

                    override fun onServiceResolved(resolved: NsdServiceInfo) {
                        recordResolved(
                            resolved.serviceName,
                            resolved.attributes,
                            resolved.host?.hostAddress,
                            resolved.port,
                        )
                        synchronized(discoveredByService) {
                            resolving = false
                            drainResolveQueueLocked(manager)
                        }
                    }
                },
            )
        }.onFailure {
            resolving = false
        }
    }

    private fun recordResolved(
        serviceName: String,
        attributes: Map<String, ByteArray?>,
        host: String?,
        port: Int,
    ) {
        val device = SyncLogic.parseNsdAttributes(attributes, host, port, serviceName) ?: return
        synchronized(discoveredByService) {
            discoveredByService[serviceName] = device
            publishLocked()
        }
    }

    private fun publishLocked() {
        val snapshot = SyncLogic.filterSelf(
            discoveredByService.values.distinctBy { it.deviceId },
            selfDeviceId,
        ).sortedBy { it.deviceName.lowercase() }
        main.post { onChanged(snapshot) }
    }
}
