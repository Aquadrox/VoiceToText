package io.github.aquadrox.voicetotext

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveLogicTest {
    @Test
    fun enforcesPrivacyAndModeForLivePreview() {
        // Dictée et traduction sur champ normal ou sans historique : autorisées.
        assertTrue(LiveLogic.canUseLive(Native.NORMAL, Native.DICTATE))
        assertTrue(LiveLogic.canUseLive(Native.NORMAL, Native.TRANSLATE))
        assertTrue(LiveLogic.canUseLive(Native.NO_HISTORY, Native.DICTATE))
        assertTrue(LiveLogic.canUseLive(Native.NO_HISTORY, Native.TRANSLATE))

        // Mot de passe : jamais de session ni d'aperçu en direct.
        assertFalse(LiveLogic.canUseLive(Native.PASSWORD, Native.DICTATE))
        assertFalse(LiveLogic.canUseLive(Native.PASSWORD, Native.TRANSLATE))

        // Command Mode : maintien uniquement, pas de flux incrémental.
        assertFalse(LiveLogic.canUseLive(Native.NORMAL, Native.COMMAND))
        assertFalse(LiveLogic.canUseLive(Native.NO_HISTORY, Native.COMMAND))
        assertFalse(LiveLogic.canUseLive(Native.PASSWORD, Native.COMMAND))
    }

    @Test
    fun neverShowsPreviewTextOnPasswordFields() {
        assertFalse(LiveLogic.canShowPreview(Native.PASSWORD, "mon mot de passe"))
        assertFalse(LiveLogic.canShowPreview(Native.NORMAL, "   "))
        assertTrue(LiveLogic.canShowPreview(Native.NORMAL, "… derniers mots"))
        assertTrue(LiveLogic.canShowPreview(Native.NO_HISTORY, "… derniers mots"))

        assertEquals(
            "Mains libres : appuyez pour terminer",
            LiveLogic.handsFreeStatus(Native.DICTATE, "anglais", Native.PASSWORD, "secret123"),
        )
        assertEquals(
            "Mains libres → anglais : appuyez pour terminer",
            LiveLogic.handsFreeStatus(Native.TRANSLATE, "anglais", Native.NORMAL, ""),
        )
        assertEquals(
            "… bonjour tout le monde",
            LiveLogic.handsFreeStatus(Native.DICTATE, "anglais", Native.NORMAL, "  … bonjour tout le monde  "),
        )
    }

    @Test
    fun parsesStartFeedAndErrorResponses() {
        assertEquals(7L, LiveLogic.parseStartSession("""{"ok":7}"""))
        assertNull(LiveLogic.parseStartSession("""{"ok":0}"""))
        assertNull(LiveLogic.parseStartSession("""{"error":"moteur non démarré"}"""))
        assertNull(LiveLogic.parseStartSession("invalide"))

        assertEquals("moteur non démarré", LiveLogic.parseError("""{"error":"moteur non démarré"}"""))
        assertNull(LiveLogic.parseError("""{"ok":7}"""))
        assertNull(LiveLogic.parseError("""{"error":null}"""))

        val feed = LiveLogic.parseFeed("""{"ok":{"changed":true,"preview":"… texte en direct","segments":2}}""")
        assertEquals(LiveFeedResult(changed = true, preview = "… texte en direct", segments = 2), feed)

        val unchanged = LiveLogic.parseFeed("""{"ok":{"changed":false,"preview":"","segments":0}}""")
        assertEquals(LiveFeedResult(changed = false, preview = "", segments = 0), unchanged)

        assertNull(LiveLogic.parseFeed("""{"error":"boom"}"""))
        assertNull(LiveLogic.parseFeed("pas du json"))
    }
}
