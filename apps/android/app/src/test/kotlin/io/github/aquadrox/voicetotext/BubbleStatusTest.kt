package io.github.aquadrox.voicetotext

import android.app.ApplicationExitInfo
import org.junit.Assert.assertEquals
import org.junit.Test

class BubbleStatusTest {
    @Test
    fun reasonLabelTranslatesKnownReasonsAndFallsBackToCode() {
        assertEquals("plantage de l'application", BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_CRASH))
        assertEquals("plantage du moteur de dictée", BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_CRASH_NATIVE))
        assertEquals("application bloquée (ANR)", BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_ANR))
        assertEquals("mémoire insuffisante", BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_LOW_MEMORY))
        assertEquals(
            "tuée par le système (signal 9)",
            BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_SIGNALED, status = 9),
        )
        assertEquals(
            "mise à jour de l'application",
            BubbleStatus.reasonLabel(ApplicationExitInfo.REASON_PACKAGE_UPDATED),
        )
        assertEquals("code 999", BubbleStatus.reasonLabel(999))
    }
}
