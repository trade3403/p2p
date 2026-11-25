package com.p2p.remote

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
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

/** Runs the actual Android native viewer on a disposable emulator. No mock frames. */
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
    private class LanPeer(private val root: ByteArray, private val jpeg: ByteArray) : AutoCloseable {
        private val server = MockWebServer()
        private val worker = Executors.newSingleThreadScheduledExecutor()
        private val peer = AtomicReference<WebSocket>()
        private val error = AtomicReference<Throwable>()
        private val random = SecureRandom()
        private val host = ByteArray(32).also { random.nextBytes(it) }
        private var client: ByteArray? = null
        private var sequence = 0L
        val address: String

        private fun prefix(kind: String) = "P2P-LAN-$kind-v4".toByteArray(Charsets.UTF_8)
        private fun encode(key: ByteArray, kind: Int, body: ByteArray, nonce: ByteArray =
            ByteArray(12).also { random.nextBytes(it) }): ByteString {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key,"AES"), GCMParameterSpec(128,nonce))
            cipher.updateAAD(byteArrayOf(kind.toByte()))
            return (byteArrayOf(kind.toByte()) + nonce + cipher.doFinal(body)).toByteString()
        }
        private fun decode(packet: ByteArray): ByteArray {
            require(packet.size >= 30 && packet[0] == 6.toByte())
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(root,"AES"), GCMParameterSpec(128,packet.copyOfRange(1,13)))
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
                        if (bytes.size == 0 || bytes[0] != 6.toByte()) return
                        val body = decode(bytes.toByteArray())
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
                            check(webSocket.send(encode(root,7,prefix("READY")+current+host)))
                            worker.scheduleAtFixedRate({
                                try {
                                    val nonce = ByteBuffer.allocate(12).putInt(0x484f5354).putLong(++sequence).array()
                                    webSocket.send(encode(key,1,jpeg,nonce))
                                } catch (failure: Throwable) { error.compareAndSet(null,failure) }
                            }, 200, 200, TimeUnit.MILLISECONDS)
                        }
                    } catch (failure: Throwable) {
                        error.compareAndSet(null,failure)
                        webSocket.cancel()
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
            peer.get()?.cancel()
            server.shutdown()
        }
    }

    @Test fun realPublicRelayPairingVideoInputAndRecovery() {
        val ctx = inst.targetContext
        val configFile = File(ctx.filesDir,"ci-private.json")
        val cfg = JSONObject(configFile.readText())
        configFile.delete()
        require(cfg.getString("server") == "wss://p2ptest.aikissgo.com")
        val out = File(ctx.filesDir,"e2e").apply { deleteRecursively(); mkdirs() }
        val report = JSONObject().put("scope","actual AndroidRemoteEngine and native Windows Host via public WSS; LAN peer is an emulator-local encrypted fixture, not a physical Windows LAN host")
        val metrics = JSONObject()
        val steps = org.json.JSONArray()
        val lastState = AtomicReference("")
        val lastJpeg = AtomicReference<ByteArray>()
        val frames = AtomicLong()
        var lastRecord = 0L
        var recorded = 0
        var stage = "initialize"
        var engine: AndroidRemoteEngine? = null
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
            command("requestPairing",mapOf("deviceNumber" to cfg.getString("number"),"pairingCode" to cfg.getString("code")))
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
                success("Actual encrypted LAN socket and fixture JPEG; zero viewer relay opens")
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
                success("LAN fixture close automatically falls back to real Windows WSS video")
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
            metrics.put("verified_frames",frames.get()).put("recorded_frames",recorded)
            report.put("metrics",metrics).put("steps",steps)
            File(out,"report.json").writeText(report.toString(2))
        }
    }
}
