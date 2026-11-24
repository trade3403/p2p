package com.p2p.remote

import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaCodecList
import android.os.Build
import java.util.concurrent.atomic.AtomicLong
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

    @Volatile var codecName = ""
        private set
    @Volatile var hardware: Boolean? = null
        private set
    private val drops=AtomicLong()
    val droppedFrames:Long get()=drops.get()
    private val needsIdr=AtomicBoolean(true)
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
            drops.incrementAndGet()
            needsIdr.set(true)
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
        // Never decode a dependent P frame after a dropped access unit.
        val idr=containsIdr(packet)
        if (needsIdr.get() || codec == null || width != w || height != h) {
            if(!idr) return
            closeCodec()
            val format=MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC,w,h)
            if(Build.VERSION.SDK_INT>=30) format.setInteger(MediaFormat.KEY_LOW_LATENCY,1)
            val candidates=MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                .filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(MediaFormat.MIMETYPE_VIDEO_AVC,true) } }
                .sortedByDescending { if(Build.VERSION.SDK_INT>=29 && it.isHardwareAccelerated) 1 else 0 }
            for(info in candidates) {
                var candidate:MediaCodec?=null
                try {
                    candidate=MediaCodec.createByCodecName(info.name)
                    candidate.configure(format,target,null,0)
                    candidate.start()
                    codec=candidate
                    codecName=info.name
                    // Emulator virtual codecs may advertise hardware acceleration even
                    // when no physical hardware decoder is used by the guest.
                    val virtualCodec = info.name.contains("goldfish", ignoreCase = true) ||
                        info.name.contains("ranchu", ignoreCase = true)
                    val emulator = Build.HARDWARE.contains("goldfish", ignoreCase = true) ||
                        Build.HARDWARE.contains("ranchu", ignoreCase = true) ||
                        Build.FINGERPRINT.startsWith("generic") ||
                        Build.FINGERPRINT.contains("sdk_gphone", ignoreCase = true)
                    hardware = when {
                        virtualCodec -> false
                        emulator -> if (Build.VERSION.SDK_INT >= 29 && info.isSoftwareOnly) false else null
                        Build.VERSION.SDK_INT >= 29 -> info.isHardwareAccelerated
                        else -> null
                    }
                    width=w
                    height=h
                    needsIdr.set(false)
                    break
                } catch (_:Exception) { try { candidate?.release() } catch (_:Exception) {} }
            }
            check(codec!=null) {"No usable AVC decoder"}
        }
        val decoder = codec ?: return
        val accessUnitSize = packet.size - 4
        val inputId = decoder.dequeueInputBuffer(5000L)
        if (inputId < 0) {
            drops.incrementAndGet()
            needsIdr.set(true)
            return
        }
        val input = decoder.getInputBuffer(inputId)
        if (input == null || input.capacity() < accessUnitSize) {
            decoder.queueInputBuffer(inputId, 0, 0, 0, 0)
            drops.incrementAndGet()
            needsIdr.set(true)
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

    private fun containsIdr(packet:ByteArray):Boolean {
        for(i in 4 until packet.size-3) {
            if(packet[i]==0.toByte() && packet[i+1]==0.toByte() && packet[i+2]==1.toByte()
                && (packet[i+3].toInt() and 31)==5) return true
        }
        return false
    }

    private fun closeCodec() {
        needsIdr.set(true)
        codecName=""
        hardware=null
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
