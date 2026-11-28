package com.p2p.remote

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

class LanReadmissionCandidateTest {
    private val root = ByteArray(32) { 7 }
    private val host = ByteArray(32) { 2 }
    private fun prefix(kind: String) = "P2P-LAN-$kind-v5".toByteArray(Charsets.UTF_8)

    @Test fun stagedTranscriptRejectsEarlyReadyAndSeparatesKeys() {
        val nonce = ByteArray(32) { 1 }
        val s = LanStagedHandshake(nonce)
        fun transcript(kind: String) = prefix(kind) + nonce + host
        assertNull(s.commit())
        assertFalse(s.acceptReady(transcript("READY")))
        assertArrayEquals(transcript("PROOF"),s.acceptChallenge(transcript("CHALLENGE")))
        assertFalse(s.acceptReady(transcript("READY")))
        assertTrue(s.acceptPrepared(transcript("PREPARED")))
        assertFalse(s.acceptPrepared(transcript("PREPARED")))
        assertFalse(s.acceptReady(transcript("READY")))
        assertArrayEquals(transcript("COMMIT"),s.commit())
        assertNull(s.commit())
        assertFalse(s.acceptReady(prefix("READY")+nonce+ByteArray(32){3}))
        assertTrue(s.acceptReady(transcript("READY")))
        val legacy = LanHandshake(nonce)
        legacy.acceptChallenge("P2P-LAN-CHALLENGE-v4".toByteArray()+nonce+host)
        assertTrue(legacy.acceptReady("P2P-LAN-READY-v4".toByteArray()+nonce+host))
        assertFalse(s.businessKey(root).contentEquals(legacy.businessKey(root)))
        assertFalse(s.acceptReady(transcript("READY")))
    }

    @Test fun realWebSocketWaitsForExplicitCommit() = exercise(false, false)
    @Test fun realWebSocketCancelBeforeCommit() = exercise(true, false)
    @Test fun realWebSocketRejectsTamperedPrepared() = exercise(false, true)

    private fun exercise(cancel: Boolean, tamper: Boolean) {
        val server = MockWebServer()
        val owner = Executors.newSingleThreadExecutor()
        val http = OkHttpClient()
        val prepared = CountDownLatch(1)
        val done = CountDownLatch(1)
        val commits = AtomicInteger()
        val failure = AtomicReference<String>()
        val actualKey = AtomicReference<ByteArray>()
        val fixtureError = AtomicReference<Throwable>()
        val fixtureClient = AtomicReference<ByteArray>()
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            private var step = 0
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                try {
                    val (kind, data) = decryptPacket(root, bytes.toByteArray())
                    check(kind == 6)
                    val response = when (step++) {
                        0 -> {
                            val hello = prefix("HELLO")
                            check(data.size == hello.size+32 && data.copyOfRange(0,hello.size).contentEquals(hello))
                            val nonce = data.copyOfRange(hello.size,data.size)
                            fixtureClient.set(nonce)
                            prefix("CHALLENGE")+nonce+host
                        }
                        1 -> {
                            check(data.contentEquals(prefix("PROOF")+fixtureClient.get()+host))
                            prefix("PREPARED")+fixtureClient.get()+host
                        }
                        2 -> {
                            check(data.contentEquals(prefix("COMMIT")+fixtureClient.get()+host))
                            commits.incrementAndGet()
                            prefix("READY")+fixtureClient.get()+host
                        }
                        else -> error("unexpected handshake step")
                    }
                    val packet = encryptPacket(root,7,response)
                    if (tamper && step == 2) packet[packet.lastIndex] = (packet.last().toInt() xor 1).toByte()
                    check(webSocket.send(packet.toByteString()))
                } catch (t: Throwable) {
                    fixtureError.set(t)
                    webSocket.cancel()
                }
            }
        }))
        server.start()
        var candidate: LanReadmissionCandidate? = null
        try {
            val start = System.nanoTime()
            val clock = { TimeUnit.NANOSECONDS.toMillis(System.nanoTime()-start) }
            val c = LanReadmissionCandidate(http,Request.Builder().url(server.url("/direct")).build(),
                root,clock,15_000L,{ task -> owner.execute(task) },
                { prepared.countDown() },
                { _, key -> actualKey.set(key); done.countDown() },
                { _, reason -> failure.set(reason); done.countDown() })
            candidate = c
            owner.submit { c.start() }.get(5,TimeUnit.SECONDS)
            if (tamper) {
                assertTrue(done.await(5,TimeUnit.SECONDS))
                assertEquals("authentication_failed",failure.get())
                assertEquals(1L,prepared.count)
            } else {
                assertTrue(prepared.await(5,TimeUnit.SECONDS))
                assertEquals(0,commits.get())
                if (cancel) {
                    owner.submit { c.cancel() }.get(5,TimeUnit.SECONDS)
                    assertFalse(owner.submit<Boolean> { c.commit() }.get(5,TimeUnit.SECONDS))
                    assertEquals(0,commits.get())
                } else {
                    assertTrue(owner.submit<Boolean> { c.commit() }.get(5,TimeUnit.SECONDS))
                    assertTrue(done.await(5,TimeUnit.SECONDS))
                    assertNull(failure.get())
                    assertEquals(1,commits.get())
                    val mac = javax.crypto.Mac.getInstance("HmacSHA256")
                    mac.init(javax.crypto.spec.SecretKeySpec(root,"HmacSHA256"))
                    mac.update(prefix("BUSINESS"))
                    mac.update(fixtureClient.get())
                    assertArrayEquals(mac.doFinal(host),actualKey.get())
                }
            }
            assertNull(fixtureError.get())
        } finally {
            candidate?.let { c -> owner.submit { c.cancel() }.get(5,TimeUnit.SECONDS) }
            http.dispatcher.executorService.shutdown()
            http.connectionPool.evictAll()
            server.shutdown()
            owner.shutdownNow()
        }
    }
}
