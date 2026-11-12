package com.p2p.remote

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Surface
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Bounded Annex-B H.264 access-unit decoder rendered to a TextureView Surface.
 * MediaCodec (hardware when the device provides an AVC HW decoder) runs off UI.
 * A new encoder GOP carries SPS/PPS after reconnect or a mode switch.
 */
internal class H264SurfaceDecoder(
    private val onRendered: () -> Unit,
    private val onDecoderError: (String) -> Unit,
) {
    private val thread = HandlerThread("P2PRemote-H264").apply { start() }
    private val handler = Handler(thread.looper)
    private val pending = AtomicInteger(0)
    private val closed = AtomicBoolean(false)

    private var surface: Surface? = null
    private var codec: MediaCodec? = null
    private var width = 0
    private var height = 0
    private var lastErrorAt = 0L

    fun attachSurface(newSurface: Surface?) {
        if (closed.get()) {
            newSurface?.release()
            return
        }
        handler.post {
            closeCodec()
            surface?.release()
            surface = newSurface
        }
    }

    fun offer(packet: ByteArray) {
        if (closed.get()) return
        val header = AvcPayload.parse(packet) ?: return
        val w = header.width
        val h = header.height
        if (pending.incrementAndGet() > 2) {
            pending.decrementAndGet()
            return
        }
        val accepted = handler.post {
            try {
                if (!closed.get()) decode(w, h, packet)
            } catch (error: Exception) {
                closeCodec()
                val now = SystemClock.elapsedRealtime()
                if (now - lastErrorAt > 5000) {
                    lastErrorAt = now
                    onDecoderError("H.264 解码失败：${error.javaClass.simpleName}")
                }
            } finally {
                pending.decrementAndGet()
            }
        }
        if (!accepted) pending.decrementAndGet()
    }

    private fun decode(w: Int, h: Int, packet: ByteArray) {
        val target = surface ?: return
        if (!target.isValid) return
        if (codec == null || width != w || height != h) {
            closeCodec()
            val decoder = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            try {
                decoder.configure(MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h), target, null, 0)
                decoder.start()
            } catch (error: Exception) {
                decoder.release()
                throw error
            }
            codec = decoder
            width = w
            height = h
        }
        val decoder = codec ?: return
        val accessUnitSize = packet.size - 4
        val inputId = decoder.dequeueInputBuffer(5000L)
        if (inputId < 0) return // Do not allow an unbounded frame backlog.
        val input = decoder.getInputBuffer(inputId)
        if (input == null || input.capacity() < accessUnitSize) {
            decoder.queueInputBuffer(inputId, 0, 0, 0, 0)
            return
        }
        input.clear()
        input.put(packet, 4, accessUnitSize)
        val ptsUs = SystemClock.elapsedRealtimeNanos() / 1000L
        decoder.queueInputBuffer(inputId, 0, accessUnitSize, ptsUs, 0)
        val info = MediaCodec.BufferInfo()
        repeat(6) {
            when (val outputId = decoder.dequeueOutputBuffer(info, 0L)) {
                MediaCodec.INFO_TRY_AGAIN_LATER -> return
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> Unit
                else -> {
                    if (outputId >= 0) {
                        decoder.releaseOutputBuffer(outputId, info.size > 0)
                        if (info.size > 0) onRendered()
                    }
                }
            }
        }
    }

    private fun closeCodec() {
        val decoder = codec ?: return
        codec = null
        width = 0
        height = 0
        try { decoder.stop() } catch (_: Exception) {}
        try { decoder.release() } catch (_: Exception) {}
    }

    fun reset() {
        if (!closed.get()) handler.post { closeCodec() }
    }

    fun close() {
        if (!closed.compareAndSet(false, true)) return
        handler.post {
            closeCodec()
            surface?.release()
            surface = null
            thread.quitSafely()
        }
    }
}
