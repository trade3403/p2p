package com.p2p.remote

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString

/**
 * A separate, single-use candidate transport. All state and callbacks run on
 * the owner's serial executor (the engine main thread when integrated).
 * Never modifies the active viewer socket, decoder or input permissions.
 * Caller must validate subnet and reservation before construction and commit,
 * and schedule the absolute deadline. No automatic commit on PREPARED.
 */
internal class LanReadmissionCandidate(
    private val client: OkHttpClient,
    private val request: Request,
    root: ByteArray,
    private val now: () -> Long,
    private val deadlineMs: Long,
    private val dispatch: (() -> Unit) -> Unit,
    private val onPrepared: (LanReadmissionCandidate) -> Unit,
    private val onReady: (LanReadmissionCandidate, ByteArray) -> Unit,
    private val onFailed: (LanReadmissionCandidate, String) -> Unit,
) {
    private enum class Stage { NEW, CONNECTING, PROOF, PREPARED, COMMIT, READY, CLOSED }
    private var stage = Stage.NEW
    private var socket: WebSocket? = null
    private val key = root.copyOf()
    private val handshake = LanStagedHandshake()

    init {
        require(root.size == 32)
        require(request.header("Authorization") == null)
    }

    fun start() {
        check(stage == Stage.NEW)
        if (now() >= deadlineMs) {
            fail("timeout")
            return
        }
        stage = Stage.CONNECTING
        socket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                dispatch {
                    if (current(webSocket)) {
                        if (stage != Stage.CONNECTING || !send(handshake.hello())) fail("hello_failed")
                    }
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                // Only small authenticated handshake packets are accepted by
                // this candidate. Business messages cannot reach the engine.
                if (bytes.size > 256) {
                    dispatch { if (current(webSocket)) fail("unexpected_packet") }
                    return
                }
                dispatch {
                    if (current(webSocket)) receive(bytes.toByteArray())
                }
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                dispatch { if (current(webSocket)) fail("unexpected_packet") }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                dispatch { if (current(webSocket)) fail("transport_failure") }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                dispatch { if (current(webSocket)) fail("peer_closed") }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                dispatch { if (current(webSocket)) fail("peer_closed") }
            }
        })
    }

    private fun current(ws: WebSocket): Boolean {
        if (socket !== ws || stage == Stage.CLOSED) return false
        if (now() >= deadlineMs) {
            fail("timeout")
            return false
        }
        return true
    }

    private fun send(payload: ByteArray): Boolean =
        runCatching { socket?.send(encryptPacket(key, 6, payload).toByteString()) == true }
            .getOrDefault(false)

    private fun receive(packet: ByteArray) {
        val decoded = runCatching { decryptPacket(key, packet) }.getOrNull()
        if (decoded == null || decoded.first != 7) {
            fail("authentication_failed")
            return
        }
        val payload = decoded.second
        when (stage) {
            Stage.CONNECTING -> {
                val proof = handshake.acceptChallenge(payload)
                if (proof == null) fail("invalid_challenge")
                else {
                    stage = Stage.PROOF
                    if (!send(proof)) fail("proof_failed")
                }
            }
            Stage.PROOF -> {
                if (!handshake.acceptPrepared(payload)) fail("invalid_prepared")
                else {
                    stage = Stage.PREPARED
                    onPrepared(this)
                }
            }
            Stage.COMMIT -> {
                if (!handshake.acceptReady(payload)) fail("invalid_ready")
                else {
                    stage = Stage.READY
                    val business = handshake.businessKey(key)
                    key.fill(0)
                    onReady(this, business)
                }
            }
            else -> fail("unexpected_step")
        }
    }

    fun commit(): Boolean {
        if (stage != Stage.PREPARED) return false
        if (now() >= deadlineMs) {
            fail("timeout")
            return false
        }
        val payload = requireNotNull(handshake.commit())
        stage = Stage.COMMIT
        if (!send(payload)) {
            fail("commit_failed")
            return false
        }
        return true
    }

    fun expire() {
        if (stage != Stage.CLOSED && now() >= deadlineMs) fail("timeout")
    }

    fun cancel() {
        if (stage == Stage.CLOSED) return
        stage = Stage.CLOSED
        key.fill(0)
        val old = socket
        socket = null
        old?.cancel()
    }

    private fun fail(reason: String) {
        if (stage == Stage.CLOSED) return
        cancel()
        onFailed(this, reason)
    }
}
