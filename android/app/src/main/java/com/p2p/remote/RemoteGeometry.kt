package com.p2p.remote

import kotlin.math.min

/**
 * Convert a touch on the remote-video viewport into a normalized coordinate.
 * Black bars around a scaled desktop are NOT interactive, preventing accidental
 * clicks on the remote edge when the phone and PC have different aspect ratios.
 */
internal object RemoteGeometry {
    fun normalizedPointer(
        x: Float, y: Float,
        surfaceW: Float, surfaceH: Float,
        imageW: Int, imageH: Int
    ): Pair<Float, Float>? {
        if (!x.isFinite() || !y.isFinite() || !surfaceW.isFinite() || !surfaceH.isFinite()
            || surfaceW <= 0f || surfaceH <= 0f || imageW <= 0 || imageH <= 0) return null
        val scale = min(surfaceW / imageW.toFloat(), surfaceH / imageH.toFloat())
        val drawnW = imageW.toFloat() * scale
        val drawnH = imageH.toFloat() * scale
        if (drawnW <= 0f || drawnH <= 0f) return null
        val left = (surfaceW - drawnW) / 2f
        val top = (surfaceH - drawnH) / 2f
        if (x < left || y < top || x > left + drawnW || y > top + drawnH) return null
        return Pair(
            ((x - left) / drawnW).coerceIn(0f, 1f),
            ((y - top) / drawnH).coerceIn(0f, 1f)
        )
    }
}
