package io.github.aquadrox.voicetotext

import android.media.AudioDeviceInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MicrophonesTest {
    @Test
    fun rankOrdersRealMicrophonesAndFiltersNonInputs() {
        assertEquals(0, Microphones.rank(AudioDeviceInfo.TYPE_BUILTIN_MIC))
        assertEquals(1, Microphones.rank(AudioDeviceInfo.TYPE_WIRED_HEADSET))
        assertEquals(2, Microphones.rank(AudioDeviceInfo.TYPE_USB_HEADSET))
        assertEquals(2, Microphones.rank(AudioDeviceInfo.TYPE_USB_DEVICE))
        assertEquals(3, Microphones.rank(AudioDeviceInfo.TYPE_BLUETOOTH_SCO))
        assertEquals(3, Microphones.rank(AudioDeviceInfo.TYPE_BLE_HEADSET))
        assertEquals(4, Microphones.rank(AudioDeviceInfo.TYPE_LINE_ANALOG))
        assertEquals(4, Microphones.rank(AudioDeviceInfo.TYPE_DOCK))

        // Téléphonie, tuner et flux internes écartés.
        assertNull(Microphones.rank(AudioDeviceInfo.TYPE_TELEPHONY))
        assertNull(Microphones.rank(AudioDeviceInfo.TYPE_FM_TUNER))
        assertNull(Microphones.rank(AudioDeviceInfo.TYPE_REMOTE_SUBMIX))
    }

    @Test
    fun bluetoothIdentifiesScoAndBleOnly() {
        assertTrue(Microphones.bluetooth(AudioDeviceInfo.TYPE_BLUETOOTH_SCO))
        assertTrue(Microphones.bluetooth(AudioDeviceInfo.TYPE_BLE_HEADSET))
        assertFalse(Microphones.bluetooth(AudioDeviceInfo.TYPE_BUILTIN_MIC))
        assertFalse(Microphones.bluetooth(AudioDeviceInfo.TYPE_USB_HEADSET))
    }

    @Test
    fun placeAndNamedProduceFrenchLabels() {
        assertEquals("bas", Microphones.place("bottom", 0))
        assertEquals("arrière", Microphones.place("BACK", 1))
        assertEquals("haut", Microphones.place("Top", 2))
        assertEquals("avant", Microphones.place("front", 3))
        assertEquals("n° 2", Microphones.place("unknown", 1))

        assertEquals("Micro USB", Microphones.named("Micro USB", ""))
        assertEquals("Micro USB : Rode NT-USB", Microphones.named("Micro USB", "Rode NT-USB"))
    }
}
