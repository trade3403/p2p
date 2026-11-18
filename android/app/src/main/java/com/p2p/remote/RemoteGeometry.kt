package com.p2p.remote

import kotlin.math.min

/**
 * Convert a touch on the remote-video viewport into a normalized coordinate.
 * Black bars around a scaled desktop are NOT interactive, preventing accidental
 * clicks on the remote edge when the phone and PC have different aspect ratios.
 */
internal object RemoteGeometry {
    // Inverse of centered UI graphicsLayer(scale, translation): transform
    // touch coordinates back into the unzoomed, letterboxed video canvas.
    fun transformedPointer(
        x: Float, y: Float, surfaceW: Float, surfaceH: Float,
        imageW: Int, imageH: Int, zoom: Float, panX: Float, panY: Float
    ): Pair<Float, Float>? {
        if (!zoom.isFinite() || zoom < 1f || !panX.isFinite() || !panY.isFinite()
            || surfaceW <= 0f || surfaceH <= 0f) return null
        val sourceX = (x - surfaceW / 2f - panX) / zoom + surfaceW / 2f
        val sourceY = (y - surfaceH / 2f - panY) / zoom + surfaceH / 2f
        return normalizedPointer(sourceX, sourceY, surfaceW, surfaceH, imageW, imageH)
    }

    fun boundPan(pan: Float, zoom: Float, span: Float): Float {
        if (!pan.isFinite() || !zoom.isFinite() || !span.isFinite()
            || span <= 0f) return 0f
        val max = ((zoom.coerceIn(1f, 4f) - 1f) * span / 2f).coerceAtLeast(0f)
        return pan.coerceIn(-max, max)
    }

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
