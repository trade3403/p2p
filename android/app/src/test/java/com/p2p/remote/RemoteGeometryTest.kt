package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertNotNull
import org.junit.Test

class RemoteGeometryTest {
    @Test fun zoomedCursorHitsTheCorrectRemotePixel() {
        val center = RemoteGeometry.transformedPointer(
            160f,90f,320f,180f,1920,1080,2f,0f,0f
        )
        assertNotNull(center)
        assertEquals(0.5f,center!!.first,0.0001f)
        assertEquals(0.5f,center.second,0.0001f)
        val right = RemoteGeometry.transformedPointer(
            240f,90f,320f,180f,1920,1080,2f,0f,0f
        )
        assertEquals(0.625f,right!!.first,0.0001f)
        val shifted = RemoteGeometry.transformedPointer(
            160f,90f,320f,180f,1920,1080,2f,80f,0f
        )
        assertEquals(0.375f,shifted!!.first,0.0001f)
    }

    @Test fun zoomedLetterboxIsNeverClickable() {
        // 4:3 PC within 16:9 phone viewport: leftmost video black bar.
        assertNull(RemoteGeometry.transformedPointer(
            0f,90f,320f,180f,1024,768,1f,0f,0f
        ))
        assertNull(RemoteGeometry.transformedPointer(
            319f,90f,320f,180f,1024,768,2f,-160f,0f
        ))
    }

    @Test fun panRemainsBoundedAtAllZoomLevels() {
        assertEquals(0f,RemoteGeometry.boundPan(100f,1f,320f),0.001f)
        assertEquals(160f,RemoteGeometry.boundPan(999f,2f,320f),0.001f)
        assertEquals(-160f,RemoteGeometry.boundPan(-999f,2f,320f),0.001f)
    }

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
