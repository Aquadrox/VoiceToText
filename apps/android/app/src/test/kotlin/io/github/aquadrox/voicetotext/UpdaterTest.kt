package io.github.aquadrox.voicetotext

import android.content.pm.PackageInstaller
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class UpdaterTest {
    private val validUrl =
        "https://github.com/Aquadrox/VoiceToText/releases/download/v0.36.0/VoiceToText_0.36.0_android.apk"
    private val validSha = "a".repeat(64)

    private fun entry(
        url: String = validUrl,
        sha256: String = validSha,
        size: Long = 12_345_678L,
    ): JSONObject = JSONObject()
        .put("url", url)
        .put("sha256", sha256)
        .put("size", size)

    @Test
    fun codeParsesValidSemverAndRejectsOutOfBounds() {
        assertEquals(0L, Updater.code("0.0.0"))
        assertEquals(2100L, Updater.code("0.21.0"))
        assertEquals(3501L, Updater.code(" 0.35.1 "))
        assertEquals(200_009_999L, Updater.code("20000.99.99"))

        assertNull(Updater.code(""))
        assertNull(Updater.code("0.21"))
        assertNull(Updater.code("0.21.0.1"))
        assertNull(Updater.code("0.100.0"))
        assertNull(Updater.code("0.1.100"))
        assertNull(Updater.code("20001.0.0"))
        assertNull(Updater.code("-1.0.0"))
        assertNull(Updater.code("v0.21.0"))
    }

    @Test
    fun releaseAcceptsValidEntryNormalizesShaFiltersNotesAndCapsChangelog() {
        val hugeChangelog = "x".repeat(60 * 1024)
        val rel = Updater.release(
            entry = entry(sha256 = "AB".repeat(32)),
            version = "0.36.0",
            notes = "https://github.com/Aquadrox/VoiceToText/releases/tag/v0.36.0",
            changelog = hugeChangelog,
        )
        assertEquals("0.36.0", rel.version)
        assertEquals(3600L, rel.code)
        assertEquals("ab".repeat(32), rel.sha256)
        assertEquals(12_345_678L, rel.size)
        assertEquals("https://github.com/Aquadrox/VoiceToText/releases/tag/v0.36.0", rel.notes)
        assertEquals(48 * 1024, rel.changelog.length)

        // Aller-retour JSON : relu avec les mêmes contrôles.
        val roundTripJson = JSONObject(rel.toJson())
        val reparsed = Updater.release(
            roundTripJson,
            roundTripJson.getString("version"),
            roundTripJson.optString("notes"),
            roundTripJson.optString("changelog"),
        )
        assertEquals(rel.version, reparsed.version)
        assertEquals(rel.sha256, reparsed.sha256)
        assertEquals(rel.size, reparsed.size)

        // Lien de notes hors dépôt ignoré (vidé sans faire échouer la release).
        val foreignNotes = Updater.release(
            entry = entry(),
            version = "0.36.0",
            notes = "https://evil.example/notes",
            changelog = "ok",
        )
        assertEquals("", foreignNotes.notes)
    }

    @Test
    fun releaseRejectsUntrustedUrlTraversalBadShaBadSizeAndBadVersion() {
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(entry(url = "https://example.com/app.apk"), "0.36.0", "", "")
        }
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(
                entry(url = "https://github.com/Aquadrox/VoiceToText/releases/download/v0.36.0/../../evil.apk"),
                "0.36.0",
                "",
                "",
            )
        }
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(entry(sha256 = "abcd"), "0.36.0", "", "")
        }
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(entry(sha256 = "g".repeat(64)), "0.36.0", "", "")
        }
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(entry(size = 0L), "0.36.0", "", "")
        }
        assertThrows(IllegalArgumentException::class.java) {
            Updater.release(entry(size = 1024L * 1024 * 1024 + 1), "0.36.0", "", "")
        }
        assertThrows(IllegalStateException::class.java) {
            Updater.release(entry(), "pas-une-version", "", "")
        }
    }

    @Test
    fun failureFormatsKnownAndUnknownInstallerStatuses() {
        assertEquals(
            "Mise à jour non installée : installation annulée.",
            Updater.failure(PackageInstaller.STATUS_FAILURE_ABORTED, "   "),
        )
        assertTrue(
            Updater.failure(PackageInstaller.STATUS_FAILURE_BLOCKED, "Play Protect")
                .contains("Play Protect"),
        )
        assertEquals(
            "Mise à jour non installée : espace de stockage insuffisant : disque plein.",
            Updater.failure(PackageInstaller.STATUS_FAILURE_STORAGE, "disque plein"),
        )
        assertEquals(
            "Mise à jour non installée : échec de l'installation (code 99).",
            Updater.failure(99, null),
        )
    }
}
