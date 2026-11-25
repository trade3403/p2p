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
        private val avc: List<ByteArray> = emptyList()) : AutoCloseable {
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
        val authenticatedPongs: Long get() = pongCount.get()

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
            address = Collections.list(NetworkInterface.getNetworkInterfaces())
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
                        if (packet[0] == 8.toByte()) {
                            val key = requireNotNull(business)
                            val ping = decode(key,packet)
                            val nonce = ByteBuffer.wrap(packet,1,12)
                            require(nonce.int == 0x56494557)
                            val n = nonce.long
                            require(n > received && n <= 0x7fff_ffffL && ping.size == 8)
                            received = n
                            sendBusiness(webSocket,key,9,ping)
                            pongCount.incrementAndGet()
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
                            check(webSocket.send(encode(root,7,prefix("CHALLENGE")+fresh+host)))
                        } else {
                            require(body.contentEquals(prefix("PROOF")+current+host))
                            val mac = Mac.getInstance("HmacSHA256")
                            mac.init(SecretKeySpec(root,"HmacSHA256"))
                            val key = mac.doFinal("P2P-LAN-BUSINESS-v4".toByteArray(Charsets.UTF_8)+current+host)
                            business = key
                            check(webSocket.send(encode(root,7,prefix("READY")+current+host)))
                            var index = 0
                            worker.scheduleAtFixedRate({
                                try {
                                    if (avc.isEmpty()) sendBusiness(webSocket,key,1,jpeg)
                                    else {
                                        sendBusiness(webSocket,key,5,avc[index])
                                        index=(index+1)%avc.size
                                    }
                                } catch (failure: Throwable) { error.compareAndSet(null,failure) }
                            }, 200, if(avc.isEmpty()) 200 else 100, TimeUnit.MILLISECONDS)
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
            // Independent AVC session, after every live Windows/JPEG assertion.
            // This is an encoded test pattern, not a physical LAN host capture.
            stage="lan_avc_setup"
            val fixtureFile=File(ctx.filesDir,"ci-lan-avc.bin")
            val fixture=ByteBuffer.wrap(fixtureFile.readBytes())
            fixtureFile.delete()
            val units=mutableListOf<ByteArray>()
            while(fixture.hasRemaining()) {
                val size=fixture.int
                require(size in 9..fixture.remaining())
                val unit=ByteArray(size)
                fixture.get(unit)
                requireNotNull(AvcPayload.parse(unit))
                units.add(unit)
            }
            assertEquals(20,units.size)
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
                await("real MediaCodec LAN surface output",30_000) {
                    val v=snap()
                    v["transport"]=="局域网 P2P 直连" && v["h264_active"]==true &&
                        v["viewer_connected"]==true && (v["avc_rendered_frames"] as Number).toLong()>beforeAvc &&
                        consumer.frames>0 && v["rtt_ms"] is Number
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
            root.fill(0)
            stage="disconnect"
            command("disconnectViewer")
            await("manual disconnect") { snap()["viewer_requested"]==false && snap()["viewer_connected"]==false }
            success("Explicit disconnect stops reconnect")
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
