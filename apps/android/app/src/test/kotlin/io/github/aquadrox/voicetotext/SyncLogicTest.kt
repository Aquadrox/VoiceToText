package io.github.aquadrox.voicetotext

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SyncLogicTest {
    @Test
    fun extractsAndValidatesPairingUri() {
        val uri = "vttsync://v1?id=0123456789abcdef0123456789abcdef&n=PC&a=192.168.1.10:41739&k=abc&exp=9999"
        assertTrue(SyncLogic.isPairingUri(uri))
        assertFalse(SyncLogic.isPairingUri("https://example.com"))
        assertFalse(SyncLogic.isPairingUri("vttsync://v1?"))

        val pasted = "Lien d'appairage : $uri."
        assertEquals(uri, SyncLogic.extractPairingUri(pasted))
        assertNull(SyncLogic.extractPairingUri("rien ici"))
    }

    @Test
    fun parsesValidNsdAttributesAndRejectsInvalidOnes() {
        val id = "0123456789abcdef0123456789abcdef"
        val device = SyncLogic.parseNsdAttributes(
            attributes = mapOf(
                "ID" to id.uppercase().toByteArray(),
                "name" to "PC Bureau".toByteArray(),
            ),
            host = "192.168.1.42",
            port = 41739,
            fallbackName = "vtt-0123",
        )
        assertEquals(DiscoveredDevice(id, "PC Bureau", "192.168.1.42:41739"), device)

        // Port invalide ou ID manquant -> ignoré.
        assertNull(SyncLogic.parseNsdAttributes(mapOf("id" to id.toByteArray()), "192.168.1.42", 0, "x"))
        assertNull(SyncLogic.parseNsdAttributes(mapOf("id" to "court".toByteArray()), "192.168.1.42", 41739, "x"))
    }

    @Test
    fun resolvesAddressWithManualOverrideThenNsdThenFallback() {
        val id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        val peer = PairedDevice(
            deviceId = id,
            deviceName = "PC",
            fingerprint = "1111-2222-3333-4444",
            pairedAtMs = 1_000L,
            lastSyncMs = null,
            lastAddr = "192.168.1.10:41739",
            syncSettings = true,
            syncMeetings = true,
        )
        val nsd = listOf(DiscoveredDevice(id, "PC", "192.168.1.55:41739"))

        assertEquals("10.0.0.5:41739", SyncLogic.resolveSyncAddress(peer, nsd, " 10.0.0.5:41739 "))
        assertEquals("192.168.1.55:41739", SyncLogic.resolveSyncAddress(peer, nsd, ""))
        assertEquals("", SyncLogic.resolveSyncAddress(peer, emptyList(), ""))
    }

    @Test
    fun parsesPeersJsonAndFormatsSummary() {
        val json = JSONObject().put(
            "peers",
            JSONArray().put(
                JSONObject()
                    .put("device_id", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                    .put("device_name", "Steam Deck")
                    .put("fingerprint", "AAAA-BBBB-CCCC-DDDD")
                    .put("paired_at_ms", 1234L)
                    .put("last_addr", "192.168.1.30:41739")
                    .put("sync_settings", true)
                    .put("sync_meetings", false),
            ),
        )
        val peers = SyncLogic.parsePeers(json)
        assertEquals(1, peers.size)
        assertEquals("Steam Deck", peers[0].deviceName)
        assertNull(peers[0].lastSyncMs)
        assertEquals("192.168.1.30:41739", peers[0].lastAddr)
        assertFalse(peers[0].syncMeetings)

        assertTrue(SyncLogic.sectionSummary(0).startsWith("Aucun appareil"))
        assertTrue(SyncLogic.sectionSummary(1).startsWith("1 appareil"))
        assertTrue(SyncLogic.sectionSummary(3).startsWith("3 appareils"))
    }
}
