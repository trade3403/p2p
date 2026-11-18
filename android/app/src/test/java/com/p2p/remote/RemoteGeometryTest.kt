package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
import org.junit.Test

class RemoteGeometryTest {
    @Test fun centeredSixteenByNine() {
        val pointer = RemoteGeometry.normalizedPointer(160f, 90f, 320f, 180f, 1920, 1080)
        assertNotNull(pointer)
        assertEquals(0.5f, pointer!!.first, 0.0001f)
        assertEquals(0.5f, pointer.second, 0.0001f)
    }

    @Test fun letterboxedImageRejectsNonImageTaps() {
        // 4:3 desktop is pillarboxed when shown inside a 16:9 phone viewport.
        assertNull(RemoteGeometry.normalizedPointer(5f, 90f, 320f, 180f, 1024, 768))
        val center = RemoteGeometry.normalizedPointer(160f, 90f, 320f, 180f, 1024, 768)
        assertNotNull(center)
        assertEquals(0.5f, center!!.first, 0.0001f)
    }

    @Test fun horizontalImageLetterboxesTopAndBottom() {
        assertNull(RemoteGeometry.normalizedPointer(160f, 3f, 320f, 240f, 1920, 1080))
        assertNotNull(RemoteGeometry.normalizedPointer(160f, 120f, 320f, 240f, 1920, 1080))
    }

    @Test fun invalidGeometryNeverReturnsRemoteCoordinates() {
        assertNull(RemoteGeometry.normalizedPointer(Float.NaN, 0f, 320f, 240f, 1920, 1080))
        assertNull(RemoteGeometry.normalizedPointer(10f, 10f, 0f, 240f, 1920, 1080))
        assertNull(RemoteGeometry.normalizedPointer(10f, 10f, 320f, 240f, 0, 1080))
        assertNull(RemoteGeometry.normalizedPointer(-1f, 10f, 320f, 240f, 1920, 1080))
    }
}
