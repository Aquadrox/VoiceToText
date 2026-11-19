package io.github.aquadrox.voicetotext

import io.github.aquadrox.voicetotext.Meetings.objects
import io.github.aquadrox.voicetotext.Meetings.text
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Calendar

class MeetingsTest {
    @Test
    fun clockFormatsMinutesAndHours() {
        assertEquals("00:00", Meetings.clock(0))
        assertEquals("00:59", Meetings.clock(59_000))
        assertEquals("12:34", Meetings.clock(754_000))
        assertEquals("1:02:03", Meetings.clock(3_723_000))
    }

    @Test
    fun groupAndDateClassifyRelativeDays() {
        val now = Calendar.getInstance()
        assertEquals("Aujourd'hui", Meetings.group(now.timeInMillis))
        assertTrue(Meetings.date(now.timeInMillis).startsWith("Aujourd'hui, "))

        val yesterday = (now.clone() as Calendar).apply { add(Calendar.DAY_OF_YEAR, -1) }
        assertEquals("Hier", Meetings.group(yesterday.timeInMillis))
        assertTrue(Meetings.date(yesterday.timeInMillis).startsWith("Hier, "))

        val threeDaysAgo = (now.clone() as Calendar).apply { add(Calendar.DAY_OF_YEAR, -3) }
        assertEquals("Cette semaine", Meetings.group(threeDaysAgo.timeInMillis))

        val lastYear = (now.clone() as Calendar).apply { add(Calendar.YEAR, -1) }
        assertEquals(lastYear.get(Calendar.YEAR).toString(), Meetings.group(lastYear.timeInMillis))
    }

    @Test
    fun metaSummarizesAllMeetingStates() {
        assertEquals(
            "réunion · enregistrement en cours",
            Meetings.meta(JSONObject().put("status", "enregistrement")),
        )
        assertEquals(
            "importée · cours ou conférence · lecture du fichier",
            Meetings.meta(
                JSONObject()
                    .put("imported", true)
                    .put("conference", true)
                    .put("status", "enregistrement"),
            ),
        )
        assertEquals(
            "réunion · 3 intervenant(s) · transcription 42 %",
            Meetings.meta(
                JSONObject()
                    .put("speakers", 3)
                    .put("status", "transcription")
                    .put("step", "transcription")
                    .put("progress", 42),
            ),
        )
        assertEquals(
            "réunion · intervenants 80 %",
            Meetings.meta(
                JSONObject()
                    .put("status", "transcription")
                    .put("step", "speakers")
                    .put("progress", 80),
            ),
        )
        assertEquals(
            "réunion · interrompue",
            Meetings.meta(JSONObject().put("status", "interrompue")),
        )
        assertEquals(
            "réunion · compte rendu · rapport",
            Meetings.meta(
                JSONObject()
                    .put("status", "terminee")
                    .put("summary", true)
                    .put("report", true),
            ),
        )
        assertEquals(
            "réunion · transcription seule",
            Meetings.meta(JSONObject().put("status", "terminee")),
        )
        assertEquals(
            "réunion · rapport 55 %",
            Meetings.meta(
                JSONObject()
                    .put("status", "terminee")
                    .put("step", "report")
                    .put("progress", 55),
            ),
        )
    }

    @Test
    fun costFormatsTinyRegularAndUnknownCurrency() {
        val tiny = Meetings.cost(0.0001234, "USD")
        assertTrue("attendu chiffres significatifs, reçu: $tiny", tiny.contains("0,00012"))

        val regular = Meetings.cost(1.5, "EUR")
        assertTrue("attendu 1,50, reçu: $regular", regular.contains("1,50"))

        val fallback = Meetings.cost(0.25, "INCONNUE")
        assertTrue("attendu devise en repli, reçu: $fallback", fallback.endsWith("INCONNUE"))
    }

    @Test
    fun jsonHelpersHandleNullMissingAndObjectArrays() {
        val obj = JSONObject()
            .put("title", "Point hebdo")
            .put("empty", JSONObject.NULL)
            .put("literalNull", "null")
        assertEquals("Point hebdo", obj.text("title"))
        assertEquals("", obj.text("empty"))
        assertEquals("", obj.text("literalNull"))
        assertEquals("", obj.text("absent"))

        val arr = JSONArray()
            .put(JSONObject().put("id", 1))
            .put(JSONObject().put("id", 2))
        assertEquals(listOf(1, 2), arr.objects().map { it.getInt("id") })
    }
}
