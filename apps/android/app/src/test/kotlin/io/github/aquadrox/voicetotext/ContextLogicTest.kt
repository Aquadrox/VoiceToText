package io.github.aquadrox.voicetotext

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ContextLogicTest {
    @Test
    fun enforcesOptInPrivacyAndMode() {
        // Opt-in obligatoire.
        assertFalse(ContextLogic.canReadContext(enabled = false, privacy = Native.NORMAL, mode = Native.DICTATE))
        // Mot de passe et navigation privée / sans historique : strictement bloqués.
        assertFalse(ContextLogic.canReadContext(enabled = true, privacy = Native.PASSWORD, mode = Native.DICTATE))
        assertFalse(ContextLogic.canReadContext(enabled = true, privacy = Native.NO_HISTORY, mode = Native.DICTATE))
        // Command Mode : la sélection seule sert.
        assertFalse(ContextLogic.canReadContext(enabled = true, privacy = Native.NORMAL, mode = Native.COMMAND))
        // Dictée et traduction sur champ normal avec opt-in : autorisés.
        assertTrue(ContextLogic.canReadContext(enabled = true, privacy = Native.NORMAL, mode = Native.DICTATE))
        assertTrue(ContextLogic.canReadContext(enabled = true, privacy = Native.NORMAL, mode = Native.TRANSLATE))
    }

    @Test
    fun buildsBoundedCursorContext() {
        assertEquals("", ContextLogic.aroundCursor(null, null))
        assertEquals("", ContextLogic.aroundCursor("   ", "\n"))
        assertEquals("Bonjour Élodie", ContextLogic.aroundCursor("Bonjour", "Élodie"))
        assertEquals("Bonjour Élodie", ContextLogic.aroundCursor("Bonjour ", "Élodie"))

        val hugeBefore = "a".repeat(3_000)
        val hugeAfter = "b".repeat(3_000)
        val bounded = ContextLogic.aroundCursor(hugeBefore, hugeAfter)
        assertEquals(ContextLogic.MAX_CONTEXT_CHARS, bounded.length)
    }

    @Test
    fun extractsBoundedWindowAroundSelection() {
        assertEquals("", ContextLogic.windowAroundSelection("   ", 0, 0))
        assertEquals("Bonjour Élodie", ContextLogic.windowAroundSelection("Bonjour Élodie", 7, 7))

        val prefix = "x".repeat(3_000)
        val middle = "ContextManager"
        val suffix = "y".repeat(3_000)
        val full = prefix + middle + suffix
        val slice = ContextLogic.windowAroundSelection(full, prefix.length, prefix.length + middle.length)
        assertTrue(slice.contains("ContextManager"))
        assertTrue(slice.length <= ContextLogic.MAX_CONTEXT_CHARS)

        // Bornes inversées ou négatives gérées sans erreur.
        val inverted = ContextLogic.windowAroundSelection("Bonjour Élodie", 14, 0)
        assertEquals("Bonjour Élodie", inverted)
        val noCursor = ContextLogic.windowAroundSelection("Bonjour Élodie", -1, -1)
        assertEquals("Bonjour Élodie", noCursor)
    }

    @Test
    fun parsesAndFormatsExcludedApps() {
        val parsed = ContextLogic.parseExcludedApps(" whatsapp, , WHATSAPP ; banque\nfr.gouv.impots ")
        assertEquals(listOf("whatsapp", "banque", "fr.gouv.impots"), parsed)
        assertEquals("whatsapp, banque, fr.gouv.impots", ContextLogic.formatExcludedApps(parsed))
    }

    @Test
    fun settingsSearchFindsScreenContext() {
        val hits = SettingsIndex.search("contexte", bubble = true)
        assertTrue(hits.any { it.key == "screen_context" && it.section == Section.DICTATION })
    }
}
