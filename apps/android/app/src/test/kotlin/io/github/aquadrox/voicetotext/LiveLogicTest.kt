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

    @Test
    fun parsesActionsToInsertFromFeedAndFinish() {
        val feed = LiveLogic.parseFeed(
            """{"ok":{"changed":true,"preview":"Bonjour.","segments":1,"actions":[{"kind":"text","text":"Bonjour."},{"kind":"undo","text":""},{"kind":"inconnu","text":"x"}]}}""",
        )!!
        assertEquals(listOf(LiveAction("text", "Bonjour."), LiveAction("undo", "")), feed.actions)
        assertTrue(feed.actions[1].isUndo)
        // Sans insertion au fil de la dictée : aucune action.
        val plain = LiveLogic.parseFeed("""{"ok":{"changed":true,"preview":"a","segments":1}}""")!!
        assertTrue(plain.actions.isEmpty())
        val finish = org.json.JSONObject("""{"kind":"live","text":"A. B.","actions":[{"kind":"text","text":" B."}]}""")
        assertEquals(listOf(LiveAction("text", " B.")), LiveLogic.parseActions(finish))
    }

    @Test
    fun spacesOnlyTheFirstPieceAfterAWord() {
        assertEquals(" Bonjour.", LiveLogic.spaced("mot", "Bonjour.", first = true))
        assertEquals("Bonjour.", LiveLogic.spaced("", "Bonjour.", first = true))
        assertEquals("Bonjour.", LiveLogic.spaced("mot ", "Bonjour.", first = true))
        assertEquals(" Suite.", LiveLogic.spaced("mot", " Suite.", first = true))
        // Les morceaux suivants arrivent avec leur séparateur.
        assertEquals("Suite.", LiveLogic.spaced("fin.", "Suite.", first = false))
    }

    @Test
    fun piecesUndoTheLastOneThenTheWholeDictationIsKnown() {
        val pieces = LivePieces()
        assertTrue(pieces.isEmpty)
        assertEquals(0, pieces.undoLast())
        pieces.added(9)
        pieces.added(0)
        pieces.added(6)
        assertEquals(15, pieces.total)
        assertEquals(6, pieces.undoLast())
        assertEquals(9, pieces.total)
        pieces.clear()
        assertTrue(pieces.isEmpty)
    }

    @Test
    fun holdStatusShowsPreviewExceptOnPasswords() {
        assertEquals("bonjour", LiveLogic.holdStatus(Native.DICTATE, "anglais", Native.NORMAL, " bonjour "))
        assertEquals("Écoute…", LiveLogic.holdStatus(Native.DICTATE, "anglais", Native.PASSWORD, "secret"))
        assertEquals("Écoute… → anglais", LiveLogic.holdStatus(Native.TRANSLATE, "anglais", Native.NORMAL, ""))
    }
}
