package com.p2p.remote

import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaCodecList
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Surface

/**
 * Bounded Annex-B H.264 access-unit decoder rendered to a TextureView Surface.
 * MediaCodec (hardware when the device provides an AVC HW decoder) runs off UI.
 * A new encoder GOP carries SPS/PPS after reconnect or a mode switch.
 */
internal class H264SurfaceDecoder(
    private val onRendered: (Long) -> Unit,
    private val onDecoderError: (Long, String) -> Unit,
) {
    private val thread = HandlerThread("P2PRemote-H264").apply { start() }
    private val handler = Handler(thread.looper)
    private val admissions = AvcDecodeQueue()
    fun isCurrent(g: Long): Boolean = admissions.isCurrent(g)

    @Volatile var codecName = ""
        private set
    @Volatile var hardware: Boolean? = null
        private set
    val droppedFrames:Long get()=admissions.droppedFrames
    // Worker-owned, never cleared by an older task on behalf of a later loss.
    private var needsIdr = true
    private var workerTicket: AvcDecodeQueue.Ticket? = null
    private var surface: Surface? = null
    private var codec: MediaCodec? = null
    private var width = 0
    private var height = 0
    private var lastErrorAt = 0L

    // Serialize submission and lifecycle posting, including callers from
    // different threads. MediaCodec operations remain on the worker.
    @Synchronized fun attachSurface(newSurface: Surface?) {
        if (!admissions.invalidate()) {
            newSurface?.release()
            return
        }
        handler.post {
            closeCodec()
            workerTicket = null
            lastErrorAt = 0L
            surface?.release()
            surface = newSurface
        }
    }

    @Synchronized fun offer(packet: ByteArray) {
        val header = AvcPayload.parse(packet) ?: return
        val ticket = admissions.admit(header.width, header.height) ?: return
        val g = ticket.generation
        val accepted = handler.post {
            try {
                if (isCurrent(g)) {
                    // The first admitted unit after a producer-side loss
                    // must not inherit a decoder with missing references.
                    if (workerTicket != ticket) {
                        closeCodec()
                        if (workerTicket?.generation != g) lastErrorAt = 0L
                        workerTicket = ticket
                    }
                    decode(header, g)
                }
            } catch (error: Exception) {
                closeCodec()
                val now = SystemClock.elapsedRealtime()
                if (isCurrent(g) && now - lastErrorAt > 5000) {
                    lastErrorAt = now
                    onDecoderError(g, "H.264 解码失败：${error.javaClass.simpleName}")
                }
            } finally {
                admissions.complete(ticket)
            }
        }
        if (!accepted) admissions.complete(ticket)
    }

    private fun decode(unit: AvcPayload, g: Long) {
        val target = surface ?: return
        if (!target.isValid) return
        val w = unit.width
        val h = unit.height
        val packet = unit.bytes
        // Only scan for a random-access unit when recovery is necessary.
        // A fresh decoder cannot use parameter sets from the retired codec.
        if (needsIdr || codec == null || width != w || height != h) {
            if (!unit.hasSelfContainedIdr()) return
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
                    needsIdr = false
                    break
                } catch (_:Exception) { try { candidate?.release() } catch (_:Exception) {} }
            }
            check(codec!=null) {"No usable AVC decoder"}
        }
        val decoder = codec ?: return
        val accessUnitSize = packet.size - 4
        val inputId = decoder.dequeueInputBuffer(5000L)
        if (inputId < 0) {
            admissions.recordWorkerDrop(g)
            needsIdr = true
            return
        }
        val input = decoder.getInputBuffer(inputId)
        if (input == null || input.capacity() < accessUnitSize) {
            decoder.queueInputBuffer(inputId, 0, 0, 0, 0)
            admissions.recordWorkerDrop(g)
            needsIdr = true
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
                        // A reset cannot occur between the generation check
                        // and submission of this output buffer to the Surface.
                        val rendered = info.size > 0 && admissions.ifCurrent(g) {
                            decoder.releaseOutputBuffer(outputId, true)
                        }
                        if (!rendered) decoder.releaseOutputBuffer(outputId, false)
                        if (rendered && isCurrent(g)) onRendered(g)
                    }
                }
            }
        }
    }

    private fun closeCodec() {
        needsIdr = true
        codecName=""
        hardware=null
        val decoder = codec ?: return
        codec = null
        width = 0
        height = 0
        try { decoder.stop() } catch (_: Exception) {}
        try { decoder.release() } catch (_: Exception) {}
    }

    @Synchronized fun reset() {
        if (admissions.invalidate()) {
            handler.post {
                closeCodec()
                workerTicket = null
                lastErrorAt = 0L
            }
        }
    }

    @Synchronized fun close() {
        if (!admissions.close()) return
        handler.post {
            closeCodec()
            workerTicket = null
            surface?.release()
            surface = null
            thread.quitSafely()
        }
    }
}
