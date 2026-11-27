package com.p2p.remote

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.SurfaceTexture
import android.os.Handler
import android.os.HandlerThread
import android.opengl.EGL14
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.view.Surface
import android.os.SystemClock
import android.os.Build
import android.os.ParcelFileDescriptor
import android.net.ConnectivityManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.net.Inet4Address
import java.net.InetAddress
import java.net.NetworkInterface
import java.nio.ByteBuffer
import java.security.SecureRandom
import java.util.Collections
import java.util.concurrent.Executors
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.abs

/** Real native viewer and Windows WSS capture; LAN fixture reuses a received JPEG. */
@RunWith(AndroidJUnit4::class)
class PublicRelayE2eTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val colors = mapOf("ready" to intArrayOf(25,85,155),
        "clicked" to intArrayOf(32,145,72), "key" to intArrayOf(155,72,35),
        "recovered" to intArrayOf(95,45,145))
    private val mark = intArrayOf(230,190,35)
    private fun <T> main(block: () -> T): T {
        val result = AtomicReference<Result<T>>()
        inst.runOnMainSync { result.set(runCatching(block)) }
        return result.get().getOrThrow()
    }
    private fun await(label: String, ms: Long = 30_000, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + ms
        while (SystemClock.elapsedRealtime() < deadline) {
            if (condition()) return
            Thread.sleep(100)
        }
        throw AssertionError("Timed out: " + label)
    }
    // Android shell commands execute inside the disposable CI emulator.
    // Do not print their output (network identifiers may be present).
    private fun shell(command: String): String =
        ParcelFileDescriptor.AutoCloseInputStream(inst.uiAutomation.executeShellCommand(command))
            .bufferedReader().use { it.readText().trim() }

    private fun near(pixel: Int, rgb: IntArray) =
        abs(Color.red(pixel)-rgb[0]) < 40 && abs(Color.green(pixel)-rgb[1]) < 40 &&
        abs(Color.blue(pixel)-rgb[2]) < 40
    private fun state(bitmap: Bitmap): String? {
        for (x in listOf(.08,.92)) for (y in listOf(.08,.92))
            if (!near(bitmap.getPixel((bitmap.width*x).toInt(),(bitmap.height*y).toInt()),mark)) return null
        val p=bitmap.getPixel((bitmap.width*.25).toInt(),(bitmap.height*.25).toInt())
        return colors.entries.firstOrNull { near(p,it.value) }?.key
    }

    // A test-only LAN peer using real TCP/WebSocket and v4 AES-GCM. The JPEG
    // is a previously received Windows frame, NOT a live Windows LAN capture.
    private class LanPeer(private val root: ByteArray, private val jpeg: ByteArray,
        private val avc: List<ByteArray> = emptyList(),
        private val stallHandshake: Boolean = false,
        private val bindAddress: String? = null) : AutoCloseable {
        private val server = MockWebServer()
        private val worker = Executors.newSingleThreadScheduledExecutor()
        private val peer = AtomicReference<WebSocket>()
        private val error = AtomicReference<Throwable>()
        private val random = SecureRandom()
        private val host = ByteArray(32).also { random.nextBytes(it) }
        private var client: ByteArray? = null
        private var sequence = 0L
        private var business: ByteArray? = null
        private var received = 0L
        private val pongCount = AtomicLong()
        private val droppedPings = AtomicLong()
        private val suppressPong = java.util.concurrent.atomic.AtomicBoolean(false)
        val unansweredPings: Long get() = droppedPings.get()
        fun stopAnsweringPings() { suppressPong.set(true) }
        private val codecRequests = AtomicLong()
        private val sentVideo = AtomicLong()
        private val avcStarted = java.util.concurrent.atomic.AtomicBoolean(false)
        // The scheduled worker alone owns the current GOP/index. A switch
        // replaces the entire list at a packet boundary, not the WebSocket.
        private val nextVideo = AtomicReference<List<ByteArray>?>()
        private val videoSwitches = AtomicLong()
        val appliedVideoSwitches: Long get() = videoSwitches.get()
        fun switchAvc(units: List<ByteArray>) {
            require(avc.isNotEmpty() && units.isNotEmpty())
            check(nextVideo.compareAndSet(null,units)) { "Unapplied AVC fixture switch" }
        }
        private val helloCount = AtomicLong()
        val authenticatedHellos: Long get() = helloCount.get()
        val authenticatedPongs: Long get() = pongCount.get()
        val authenticatedCodecRequests: Long get() = codecRequests.get()
        val sentVideoPackets: Long get() = sentVideo.get()
        val failureType: String? get() = error.get()?.javaClass?.simpleName

        private fun startVideo(ws: WebSocket, key: ByteArray) {
            if (!avcStarted.compareAndSet(false,true)) return
            var index = 0
            var activeUnits = avc
            // Do not emit catch-up bursts after the emulator or worker stalls.
            worker.scheduleWithFixedDelay({
                try {
                    if (avc.isEmpty()) sendBusiness(ws,key,1,jpeg)
                    else {
                        val replacement = nextVideo.getAndSet(null)
                        if (replacement != null) {
                            activeUnits = replacement
                            index = 0
                            videoSwitches.incrementAndGet()
                        }
                        sendBusiness(ws,key,5,activeUnits[index])
                        index=(index+1)%activeUnits.size
                    }
                    sentVideo.incrementAndGet()
                } catch (failure: Throwable) { error.compareAndSet(null,failure) }
            }, 200, if(avc.isEmpty()) 200 else 100, TimeUnit.MILLISECONDS)
        }

        // Serializes the nonce allocation AND WebSocket enqueue order across
        // the video worker and the listener's authenticated PONG replies.
        @Synchronized private fun sendBusiness(ws: WebSocket, key: ByteArray, kind: Int, body: ByteArray) {
            val nonce = ByteBuffer.allocate(12).putInt(0x484f5354).putLong(++sequence).array()
            check(ws.send(encode(key,kind,body,nonce)))
        }
        val address: String

        private fun prefix(kind: String) = "P2P-LAN-$kind-v4".toByteArray(Charsets.UTF_8)
        private fun encode(key: ByteArray, kind: Int, body: ByteArray, nonce: ByteArray =
            ByteArray(12).also { random.nextBytes(it) }): ByteString {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key,"AES"), GCMParameterSpec(128,nonce))
            cipher.updateAAD(byteArrayOf(kind.toByte()))
            return (byteArrayOf(kind.toByte()) + nonce + cipher.doFinal(body)).toByteString()
        }
        private fun decode(key: ByteArray, packet: ByteArray): ByteArray {
            require(packet.size >= 29)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key,"AES"), GCMParameterSpec(128,packet.copyOfRange(1,13)))
            cipher.updateAAD(packet.copyOfRange(0,1))
            return cipher.doFinal(packet.copyOfRange(13,packet.size))
        }
        init {
            address = bindAddress ?: Collections.list(NetworkInterface.getNetworkInterfaces())
                .filter { it.isUp && !it.isLoopback }
                .flatMap { Collections.list(it.inetAddresses) }
                .filterIsInstance<Inet4Address>()
                .mapNotNull { it.hostAddress }
                .first { LanEndpoint.url(it) != null }
            server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: okhttp3.Response) {
                    peer.set(webSocket)
                }
                override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                    try {
                        // Only handshake messages use the root key. The fixture
                        // does not implement remote control or claim LAN input.
                        if (bytes.size == 0) return
                        val packet = bytes.toByteArray()
                        if (packet[0] == 8.toByte() || packet[0] == 2.toByte()) {
                            val key = requireNotNull(business)
                            val body = decode(key,packet)
                            val nonce = ByteBuffer.wrap(packet,1,12)
                            require(nonce.int == 0x56494557)
                            val n = nonce.long
                            require(n > received && n <= 0x7fff_ffffL)
                            received = n
                            if (packet[0] == 8.toByte()) {
                                require(body.size == 8)
                                if (suppressPong.get()) droppedPings.incrementAndGet()
                                else {
                                    sendBusiness(webSocket,key,9,body)
                                    pongCount.incrementAndGet()
                                }
                            } else {
                                val control=JSONObject(body.toString(Charsets.UTF_8))
                                if (control.optString("type")=="video_codec" &&
                                    control.optString("codec")=="h264") {
                                    codecRequests.incrementAndGet()
                                    if(avc.isNotEmpty()) startVideo(webSocket,key)
                                }
                            }
                            return
                        }
                        if (packet[0] != 6.toByte()) return
                        val body = decode(root,packet)
                        val current = client
                        if (current == null) {
                            val hello = prefix("HELLO")
                            require(body.size == hello.size + 32 &&
                                body.copyOfRange(0,hello.size).contentEquals(hello))
                            val fresh = body.copyOfRange(hello.size,body.size)
                            client = fresh
                            helloCount.incrementAndGet()
                            if (stallHandshake) return
                            check(webSocket.send(encode(root,7,prefix("CHALLENGE")+fresh+host)))
                        } else {
                            require(body.contentEquals(prefix("PROOF")+current+host))
                            val mac = Mac.getInstance("HmacSHA256")
                            mac.init(SecretKeySpec(root,"HmacSHA256"))
                            val key = mac.doFinal("P2P-LAN-BUSINESS-v4".toByteArray(Charsets.UTF_8)+current+host)
                            business = key
                            check(webSocket.send(encode(root,7,prefix("READY")+current+host)))
                            // JPEG compatibility starts immediately. AVC starts
                            // only after authenticated viewer codec negotiation.
                            if(avc.isEmpty()) startVideo(webSocket,key)
                        }
                    } catch (failure: Throwable) {
                        error.compareAndSet(null,failure)
                        // Server-side RealWebSocket has no client Call to cancel.
                        webSocket.close(1011, "LAN fixture handshake failed")
                    }
                }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    webSocket.close(1000,null)
                }
            }))
            // MockWebServer.start(port) defaults to loopback; the client uses
            // this literal private IPv4, so bind the same interface explicitly.
            server.start(InetAddress.getByName(address), LanEndpoint.PORT)
        }
        fun interrupt() {
            check(error.get() == null) { "LAN fixture authentication failed" }
            worker.shutdownNow()
            check(requireNotNull(peer.get()).close(1000,"CI LAN interruption"))
        }
        override fun close() {
            worker.shutdownNow()
            // The server-side socket has no OkHttp client Call. cancel()
            // dereferences that absent Call; use a close handshake and let
            // MockWebServer own the underlying TCP socket shutdown.
            try {
                peer.get()?.close(1000, null)
            } finally {
                server.shutdown()
            }
        }
    }

    // A SurfaceTexture is a producer/consumer queue. Merely registering a
    // frame callback leaves buffers unconsumed. Latch actual decoder output
    // on an EGL-owned thread, independently of the engine/UI thread.
    private class AvcSurfaceConsumer : AutoCloseable {
        private val thread = HandlerThread("CI-AVC-consumer").apply { start() }
        private val handler = Handler(thread.looper)
        private val consumed = AtomicLong()
        private val failures = AtomicLong()
        private var display = EGL14.EGL_NO_DISPLAY
        private var context = EGL14.EGL_NO_CONTEXT
        private var pbuffer = EGL14.EGL_NO_SURFACE
        private var texture: SurfaceTexture? = null
        val frames: Long get() = consumed.get()
        val errors: Long get() = failures.get()
        private fun <T> onThread(block: () -> T): T {
            val task = FutureTask<T> { block() }
            check(handler.post(task))
            return task.get(10,TimeUnit.SECONDS)
        }
        val surface: Surface = try {
            onThread {
                display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
                check(display != EGL14.EGL_NO_DISPLAY)
                val version = IntArray(2)
                check(EGL14.eglInitialize(display,version,0,version,1))
                val configs = arrayOfNulls<android.opengl.EGLConfig>(1)
                val count = IntArray(1)
                check(EGL14.eglChooseConfig(display,intArrayOf(
                    EGL14.EGL_RENDERABLE_TYPE,EGL14.EGL_OPENGL_ES2_BIT,
                    EGL14.EGL_SURFACE_TYPE,EGL14.EGL_PBUFFER_BIT,
                    EGL14.EGL_RED_SIZE,8,EGL14.EGL_GREEN_SIZE,8,EGL14.EGL_BLUE_SIZE,8,
                    EGL14.EGL_NONE),0,configs,0,1,count,0) && count[0]>0)
                val config = requireNotNull(configs[0])
                check(EGL14.eglBindAPI(EGL14.EGL_OPENGL_ES_API))
                context = EGL14.eglCreateContext(display,config,EGL14.EGL_NO_CONTEXT,
                    intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION,2,EGL14.EGL_NONE),0)
                check(context != EGL14.EGL_NO_CONTEXT)
                pbuffer = EGL14.eglCreatePbufferSurface(display,config,
                    intArrayOf(EGL14.EGL_WIDTH,1,EGL14.EGL_HEIGHT,1,EGL14.EGL_NONE),0)
                check(pbuffer != EGL14.EGL_NO_SURFACE)
                check(EGL14.eglMakeCurrent(display,pbuffer,pbuffer,context))
                val names = IntArray(1)
                GLES20.glGenTextures(1,names,0)
                check(names[0]!=0)
                GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,names[0])
                GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MIN_FILTER,GLES20.GL_LINEAR)
                GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MAG_FILTER,GLES20.GL_LINEAR)
                GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_S,GLES20.GL_CLAMP_TO_EDGE)
                GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_T,GLES20.GL_CLAMP_TO_EDGE)
                val target = SurfaceTexture(names[0])
                texture = target
                target.setDefaultBufferSize(320,240)
                target.setOnFrameAvailableListener({
                    try {
                        target.updateTexImage()
                        consumed.incrementAndGet()
                    } catch (_: Exception) { failures.incrementAndGet() }
                },handler)
                Surface(target)
            }
        } catch (error: Throwable) {
            close()
            throw error
        }
        override fun close() {
            try {
                onThread {
                    texture?.setOnFrameAvailableListener(null)
                    texture?.release()
                    texture = null
                    if (display != EGL14.EGL_NO_DISPLAY) {
                        EGL14.eglMakeCurrent(display,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_CONTEXT)
                        if(pbuffer != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display,pbuffer)
                        if(context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display,context)
                        EGL14.eglTerminate(display)
                        EGL14.eglReleaseThread()
                    }
                }
            } finally {
                thread.quitSafely()
                thread.join(5000)
            }
        }
    }

    @Test fun realPublicRelayPairingVideoInputAndRecovery() {
        val ctx = inst.targetContext
        val configFile = File(ctx.filesDir,"ci-private.json")
        val cfg = JSONObject(configFile.readText())
        configFile.delete()
        require(cfg.getString("server") == "wss://p2ptest.aikissgo.com")
        val out = File(ctx.filesDir,"e2e").apply { deleteRecursively(); mkdirs() }
        val report = JSONObject().put("scope","actual AndroidRemoteEngine and native Windows Host via public WSS; LAN peer is an emulator-local encrypted fixture (received JPEG and FFmpeg AVC test pattern), not a physical Windows LAN host")
        val metrics = JSONObject()
        val steps = org.json.JSONArray()
        val lastState = AtomicReference("")
        val lastJpeg = AtomicReference<ByteArray>()
        val frames = AtomicLong()
        var lastRecord = 0L
        var recorded = 0
        var stage = "initialize"
        var engine: AndroidRemoteEngine? = null
        var avcConsumer: AvcSurfaceConsumer? = null
        fun success(name: String) { steps.put(name) }
        fun invoke(method: String, args: Map<String,Any?> = emptyMap()) =
            main { engine!!.invoke(method,args) }
        fun snap(): Map<*,*> = invoke("snapshot")["snapshot"] as Map<*,*>
        fun command(method: String, args: Map<String,Any?> = emptyMap()) {
            assertEquals("Native command failed: "+method,true,invoke(method,args)["ok"])
        }
        fun phase(name: String) { File(out,"phase.txt").writeText(name) }
        fun frame(name: String) {
            await("authenticated video "+name) { lastState.get()==name && snap()["viewer_connected"]==true }
            lastJpeg.get()?.let { File(out,name+".jpg").writeBytes(it) }
        }
        // Each sample uses elapsed realtime and counter deltas, not configured
        // FPS or target bitrate. RTT observations may repeat between pings.
        fun sampleTransport(name: String, expected: String, lan: Boolean) {
            await("$name authenticated RTT",20_000) {
                val s = snap()
                s["transport"] == expected && s["viewer_connected"] == true && s["rtt_ms"] is Number
            }
            val prefix = if(lan) "viewer_lan" else "viewer_relay"
            val other = if(lan) "viewer_relay" else "viewer_lan"
            fun count(s: Map<*,*>, key: String) = (s[key] as Number).toLong()
            val first = snap()
            val startFrames = frames.get()
            val startMs = SystemClock.elapsedRealtime()
            val rtts = mutableListOf<Long>()
            var last = first
            repeat(20) {
                Thread.sleep(500)
                last = snap()
                assertEquals(expected,last["transport"])
                assertEquals(true,last["viewer_connected"])
                assertEquals(count(first,"relay_socket_opens"),count(last,"relay_socket_opens"))
                (last["rtt_ms"] as? Number)?.let { rtts.add(it.toLong()) }
            }
            val elapsed = SystemClock.elapsedRealtime()-startMs
            val decoded = frames.get()-startFrames
            val bytes = count(last,"${prefix}_video_bytes")-count(first,"${prefix}_video_bytes")
            val packets = count(last,"${prefix}_video_frames")-count(first,"${prefix}_video_frames")
            val otherBytes = count(last,"${other}_video_bytes")-count(first,"${other}_video_bytes")
            assertTrue(decoded>0 && bytes>0 && packets>0 && rtts.isNotEmpty())
            assertEquals(0L,otherBytes)
            metrics.put(name,JSONObject()
                .put("elapsed_ms",elapsed).put("decoded_jpeg_frames",decoded)
                .put("decoded_jpeg_fps",decoded*1000.0/elapsed)
                .put("authenticated_video_packets",packets)
                .put("encrypted_video_bytes",bytes).put("encrypted_video_bps",bytes*8000.0/elapsed)
                .put("other_transport_video_bytes",otherBytes)
                .put("rtt_observation_min_ms",rtts.minOrNull())
                .put("rtt_observation_max_ms",rtts.maxOrNull())
                .put("rtt_observations",rtts.size))
        }
        fun click() {
            command("sendPointer",mapOf("kind" to "down","x" to .5,"y" to .5))
            command("sendPointer",mapOf("kind" to "up","x" to .5,"y" to .5))
        }
        try {
            engine = main {
                AndroidRemoteEngine(ctx, onEvent = { e ->
                    if (e["type"] == "frame") {
                        val bytes = e["frame"] as? ByteArray
                        if (bytes != null) {
                            val bitmap = BitmapFactory.decodeByteArray(bytes,0,bytes.size)
                            if (bitmap != null) {
                                val name = state(bitmap)
                                bitmap.recycle()
                                if (name != null) {
                                    lastState.set(name)
                                    lastJpeg.set(bytes)
                                    frames.incrementAndGet()
                                    val now = SystemClock.elapsedRealtime()
                                    if (recorded < 450 && now-lastRecord >= 180) {
                                        File(out,String.format(java.util.Locale.US,"frame-%05d.jpg",recorded++)).writeBytes(bytes)
                                        lastRecord=now
                                    }
                                }
                            }
                        }
                    }
                }, onVideoSize = { _,_ -> })
            }
            stage="system_network_observer"
            await("Android actual default network callback",15_000) { snap()["network_observer_ready"]==true }
            assertEquals(0L,(snap()["network_change_events"] as Number).toLong())
            metrics.put("system_network_observer_registered",true)
            success("Actual Android default-network callback received before connection")
            stage="configuration"
            assertEquals(false,invoke("sendKey",mapOf("key" to "enter"))["ok"])
            command("configureRelay",mapOf("server" to cfg.getString("server"),"token" to cfg.getString("token")))
            assertEquals(true,snap()["token_configured"])
            success("Keystore configuration and disconnected command rejection")
            stage="pairing"
            val remaining=cfg.getLong("expires_at")-System.currentTimeMillis()/1000L
            metrics.put("pairing_remaining_seconds_at_submit",remaining)
            assertTrue("Pairing offer expired or too short before submission",remaining>=50)
            command("requestPairing",mapOf("deviceNumber" to cfg.getString("number"),"pairingCode" to cfg.getString("code")))
            // Only classify known status strings. Never persist an API error
            // message, device number, OTP, fingerprint or request ID.
            val pairingSamples=org.json.JSONArray()
            metrics.put("pairing_samples",pairingSamples)
            var lastPhase=""
            await("authenticated initial Windows video",55_000) {
                val v=snap()
                val status=v["viewer_status"] as? String ?: ""
                val phase=when {
                    status.startsWith("正在发起") -> "starting"
                    status.startsWith("正在查询设备") -> "lookup"
                    status.startsWith("验证码已提交") -> "submitting"
                    status.startsWith("等待电脑端用户确认") -> "pending"
                    status.startsWith("电脑端已批准") || status.startsWith("配对成功") -> "approved"
                    v["viewer_requested"]==true -> "connecting"
                    v["pairing_active"]==true -> "pairing_other"
                    else -> "terminal_or_idle"
                }
                if(phase!=lastPhase) {
                    pairingSamples.put(JSONObject().put("phase",phase)
                        .put("requested",v["viewer_requested"])
                        .put("connected",v["viewer_connected"]))
                    lastPhase=phase
                }
                lastState.get()=="ready" && v["viewer_connected"]==true
            }
            frame("ready")
            assertEquals("VPS WSS 中转",snap()["transport"])
            success("Real device pairing, explicit host approval, AES-GCM JPEG over public WSS")
            stage="permissions"
            assertEquals(false,snap()["viewer_allow_input"])
            assertEquals(false,invoke("sendPointer",mapOf("kind" to "down","x" to .5,"y" to .5))["ok"])
            assertEquals(false,invoke("getClipboardText")["ok"])
            phase("enable")
            await("host input and clipboard authorization") {
                val s=snap(); s["viewer_allow_input"]==true && s["viewer_allow_clipboard"]==true
            }
            phase("input")
            success("Default-deny controls and explicit host permission")
            stage="input"
            var start = SystemClock.elapsedRealtime()
            click()
            frame("clicked")
            metrics.put("click_to_received_frame_ms",SystemClock.elapsedRealtime()-start)
            command("sendKey",mapOf("key" to "enter"))
            frame("key")
            success("Real Windows mouse and keyboard verified in Android-decoded video")
            stage="clipboard"
            val marker="P2P-CI-SYNTHETIC"
            command("setClipboardText",mapOf("text" to marker))
            Thread.sleep(500)
            command("getClipboardText")
            await("explicit clipboard round trip") { snap()["clipboard_text"]==marker }
            success("Explicit clipboard round trip")
            stage="metrics"
            val before=frames.get()
            start=SystemClock.elapsedRealtime()
            Thread.sleep(5000)
            val fps=(frames.get()-before)*1000.0/(SystemClock.elapsedRealtime()-start)
            assertTrue("No decoded frames during FPS sample",fps>0)
            metrics.put("received_jpeg_fps",fps)
            await("authenticated RTT") { snap()["rtt_ms"] is Number }
            metrics.put("authenticated_rtt_ms",snap()["rtt_ms"])
            success("Actual received FPS and authenticated RTT")
            stage="public_performance_sample"
            sampleTransport("public_live_windows","VPS WSS 中转",false)
            stage="reconnect"
            // Test-only interruption of this viewer's real OkHttp transport.
            // No production debug endpoint or server-side disconnect is added.
            val field=AndroidRemoteEngine::class.java.getDeclaredField("socket").apply { isAccessible=true }
            val old=main { field.get(engine) as WebSocket }
            lastState.set("")
            start=SystemClock.elapsedRealtime()
            main { old.cancel() }
            await("transport reconnect",45_000) {
                val current=main { field.get(engine) }
                current!=null && current!==old && snap()["viewer_connected"]==true &&
                    lastState.get()=="key" && snap()["viewer_allow_input"]==true
            }
            metrics.put("reconnect_to_verified_frame_ms",SystemClock.elapsedRealtime()-start)
            click()
            frame("recovered")
            success("Automatic Android reconnect after real WSS cancellation; input restored")

            stage="system_network_outage_preflight"
            // Mandatory in public CI, never toggle radios on a physical device.
            require(cfg.optBoolean("disposable_emulator_network_test"))
            require(Build.HARDWARE in setOf("ranchu","goldfish"))
            val connectivity = requireNotNull(ctx.getSystemService(ConnectivityManager::class.java))
            val wifiBefore = shell("settings get global wifi_on")
            val dataBefore = shell("settings get global mobile_data")
            require(wifiBefore in setOf("0","1") && dataBefore in setOf("0","1"))
            require(wifiBefore=="1" || dataBefore=="1")
            require(connectivity.activeNetwork != null)
            val beforeOutage=snap()
            val preOutageSocket=main { field.get(engine) as WebSocket }
            fun metric(s:Map<*,*>, name:String)=(s[name] as Number).toLong()
            val evidence=JSONObject().put("scope","real Android emulator Wi-Fi/mobile-data radio controls; not physical radio mobility")
            metrics.put("system_network_outage",evidence)
            val outageStarted=SystemClock.elapsedRealtime()
            var disconnectedAt=0L
            try {
                stage="system_network_radios_off"
                shell("svc wifi disable")
                shell("svc data disable")
                await("actual Android default network absent",30_000) {
                    connectivity.activeNetwork==null &&
                        metric(snap(),"network_change_events")>metric(beforeOutage,"network_change_events")
                }
                evidence.put("default_network_lost",true)
                    .put("observed_network_loss_ms",SystemClock.elapsedRealtime()-outageStarted)
                stage="system_network_native_retry"
                await("actual native connection retired after radios off",55_000) {
                    val s=snap()
                    s["viewer_requested"]==true && s["viewer_connected"]==false &&
                        s["active_video_codec"]=="none" &&
                        s["host_metadata_current"]==false &&
                        metric(s,"relay_retry_events")>metric(beforeOutage,"relay_retry_events") &&
                        main { field.get(engine) !== preOutageSocket }
                }
                val interrupted=snap()
                disconnectedAt=SystemClock.elapsedRealtime()
                evidence.put("native_connection_retired",true)
                    .put("observed_retire_ms",disconnectedAt-outageStarted)
                    .put("retry_events_during_outage",
                        metric(interrupted,"relay_retry_events")-metric(beforeOutage,"relay_retry_events"))
                assertNull(interrupted["host_relay_socket_open"])
                assertNull(interrupted["host_relay_video_bytes"])
                assertEquals(0.0,(interrupted["rendered_fps"] as Number).toDouble(),0.0)
                assertEquals(0L,metric(interrupted,"measured_video_bps"))
            } finally {
                // Restore each original setting even if the outage assertion failed.
                val dataRestored=runCatching { shell("svc data "+if(dataBefore=="1") "enable" else "disable") }
                val wifiRestored=runCatching { shell("svc wifi "+if(wifiBefore=="1") "enable" else "disable") }
                dataRestored.getOrThrow()
                wifiRestored.getOrThrow()
            }
            stage="system_network_radios_restore"
            val restoreStarted=SystemClock.elapsedRealtime()
            await("actual Android default network restored",45_000) {
                connectivity.activeNetwork!=null &&
                    metric(snap(),"network_change_events")>=metric(beforeOutage,"network_change_events")+2
            }
            evidence.put("default_network_restored",true)
                .put("observed_default_network_restore_ms",SystemClock.elapsedRealtime()-restoreStarted)
            stage="system_network_live_video_recovery"
            val beforeRestoreFrames=frames.get()
            await("new authenticated public video after actual radio restoration",60_000) {
                val s=snap()
                s["viewer_transport_phase"]=="relay_video" && s["viewer_connected"]==true &&
                    s["viewer_allow_input"]==true && s["viewer_allow_clipboard"]==true &&
                    s["host_metadata_current"]==true && s["rtt_ms"] is Number &&
                    metric(s,"relay_socket_opens")>metric(beforeOutage,"relay_socket_opens") &&
                    metric(s,"viewer_relay_video_bytes")>metric(beforeOutage,"viewer_relay_video_bytes") &&
                    main { field.get(engine)!=null && field.get(engine)!==preOutageSocket } &&
                    frames.get()>beforeRestoreFrames && lastState.get()=="recovered"
            }
            val restored=snap()
            evidence.put("new_authenticated_video",true)
                .put("observed_restore_to_video_ms",SystemClock.elapsedRealtime()-restoreStarted)
                .put("observed_retire_to_video_ms",SystemClock.elapsedRealtime()-disconnectedAt)
                .put("network_change_events",metric(restored,"network_change_events")-metric(beforeOutage,"network_change_events"))
                .put("network_retry_wakes",metric(restored,"network_retry_wakes")-metric(beforeOutage,"network_retry_wakes"))
                .put("new_relay_socket_opens",metric(restored,"relay_socket_opens")-metric(beforeOutage,"relay_socket_opens"))
            val stableFrames=frames.get()
            val stableOpens=metric(restored,"relay_socket_opens")
            Thread.sleep(3000)
            assertTrue(frames.get()>stableFrames)
            assertEquals(stableOpens,metric(snap(),"relay_socket_opens"))
            assertEquals("relay_video",snap()["viewer_transport_phase"])
            evidence.put("sustained_video",true)
            success("Actual emulator default network lost and restored; new authenticated public Windows video and permissions verified")
            stage="lan_fixture_setup"
            // Keep the actual paired root in memory only. Private reflection is
            // confined to instrumentation; no production debug API is added.
            val keyField = AndroidRemoteEngine::class.java.getDeclaredField("secret").apply { isAccessible=true }
            val root = main { (keyField.get(engine) as ByteArray).copyOf() }
            val jpeg = requireNotNull(lastJpeg.get())
            command("disconnectViewer")
            LanPeer(root,jpeg).use { lan ->
                val connect = AndroidRemoteEngine::class.java.getDeclaredMethod("connect",
                    String::class.java,String::class.java,String::class.java,String::class.java).apply { isAccessible=true }
                val hex = root.joinToString("") { "%02x".format(it.toInt() and 255) }
                lastState.set("")
                main { connect.invoke(engine,cfg.getString("server"),cfg.getString("token"),hex,lan.address) }
                stage="lan_fixture_connect"
                await("authenticated LAN v4 fixture video",20_000) {
                    val s = snap()
                    s["transport"]=="局域网 P2P 直连" && s["viewer_connected"]==true &&
                        lastState.get()=="recovered" && (s["relay_socket_opens"] as Number).toLong()==0L
                }
                val lanStats = snap()
                assertEquals("lan_video",lanStats["viewer_transport_phase"])
                assertEquals(1,(lanStats["lan_candidate_attempts"] as Number).toInt())
                assertTrue((lanStats["lan_attempt_ms"] as Number).toLong() >= 0)
                assertNull(lanStats["lan_fallback_reason"])
                assertEquals(false,lanStats["lan_fallback_started"])
                assertNull(lanStats["lan_fallback_relay_open_ms"])
                assertNull(lanStats["lan_fallback_first_video_ms"])
                assertTrue((lanStats["viewer_lan_video_bytes"] as Number).toLong() > 0)
                assertTrue((lanStats["viewer_lan_video_frames"] as Number).toLong() > 0)
                assertEquals(0L,(lanStats["viewer_relay_video_bytes"] as Number).toLong())
                metrics.put("fixture_lan_video_bytes",(lanStats["viewer_lan_video_bytes"] as Number).toLong())
                metrics.put("fixture_relay_video_bytes",0)
                success("Actual encrypted LAN socket and fixture JPEG; zero viewer relay opens and zero admitted relay video bytes")
                stage="lan_fixture_performance_sample"
                sampleTransport("lan_local_fixture","局域网 P2P 直连",true)
                assertTrue("Fixture must answer authenticated application pings",lan.authenticatedPongs>=2)
                metrics.put("fixture_authenticated_pongs",lan.authenticatedPongs)
                lastState.set("")
                start=SystemClock.elapsedRealtime()
                stage="lan_fixture_close"
                lan.interrupt()
                stage="lan_fallback_public_video"
                await("LAN close to real public WSS video",45_000) {
                    val s=snap()
                    s["transport"]=="VPS WSS 中转" && s["viewer_connected"]==true &&
                        lastState.get()=="recovered" && s["viewer_allow_input"]==true &&
                        s["viewer_allow_clipboard"]==true && (s["relay_socket_opens"] as Number).toLong()>=1L
                }
                metrics.put("lan_close_to_public_verified_frame_ms",SystemClock.elapsedRealtime()-start)
                val fallbackStats = snap()
                assertEquals("relay_video",fallbackStats["viewer_transport_phase"])
                assertEquals(1,(fallbackStats["lan_candidate_attempts"] as Number).toInt())
                assertEquals(true,fallbackStats["lan_fallback_started"])
                assertTrue(fallbackStats["lan_fallback_reason"] in setOf("peer_closed","transport_failure"))
                val openMs = (fallbackStats["lan_fallback_relay_open_ms"] as Number).toLong()
                val firstMs = (fallbackStats["lan_fallback_first_video_ms"] as Number).toLong()
                assertTrue(openMs >= 0 && firstMs >= openMs)
                assertEquals(true,fallbackStats["host_relay_socket_open"])
                assertTrue((fallbackStats["host_relay_socket_opens"] as Number).toLong() >= 1L)
                assertTrue((fallbackStats["host_relay_video_frames"] as Number).toLong() >= 0L)
                val diag = JSONObject()
                    .put("lan_attempt_ms",fallbackStats["lan_attempt_ms"])
                    .put("reason",fallbackStats["lan_fallback_reason"])
                    .put("relay_open_ms",openMs).put("first_video_ms",firstMs)
                    .put("host_relay_socket_opens",fallbackStats["host_relay_socket_opens"])
                metrics.put("jpeg_fallback_diagnostics",diag)
                success("Actual native fallback diagnostics: " + diag.toString())
                assertTrue(firstMs <= SystemClock.elapsedRealtime()-start)
                assertTrue((fallbackStats["viewer_relay_video_bytes"] as Number).toLong() > 0)
                metrics.put("fallback_relay_video_bytes",(fallbackStats["viewer_relay_video_bytes"] as Number).toLong())
                success("LAN fixture close automatically falls back to real Windows WSS video")
                stage="lan_fixture_cleanup"
            }
            stage="lan_fallback_input"
            command("sendKey",mapOf("key" to "enter"))
            frame("key")
            click()
            frame("recovered")
            val fallbackMarker="P2P-CI-LAN-FALLBACK"
            command("setClipboardText",mapOf("text" to fallbackMarker))
            Thread.sleep(500)
            command("getClipboardText")
            await("clipboard after LAN fallback") { snap()["clipboard_text"]==fallbackMarker }
            success("Real Windows keyboard, click and clipboard restored after LAN fallback")
            stage="fallback_performance_sample"
            sampleTransport("fallback_live_windows","VPS WSS 中转",false)
            stage="multi_candidate_network_setup"
            // The disposable emulator runner installs two /32 test addresses
            // on loopback and a matching /24 on the active interface. The
            // production auto-discovery excludes own non-loopback addresses.
            val activeNet=connectivity.activeNetwork
            val nic=requireNotNull(connectivity.getLinkProperties(activeNet)?.interfaceName)
            File(out,"candidate-interface.txt").writeText(nic)
            phase("multi_candidate_setup")
            await("runner installed isolated test addresses",30_000) {
                File(out,"candidate-ready.txt").exists()
            }
            val firstAddress="10.203.71.2"
            val secondAddress="10.203.71.3"
            stage="multi_candidate_ranking"
            assertEquals(listOf(firstAddress,secondAddress),
                LanEndpoint.eligibleAdvertised(listOf(firstAddress,firstAddress,secondAddress,"8.8.8.8","10.203.71.1")))
            val multiEvidence=JSONObject().put("scope","two independent emulator-local IPv4 WebSocket listeners; candidate list injected at authenticated metadata handler boundary")
            metrics.put("multi_candidate_lan",multiEvidence)
            try {
                LanPeer(root,jpeg,stallHandshake=true,bindAddress=firstAddress).use { first ->
                    LanPeer(root,jpeg,bindAddress=secondAddress).use { second ->
                        stage="multi_candidate_start"
                        val oldRelay=main { field.get(engine) as WebSocket }
                        val baseline=snap()
                        assertEquals("relay_video",baseline["viewer_transport_phase"])
                        // This session already used a manual LAN address. Start
                        // a fresh requested session on WSS with the paired root.
                        command("disconnectViewer")
                        val connect=AndroidRemoteEngine::class.java.getDeclaredMethod("connect",
                            String::class.java,String::class.java,String::class.java,String::class.java).apply { isAccessible=true }
                        val hex=root.joinToString("") { "%02x".format(it.toInt() and 255) }
                        main { connect.invoke(engine,cfg.getString("server"),cfg.getString("token"),hex,"") }
                        await("fresh actual public video before advertised upgrade",45_000) {
                            val s=snap()
                            s["viewer_transport_phase"]=="relay_video" && s["viewer_connected"]==true &&
                                s["host_metadata_current"]==true && s["viewer_allow_input"]==true &&
                                (s["lan_candidate_attempts"] as Number).toInt()==0 &&
                                main { field.get(engine)!=null && field.get(engine)!==oldRelay }
                        }
                        val relay=main { field.get(engine) as WebSocket }
                        val prior=snap()
                        val upgrade=AndroidRemoteEngine::class.java.getDeclaredMethod("tryAdvertisedLan",
                            WebSocket::class.java,List::class.java).apply { isAccessible=true }
                        val started=SystemClock.elapsedRealtime()
                        // Only the metadata source is injected; ranking, socket
                        // creation, encryption, timeout and retry are production.
                        main { upgrade.invoke(engine,relay,listOf(firstAddress,secondAddress)) }
                        stage="multi_candidate_first_stalled"
                        await("first real candidate authenticated HELLO",8_000) { first.authenticatedHellos==1L }
                        stage="multi_candidate_second_verified"
                        await("second real candidate authenticated video",15_000) {
                            val s=snap()
                            s["viewer_transport_phase"]=="lan_video" &&
                                (s["lan_candidate_attempts"] as Number).toInt()==2 &&
                                second.authenticatedHellos==1L && second.authenticatedPongs>0 &&
                                (s["viewer_lan_video_bytes"] as Number).toLong()>0
                        }
                        val accepted=snap()
                        assertEquals(0L,first.sentVideoPackets)
                        assertNull(accepted["lan_fallback_reason"])
                        assertEquals(false,accepted["lan_fallback_started"])
                        assertEquals(prior["relay_socket_opens"],accepted["relay_socket_opens"])
                        assertTrue((accepted["lan_attempt_ms"] as Number).toLong()>=0)
                        multiEvidence.put("first_authenticated_hello",true)
                            .put("first_video_packets",first.sentVideoPackets)
                            .put("second_authenticated_hello",true)
                            .put("second_authenticated_pongs",second.authenticatedPongs)
                            .put("candidate_attempts",accepted["lan_candidate_attempts"])
                            .put("attempt_ms",accepted["lan_attempt_ms"])
                            .put("observed_upgrade_to_video_ms",SystemClock.elapsedRealtime()-started)
                            .put("relay_socket_opens_during_batch",0)
                        Thread.sleep(3000)
                        val stable=snap()
                        assertEquals("lan_video",stable["viewer_transport_phase"])
                        assertEquals(2,(stable["lan_candidate_attempts"] as Number).toInt())
                        assertEquals(accepted["viewer_relay_video_bytes"],stable["viewer_relay_video_bytes"])
                        assertTrue((stable["viewer_lan_video_bytes"] as Number).toLong()>
                            (accepted["viewer_lan_video_bytes"] as Number).toLong())
                        multiEvidence.put("stable_second_candidate_video",true)
                        stage="multi_candidate_verified_disconnect"
                        second.interrupt()
                        await("verified second candidate closes to real public WSS",45_000) {
                            val s=snap()
                            s["viewer_transport_phase"]=="relay_video" && s["viewer_allow_input"]==true &&
                                s["lan_fallback_first_video_ms"] is Number
                        }
                        val recovered=snap()
                        assertEquals(2,(recovered["lan_candidate_attempts"] as Number).toInt())
                        assertEquals(1L,first.authenticatedHellos)
                        assertEquals(1L,second.authenticatedHellos)
                        assertTrue((recovered["relay_socket_opens"] as Number).toLong()>
                            (accepted["relay_socket_opens"] as Number).toLong())
                        multiEvidence.put("verified_candidate_disconnect_recovers_public",true)
                            .put("fallback_first_video_ms",recovered["lan_fallback_first_video_ms"])
                        success("First actual candidate stalled authentication; second authenticated and carried video without intermediate WSS; verified close recovered public Windows")
                        command("disconnectViewer")
                    }
                }
            } finally {
                phase("multi_candidate_cleanup")
                await("runner removed isolated test addresses",30_000) {
                    File(out,"candidate-cleaned.txt").exists()
                }
            }
            // Independent AVC session, after every live Windows/JPEG assertion.
            // This is an encoded test pattern, not a physical LAN host capture.
            stage="lan_avc_setup"
            fun loadAvcFixture(name: String, width: Int, height: Int): List<ByteArray> {
                val file=File(ctx.filesDir,name)
                val fixture=ByteBuffer.wrap(file.readBytes())
                file.delete()
                val units=mutableListOf<ByteArray>()
                while(fixture.hasRemaining()) {
                    val size=fixture.int
                    require(size in 9..fixture.remaining())
                    val unit=ByteArray(size)
                    fixture.get(unit)
                    val parsed=requireNotNull(AvcPayload.parse(unit))
                    assertEquals(width,parsed.width)
                    assertEquals(height,parsed.height)
                    units.add(unit)
                }
                assertEquals(20,units.size)
                assertTrue(requireNotNull(AvcPayload.parse(units[0])).hasSelfContainedIdr())
                assertTrue(requireNotNull(AvcPayload.parse(units[10])).hasSelfContainedIdr())
                return units
            }
            val units=loadAvcFixture("ci-lan-avc.bin",320,240)
            val alternate=loadAvcFixture("ci-lan-avc-alt.bin",640,360)
            command("disconnectViewer")
            val consumer=AvcSurfaceConsumer()
            avcConsumer=consumer
            main { engine!!.attachVideoSurface(consumer.surface) }
            LanPeer(root,jpeg,units).use { lan ->
                val connect=AndroidRemoteEngine::class.java.getDeclaredMethod("connect",
                    String::class.java,String::class.java,String::class.java,String::class.java).apply { isAccessible=true }
                val hex=root.joinToString("") { "%02x".format(it.toInt() and 255) }
                val beforeAvc=(snap()["avc_rendered_frames"] as Number).toLong()
                main { connect.invoke(engine,cfg.getString("server"),cfg.getString("token"),hex,lan.address) }
                stage="lan_avc_decode"
                val startup=org.json.JSONArray()
                metrics.put("lan_avc_startup_samples",startup)
                val startupAt=SystemClock.elapsedRealtime()
                var lastStartup=0L
                await("real MediaCodec LAN surface output",30_000) {
                    val v=snap()
                    val now=SystemClock.elapsedRealtime()
                    if(now-lastStartup>=500) {
                        startup.put(JSONObject()
                            .put("elapsed_ms",now-startupAt)
                            .put("transport",v["transport"])
                            .put("connected",v["viewer_connected"])
                            .put("h264_active",v["h264_active"])
                            .put("width",v["frame_width"]).put("height",v["frame_height"])
                            .put("rendered",v["avc_rendered_frames"])
                            .put("avc_received_bytes",v["avc_received_bytes"])
                            .put("surface_consumed",consumer.frames)
                            .put("surface_errors",consumer.errors)
                            .put("decoder",v["avc_decoder"])
                            .put("decoder_drops",v["avc_decoder_drops"])
                            .put("lan_video_bytes",v["viewer_lan_video_bytes"])
                            .put("relay_opens",v["relay_socket_opens"])
                            .put("rtt_ms",v["rtt_ms"] ?: JSONObject.NULL)
                            .put("fixture_sent",lan.sentVideoPackets)
                            .put("fixture_codec_requests",lan.authenticatedCodecRequests)
                            .put("fixture_pongs",lan.authenticatedPongs)
                            .put("fixture_error",lan.failureType ?: JSONObject.NULL))
                        lastStartup=now
                    }
                    v["transport"]=="局域网 P2P 直连" && v["h264_active"]==true &&
                        v["viewer_connected"]==true && (v["avc_rendered_frames"] as Number).toLong()>beforeAvc &&
                        consumer.frames>0 && v["rtt_ms"] is Number &&
                        lan.authenticatedCodecRequests>0 && lan.failureType==null
                }
                val initial=snap()
                val startSurface=consumer.frames
                val started=SystemClock.elapsedRealtime()
                val samples=org.json.JSONArray()
                metrics.put("lan_avc_samples",samples)
                fun sample(v:Map<*,*>) {
                    samples.put(JSONObject()
                        .put("elapsed_ms",SystemClock.elapsedRealtime()-started)
                        .put("transport",v["transport"])
                        .put("connected",v["viewer_connected"])
                        .put("h264_active",v["h264_active"])
                        .put("rendered",v["avc_rendered_frames"])
                        .put("surface_consumed",consumer.frames)
                        .put("surface_errors",consumer.errors)
                        .put("decoder_drops",v["avc_decoder_drops"])
                        .put("lan_video_bytes",v["viewer_lan_video_bytes"])
                        .put("relay_video_bytes",v["viewer_relay_video_bytes"])
                        .put("relay_opens",v["relay_socket_opens"])
                        .put("authenticated_pongs",lan.authenticatedPongs)
                        .put("rtt_ms",v["rtt_ms"]))
                }
                sample(initial)
                stage="lan_avc_performance"
                repeat(20) { index ->
                    Thread.sleep(500)
                    val v=snap()
                    sample(v)
                    stage="lan_avc_sample_${index+1}_transport"
                    assertEquals("局域网 P2P 直连",v["transport"])
                    stage="lan_avc_sample_${index+1}_codec"
                    assertEquals(true,v["h264_active"])
                    stage="lan_avc_sample_${index+1}_connected"
                    assertEquals(true,v["viewer_connected"])
                    stage="lan_avc_sample_${index+1}_relay"
                    assertEquals(0L,(v["relay_socket_opens"] as Number).toLong())
                    assertEquals(0L,(v["viewer_relay_video_bytes"] as Number).toLong())
                }
                val last=snap()
                val elapsed=SystemClock.elapsedRealtime()-started
                fun delta(key:String)=(last[key] as Number).toLong()-(initial[key] as Number).toLong()
                val rendered=delta("avc_rendered_frames")
                val bytes=delta("viewer_lan_video_bytes")
                val callbacks=consumer.frames-startSurface
                stage="lan_avc_render_progress"
                assertTrue(rendered>0)
                stage="lan_avc_byte_progress"
                assertTrue(bytes>0)
                stage="lan_avc_surface_progress"
                assertTrue(callbacks>0 && consumer.errors==0L)
                stage="lan_avc_ping_progress"
                assertTrue(lan.authenticatedPongs>=2)
                stage="lan_avc_decoder_identity"
                assertTrue((last["avc_decoder"] as String).isNotBlank())
                metrics.put("lan_avc_fixture",JSONObject()
                    .put("elapsed_ms",elapsed).put("rendered_avc_frames",rendered)
                    .put("rendered_fps",rendered*1000.0/elapsed)
                    .put("encrypted_video_bytes",bytes).put("encrypted_video_bps",bytes*8000.0/elapsed)
                    .put("surface_consumed_frames",callbacks).put("surface_errors",consumer.errors)
                    .put("authenticated_rtt_ms",last["rtt_ms"])
                    .put("decoder",last["avc_decoder"]).put("decoder_hardware",last["avc_decoder_hardware"] ?: JSONObject.NULL)
                    .put("relay_video_bytes",0).put("relay_socket_opens",0)
                    .put("source","FFmpeg synthetic test pattern; emulator-local LAN, not physical Windows capture"))
                success("Real MediaCodec AVC surface output via encrypted LAN fixture, no viewer relay opens or video")
                // Keep the same authenticated LAN WebSocket, engine and EGL
                // consumer. Deliberately withhold the new format's keyframe
                // before supplying its real, independently encoded closed GOP.
                val switches=org.json.JSONArray()
                metrics.put("lan_avc_resolution_switches",switches)
                for ((width,height,nextUnits) in listOf(
                    Triple(640,360,alternate),Triple(320,240,units))) {
                    stage="lan_avc_resolution_${width}_predictive_only"
                    val before=snap()
                    val helloCount=lan.authenticatedHellos
                    val applied=lan.appliedVideoSwitches
                    val sent=lan.sentVideoPackets
                    val predictive=nextUnits.subList(1,10)
                    assertTrue(predictive.all {
                        !requireNotNull(AvcPayload.parse(it)).hasSelfContainedIdr()
                    })
                    lan.switchAvc(predictive)
                    fun matchesSize(v:Map<*,*>) =
                        (v["frame_width"] as Number).toInt()==width &&
                        (v["frame_height"] as Number).toInt()==height
                    await("new resolution waiting for IDR",8_000) {
                        val v=snap()
                        matchesSize(v) && v["h264_active"]==true &&
                            v["viewer_connected"]==false &&
                            lan.appliedVideoSwitches==applied+1
                    }
                    val blocked=snap()
                    val blockedRendered=(blocked["avc_rendered_frames"] as Number).toLong()
                    val blockedSent=lan.sentVideoPackets
                    // At least five actual predictive-only packets after
                    // observing the new-format barrier, with no new output.
                    await("predictive-only packets sent",5_000) {
                        lan.sentVideoPackets>=blockedSent+5
                    }
                    val stillBlocked=snap()
                    assertTrue(matchesSize(stillBlocked))
                    assertEquals(false,stillBlocked["viewer_connected"])
                    val blockedDelta=(stillBlocked["avc_rendered_frames"] as Number).toLong()-blockedRendered
                    assertEquals(0L,blockedDelta)
                    val blockedPackets=lan.sentVideoPackets-blockedSent
                    assertTrue(lan.sentVideoPackets>sent)
                    assertEquals(helloCount,lan.authenticatedHellos)
                    assertEquals(0L,(stillBlocked["relay_socket_opens"] as Number).toLong())
                    assertEquals(0L,(stillBlocked["viewer_relay_video_bytes"] as Number).toLong())
                    stage="lan_avc_resolution_${width}_recovery"
                    val resumeSurface=consumer.frames
                    val resumeAt=SystemClock.elapsedRealtime()
                    lan.switchAvc(nextUnits)
                    await("actual new-format MediaCodec output",15_000) {
                        val v=snap()
                        matchesSize(v) && v["viewer_connected"]==true &&
                            v["transport"]=="局域网 P2P 直连" &&
                            (v["avc_rendered_frames"] as Number).toLong()>=blockedRendered+3 &&
                            consumer.frames>resumeSurface &&
                            lan.appliedVideoSwitches==applied+2
                    }
                    val recoveredFormat=snap()
                    assertEquals(helloCount,lan.authenticatedHellos)
                    assertEquals(before["relay_socket_opens"],recoveredFormat["relay_socket_opens"])
                    assertEquals(0L,(recoveredFormat["viewer_relay_video_bytes"] as Number).toLong())
                    assertEquals(0L,consumer.errors)
                    assertNull(lan.failureType)
                    assertTrue((recoveredFormat["viewer_lan_video_bytes"] as Number).toLong()>
                        (before["viewer_lan_video_bytes"] as Number).toLong())
                    val evidence=JSONObject()
                        .put("width",width).put("height",height)
                        .put("predictive_only_packets",blockedPackets)
                        .put("predictive_only_render_delta",blockedDelta)
                        .put("predictive_only_connected",stillBlocked["viewer_connected"])
                        .put("render_delta",(recoveredFormat["avc_rendered_frames"] as Number).toLong()-blockedRendered)
                        .put("surface_delta",consumer.frames-resumeSurface)
                        .put("recovery_ms",SystemClock.elapsedRealtime()-resumeAt)
                        .put("same_socket",lan.authenticatedHellos==helloCount)
                        .put("relay_socket_opens",recoveredFormat["relay_socket_opens"])
                        .put("relay_video_bytes",recoveredFormat["viewer_relay_video_bytes"])
                        .put("surface_errors",consumer.errors)
                    switches.put(evidence)
                    success("Actual MediaCodec resolution recovery on same encrypted LAN: "+evidence.toString())
                }
                val beforeFallback = snap()
                val fallbackStart = SystemClock.elapsedRealtime()
                val beforeSurface = consumer.frames
                stage="lan_avc_interrupt"
                lan.interrupt()
                stage="lan_avc_fallback"
                // Keep the same engine, key and Surface. Only the fixture
                // transport closes; the real Windows host negotiates AVC on WSS.
                val encoders = setOf("OpenH264 software","Media Foundation hardware","Media Foundation D3D11 texture")
                await("live Windows AVC after LAN interruption",45_000) {
                    val v=snap()
                    v["transport"]=="VPS WSS 中转" && v["viewer_connected"]==true &&
                        v["h264_active"]==true && v["viewer_allow_input"]==true &&
                        v["viewer_allow_clipboard"]==true &&
                        v["video_encoder"] in encoders &&
                        ((v["frame_width"] as Number).toInt()!=320 || (v["frame_height"] as Number).toInt()!=240) &&
                        (v["relay_socket_opens"] as Number).toLong()>=1L &&
                        (v["viewer_relay_video_bytes"] as Number).toLong()>0L &&
                        (v["avc_rendered_frames"] as Number).toLong()>(beforeFallback["avc_rendered_frames"] as Number).toLong() &&
                        consumer.frames>beforeSurface && v["rtt_ms"] is Number &&
                        (v["host_encoded_frames"] as Number).toLong()>0
                }
                metrics.put("lan_avc_close_to_live_wss_avc_ms",SystemClock.elapsedRealtime()-fallbackStart)
                val recovered=snap()
                assertEquals("relay_video",recovered["viewer_transport_phase"])
                assertEquals(true,recovered["lan_fallback_started"])
                assertTrue(recovered["lan_fallback_reason"] in setOf("peer_closed","transport_failure"))
                val openMs = (recovered["lan_fallback_relay_open_ms"] as Number).toLong()
                val firstMs = (recovered["lan_fallback_first_video_ms"] as Number).toLong()
                assertTrue(openMs >= 0 && firstMs >= openMs)
                assertEquals(true,recovered["host_relay_socket_open"])
                assertTrue((recovered["host_relay_socket_opens"] as Number).toLong() >= 1L)
                assertTrue((recovered["host_relay_video_frames"] as Number).toLong() >= 0L)
                val diag = JSONObject()
                    .put("lan_attempt_ms",recovered["lan_attempt_ms"])
                    .put("reason",recovered["lan_fallback_reason"])
                    .put("relay_open_ms",openMs).put("first_video_ms",firstMs)
                    .put("host_relay_socket_opens",recovered["host_relay_socket_opens"])
                metrics.put("avc_fallback_diagnostics",diag)
                success("Actual native fallback diagnostics: " + diag.toString())
                assertTrue(firstMs <= SystemClock.elapsedRealtime()-fallbackStart)
                val recoveredSurface=consumer.frames
                val recoveredAt=SystemClock.elapsedRealtime()
                val fallbackSamples=org.json.JSONArray()
                metrics.put("lan_avc_fallback_samples",fallbackSamples)
                stage="lan_avc_fallback_stability"
                repeat(20) {
                    Thread.sleep(500)
                    val v=snap()
                    fallbackSamples.put(JSONObject()
                        .put("elapsed_ms",SystemClock.elapsedRealtime()-recoveredAt)
                        .put("transport",v["transport"]).put("h264_active",v["h264_active"])
                        .put("connected",v["viewer_connected"])
                        .put("relay_opens",v["relay_socket_opens"])
                        .put("relay_bytes",v["viewer_relay_video_bytes"])
                        .put("lan_bytes",v["viewer_lan_video_bytes"])
                        .put("rendered",v["avc_rendered_frames"])
                        .put("surface_consumed",consumer.frames)
                        .put("host_encoded",v["host_encoded_frames"]))
                    assertEquals("VPS WSS 中转",v["transport"])
                    assertEquals(true,v["h264_active"])
                    assertEquals(true,v["viewer_connected"])
                    assertEquals(recovered["relay_socket_opens"],v["relay_socket_opens"])
                    assertEquals(recovered["viewer_lan_video_bytes"],v["viewer_lan_video_bytes"])
                    assertEquals(recovered["lan_fallback_reason"],v["lan_fallback_reason"])
                    assertEquals(recovered["lan_fallback_relay_open_ms"],v["lan_fallback_relay_open_ms"])
                    assertEquals(recovered["lan_fallback_first_video_ms"],v["lan_fallback_first_video_ms"])
                }
                val end=snap()
                fun fallbackDelta(k:String)=(end[k] as Number).toLong()-(recovered[k] as Number).toLong()
                val fallbackMs=SystemClock.elapsedRealtime()-recoveredAt
                val fallbackFrames=fallbackDelta("avc_rendered_frames")
                val fallbackBytes=fallbackDelta("viewer_relay_video_bytes")
                assertTrue(fallbackFrames>0 && fallbackBytes>0 && consumer.frames>recoveredSurface)
                assertTrue(fallbackDelta("host_encoded_frames")>0 && consumer.errors==0L)
                assertTrue(end["video_encoder"] in encoders && end["rtt_ms"] is Number)
                metrics.put("lan_avc_fallback_live_windows",JSONObject()
                    .put("elapsed_ms",fallbackMs)
                    .put("rendered_avc_frames",fallbackFrames)
                    .put("rendered_fps",fallbackFrames*1000.0/fallbackMs)
                    .put("encrypted_video_bytes",fallbackBytes)
                    .put("encrypted_video_bps",fallbackBytes*8000.0/fallbackMs)
                    .put("lan_video_byte_delta",fallbackDelta("viewer_lan_video_bytes"))
                    .put("surface_consumed_frames",consumer.frames-recoveredSurface)
                    .put("host_encoded_frame_delta",fallbackDelta("host_encoded_frames"))
                    .put("width",end["frame_width"]).put("height",end["frame_height"])
                    .put("encoder",end["video_encoder"]).put("decoder",end["avc_decoder"])
                    .put("authenticated_rtt_ms",end["rtt_ms"]))
                stage="lan_avc_fallback_clipboard"
                val avcMarker="P2P-CI-AVC-FALLBACK"
                command("setClipboardText",mapOf("text" to avcMarker))
                Thread.sleep(500)
                command("getClipboardText")
                await("clipboard after AVC fallback") { snap()["clipboard_text"]==avcMarker }
                success("LAN AVC interruption recovered live Windows WSS AVC on same Surface; actual clipboard round trip")
                stage="lan_avc_disconnect"
                command("disconnectViewer")
            }
            stage="lan_verification_timeout"
            LanPeer(root,jpeg,stallHandshake=true).use { stalled ->
                val connect = AndroidRemoteEngine::class.java.getDeclaredMethod("connect",
                    String::class.java,String::class.java,String::class.java,String::class.java).apply { isAccessible=true }
                val hex = root.joinToString("") { "%02x".format(it.toInt() and 255) }
                val started = SystemClock.elapsedRealtime()
                main { connect.invoke(engine,cfg.getString("server"),cfg.getString("token"),hex,stalled.address) }
                await("actual LAN hello without challenge",8_000) { stalled.authenticatedHellos == 1L }
                await("stalled LAN verification to actual public video",45_000) {
                    val s=snap()
                    s["viewer_transport_phase"]=="relay_video" && s["viewer_connected"]==true &&
                        s["viewer_allow_input"]==true && s["viewer_allow_clipboard"]==true &&
                        s["lan_fallback_first_video_ms"] is Number
                }
                val s=snap()
                assertEquals("connect_or_verify_timeout",s["lan_fallback_reason"])
                assertEquals(1,(s["lan_candidate_attempts"] as Number).toInt())
                assertEquals(0L,(s["viewer_lan_video_bytes"] as Number).toLong())
                assertTrue((s["viewer_relay_video_bytes"] as Number).toLong()>0)
                assertEquals(0L,stalled.sentVideoPackets)
                val attemptMs=(s["lan_attempt_ms"] as Number).toLong()
                val openMs=(s["lan_fallback_relay_open_ms"] as Number).toLong()
                val videoMs=(s["lan_fallback_first_video_ms"] as Number).toLong()
                assertTrue(attemptMs>=0 && openMs>=0 && videoMs>=openMs)
                metrics.put("lan_stalled_verification",JSONObject()
                    .put("authenticated_hello_count",stalled.authenticatedHellos)
                    .put("candidate_attempts",s["lan_candidate_attempts"])
                    .put("attempt_ms",attemptMs).put("relay_open_ms",openMs)
                    .put("first_video_ms",videoMs)
                    .put("observed_total_ms",SystemClock.elapsedRealtime()-started))
                success("Real LAN socket with authenticated hello but no challenge timed out and recovered real public Windows video")
                command("disconnectViewer")
            }
            stage="lan_authenticated_heartbeat_timeout"
            LanPeer(root,jpeg).use { silent ->
                val connect = AndroidRemoteEngine::class.java.getDeclaredMethod("connect",
                    String::class.java,String::class.java,String::class.java,String::class.java).apply { isAccessible=true }
                val hex = root.joinToString("") { "%02x".format(it.toInt() and 255) }
                main { connect.invoke(engine,cfg.getString("server"),cfg.getString("token"),hex,silent.address) }
                await("actual authenticated LAN video and pong",20_000) {
                    val s=snap()
                    s["viewer_transport_phase"]=="lan_video" && s["rtt_ms"] is Number &&
                        silent.authenticatedPongs>0
                }
                val before=snap()
                val started=SystemClock.elapsedRealtime()
                silent.stopAnsweringPings()
                await("LAN authenticated ping unanswered",10_000) { silent.unansweredPings>0 }
                val missedAt=SystemClock.elapsedRealtime()
                // Video continues, so the 20-second no-frame watchdog is not
                // the cause of the fallback. Do not close the fixture socket.
                Thread.sleep(4000)
                val during=snap()
                assertEquals("lan_video",during["viewer_transport_phase"])
                assertTrue((during["viewer_lan_video_bytes"] as Number).toLong() >
                    (before["viewer_lan_video_bytes"] as Number).toLong())
                await("authenticated heartbeat fallback to live Windows",45_000) {
                    val s=snap()
                    s["viewer_transport_phase"]=="relay_video" && s["viewer_connected"]==true &&
                        s["viewer_allow_input"]==true && s["viewer_allow_clipboard"]==true &&
                        s["lan_fallback_first_video_ms"] is Number
                }
                val s=snap()
                assertEquals("lan_heartbeat_timeout",s["lan_fallback_reason"])
                assertEquals(1,(s["lan_candidate_attempts"] as Number).toInt())
                assertEquals(1L,silent.unansweredPings)
                assertTrue((s["viewer_relay_video_bytes"] as Number).toLong()>0)
                val openMs=(s["lan_fallback_relay_open_ms"] as Number).toLong()
                val videoMs=(s["lan_fallback_first_video_ms"] as Number).toLong()
                assertTrue(openMs>=0 && videoMs>=openMs)
                metrics.put("lan_authenticated_heartbeat_timeout",JSONObject()
                    .put("unanswered_pings",silent.unansweredPings)
                    .put("lan_video_bytes_during_silence",
                        (during["viewer_lan_video_bytes"] as Number).toLong()-(before["viewer_lan_video_bytes"] as Number).toLong())
                    .put("observed_silence_to_public_video_ms",SystemClock.elapsedRealtime()-missedAt)
                    .put("observed_total_ms",SystemClock.elapsedRealtime()-started)
                    .put("relay_open_ms",openMs).put("first_video_ms",videoMs))
                success("Real LAN encrypted video continued but authenticated PONG stopped; heartbeat deadline recovered public Windows video")
                command("disconnectViewer")
            }
            root.fill(0)
            stage="disconnect"
            command("disconnectViewer")
            await("manual disconnect") { snap()["viewer_requested"]==false && snap()["viewer_connected"]==false }
            val idle = snap()
            assertEquals("idle",idle["viewer_transport_phase"])
            assertEquals(0,(idle["lan_candidate_attempts"] as Number).toInt())
            assertEquals(0L,(idle["network_change_events"] as Number).toLong())
            assertEquals(0L,(idle["network_retry_wakes"] as Number).toLong())
            assertEquals("none",idle["active_video_codec"])
            assertEquals(false,idle["host_metadata_current"])
            assertEquals("",idle["video_encoder"])
            assertEquals("unknown",idle["capture_backend"])
            assertNull(idle["host_relay_socket_open"])
            assertNull(idle["host_relay_video_bytes"])
            assertNull(idle["lan_fallback_reason"])
            assertNull(idle["rtt_ms"])
            for (field in listOf("rendered_fps","measured_video_bps","avc_rendered_frames",
                "avc_received_bytes","received_frames","viewer_lan_video_bytes","viewer_relay_video_bytes",
                "relay_socket_opens","relay_retry_events","host_encoded_frames")) {
                assertEquals("stale disconnected metric: $field",0.0,(idle[field] as Number).toDouble(),0.0)
            }
            metrics.put("disconnected_diagnostics_cleared",true)
            success("Explicit disconnect stops reconnect and clears live/session diagnostics")
            report.put("passed",true)
            phase("done")
        } catch (error: Throwable) {
            report.put("passed",false).put("failed_stage",stage).put("error_type",error.javaClass.simpleName)
            phase("failed")
            // Do not print exception contents (could include private networking details).
            throw AssertionError("Public relay E2E failed at "+stage)
        } finally {
            engine?.let { main { it.close() } }
            avcConsumer?.close()
            metrics.put("verified_frames",frames.get()).put("recorded_frames",recorded)
            report.put("metrics",metrics).put("steps",steps)
            File(out,"report.json").writeText(report.toString(2))
        }
    }
}
