package com.p2p.remote

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Surface
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Viewer-side Android remote engine shared by the Flutter Android UI.
 * It reuses the existing numeric pairing, AES-GCM, WSS Relay, UDP probe,
 * reconnect, monitor, input and clipboard protocol implementation.
 */
internal class AndroidRemoteEngine(
    private val context: Context,
    private val onEvent: (Map<String, Any?>) -> Unit,
    private val onVideoSize: (Int, Int) -> Unit,
) {
    private val credentials = RelayCredentialStore(context)
    private val main = Handler(Looper.getMainLooper())
    private val client = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .build()
    @Volatile private var socket: WebSocket? = null
    private var secret: ByteArray? = null
    @Volatile private var lanBusinessKey: ByteArray? = null
    private var lanSequence: LanSequence? = null
    private fun encryptBusiness(key: ByteArray, kind: Int, plain: ByteArray): ByteArray {
        val nonce = if (usingLan) requireNotNull(lanSequence).nextNonce() else null
        return encryptEnginePacket(key, kind, plain, nonce)
    }
    private fun businessKey(): ByteArray? = if (usingLan) lanBusinessKey else secret
    private val decoding = AtomicBoolean(false)
    private var online = false
    private var requested = false
    private var receivingFrames = false
    private var receivedFrames = 0L
    private var activeEndpoint = ""
    private var lanAdvertised = false
    private var currentLanAddress: String? = null
    private var networkChanges = 0L
    private var networkRetryWakes = 0L
    private val networkObserver = AndroidNetworkObserver(context, main) { change -> onNetworkChanged(change) }
    private val lanAttempt = LanCandidatePlan()
    private val lanReadmission = LanReadmissionPolicy()
    private var readmissionCandidate: LanReadmissionCandidate? = null
    private var readmissionReservation: LanReadmissionPolicy.Reservation? = null
    private var usingLan = false
    @Volatile private var lanVerified = false
    private var transport = "未连接"
    private val lanRecovery = LanRecoveryMetrics()
    private var lanFailureHint: String? = null
    private var activeToken = ""
    private var retryAttempt = 0
    // Counters cover authenticated video packets admitted past the decoder
    // gate, not all wire traffic or necessarily displayed video frames.
    private class TransportVideo {
        val lanBytes = AtomicLong()
        val relayBytes = AtomicLong()
        val lanFrames = AtomicLong()
        val relayFrames = AtomicLong()
        fun record(lan: Boolean, bytes: Int) {
            (if (lan) lanBytes else relayBytes).addAndGet(bytes.toLong())
            (if (lan) lanFrames else relayFrames).incrementAndGet()
        }
    }
    private var transportVideo = TransportVideo()
    private var relaySocketOpens = 0L
    private var relayRetryEvents = 0L
    private var retryTask: Runnable? = null
    private var watchdogTask: Runnable? = null
    private var lanHeartbeatTask: Runnable? = null
    private val lanHeartbeat = LanHeartbeat { SecureRandom().nextLong() }
    private var lastFrameAtMs = 0L
    private var status = "等待配置服务器"
    private var stunReport = "STUN UDP 尚未探测"
    private var rttMs: Long? = null
    private var udpProbe: UdpPunchProbe? = null
    private var udpAttempted = false
    private var udpReport = "未执行公网 UDP 双端验证"
    private var udpPreviewFrames = 0
    @Volatile private var udpLastFrameAtMs = 0L
    private var lastPingNonce = 0L
    private var lastPingTimeMs = 0L
    private var frame: Bitmap? = null
    private var h264Active = false
    private var hasVideoSurface = false
    private var avcRequested = false
    private val fullHdPolicy=FullHdPolicy()
    private var hostFullHdNegotiation=false
    private val localFullHdCapable by lazy { H264SurfaceDecoder.supportsFullHd30() }
    private var renderedAvcFrames = 0L
    private var receivedAvcBytes = 0L
    private var videoTargetBps = 0
    private var videoTargetFps = 0
    private var videoEncoder = ""
    private var captureBackend = "unknown"
    private var captureAvgUs = 0L
    private var encodeAvgUs = 0L
    private var sendAvgUs = 0L
    private var freshFrames = 0L
    private var reusedFrames = 0L
    private var resizeCacheHits = 0L
    private var yuvCacheHits = 0L
    private var hostEncodedFrames = 0L
    private var hostMetadataCurrent = false
    private var hostLanVideoBytes: Long? = null
    private var hostRelayVideoBytes: Long? = null
    private var hostRelaySuppressedFrames: Long? = null
    private var hostLanVideoFrames: Long? = null
    private var hostRelayVideoFrames: Long? = null
    private var hostRelaySocketOpen: Boolean? = null
    private var hostRelaySocketOpens: Long? = null
    private var sampleAt = 0L
    private var sampleFrames = 0L
    private var sampleDrops = 0L
    private var renderedFps = 0.0
    private var sampleBytes = 0L
    private var measuredBps = 0L
    private var h264Width = 1280
    private var h264Height = 720
    private var clipboardResult = ""
    private var monitorCount = 1
    private var selectedMonitor = 0
    private var pointerX = 0.5f
    private var pointerY = 0.5f
    private var pointerValid = false
    private var hostAllowsInput: Boolean? = null
    private var hostAllowsClipboard: Boolean? = null
    private var relayUrl = "wss://p2ptest.aikissgo.com"
    private var relayToken = ""
    private var pairingActive = false
    private val pairingCancelled = AtomicBoolean(false)
    private var closed = false
    private var lastEmittedSnapshot: Map<String, Any?>? = null

    private val avcDecoder: H264SurfaceDecoder by lazy {
        H264SurfaceDecoder(
            onRendered = { generation ->
                main.post {
                    if (requested && h264Active && avcDecoder.isCurrent(generation)) {
                        receivingFrames = true
                        receivedFrames++
                        renderedAvcFrames++
                        lastFrameAtMs = SystemClock.elapsedRealtime()
                        fullHdPolicy.rendered(h264Width>1280 || h264Height>720)
                        if (!usingLan) lanRecovery.relayVideo(lastFrameAtMs)
                        retryAttempt = 0
                        status = "已连接家庭电脑 · H.264 正在播放"
                    }
                }
            },
            onDecoderError = { generation, message ->
                main.post {
                    if (requested && avcDecoder.isCurrent(generation)) {
                        status = message
                        // A codec that cannot start must not leave a black screen.
                        if (avcRequested) {
                            if (fullHdPolicy.downgrade()) {
                                receivingFrames=false
                                avcDecoder.reset()
                                sendVideoControl(JSONObject().put("type","video_codec").put("codec","h264"))
                            } else {
                                avcRequested = false
                                sendVideoControl(JSONObject().put("type","video_codec").put("codec","jpeg"))
                            }
                        }
                    }
                }
            },
        )
    }

    private val statePump = object : Runnable {
        override fun run() {
            if (closed) return
            emitSnapshot()
            main.postDelayed(this, 250L)
        }
    }

    init {
        credentials.load()?.let {
            relayUrl = it.relayUrl
            relayToken = it.relayToken
        }
        networkObserver.start()
        main.post(statePump)
    }

    fun attachVideoSurface(surface: Surface) {
        hasVideoSurface = true
        avcDecoder.attachSurface(surface)
    }

    fun snapshotResponse(): Map<String, Any?> =
        mapOf("ok" to true, "snapshot" to (snapshotMap() +
            ("process_cpu_ms" to android.os.Process.getElapsedCpuTime().takeIf { it >= 0L })))

    fun emitSnapshot(force: Boolean = false) {
        val snapshot = snapshotMap()
        if (force || snapshot != lastEmittedSnapshot) {
            lastEmittedSnapshot = snapshot
            onEvent(mapOf("type" to "snapshot", "snapshot" to snapshot))
        }
    }

    private fun snapshotMap(): Map<String, Any?> {
        val decoderOutput = if (h264Active) avcDecoder.outputState() else null
        return mapOf(
        "host_running" to false,
        "viewer_connected" to (online && receivingFrames),
        "viewer_requested" to requested,
        "viewer_allow_input" to (hostAllowsInput == true),
        "viewer_allow_clipboard" to (hostAllowsClipboard == true),
        "received_frames" to receivedFrames,
        "pairing_active" to pairingActive,
        "token_configured" to (relayToken.length >= 32),
        "allow_input" to (hostAllowsInput == true),
        "allow_clipboard" to (hostAllowsClipboard == true),
        "relay_url" to relayUrl,
        "host_status" to "Android 当前作为远程控制端",
        "viewer_status" to status,
        "device_number" to "",
        "pairing_code" to "",
        "pairing_expires_at" to 0,
        "pending_pairing_request_id" to null,
        "pending_viewer_fingerprint" to null,
        "monitors" to mapOf("count" to monitorCount, "selected" to selectedMonitor),
        "clipboard_text" to clipboardResult,
        "transport" to transport,
        "viewer_transport_phase" to when {
            !requested -> if (lanRecovery.reason != null) "failed" else "idle"
            usingLan && !lanVerified -> "lan_verifying"
            usingLan && receivingFrames -> "lan_video"
            usingLan -> "lan_verified"
            udpLastFrameAtMs != 0L && receivingFrames -> "udp_video"
            online && receivingFrames -> "relay_video"
            online -> "relay_open"
            retryTask != null -> "relay_retry"
            else -> "relay_connecting"
        },
        "lan_attempt_ms" to lanRecovery.attemptMs,
        "lan_candidate_attempts" to lanAttempt.attemptCount,
        "lan_readmission_state" to lanReadmission.state(SystemClock.elapsedRealtime()),
        "lan_readmission_batches" to lanReadmission.reservations,
        "network_observer_ready" to networkObserver.ready,
        "network_change_events" to networkChanges,
        "network_retry_wakes" to networkRetryWakes,
        "lan_fallback_reason" to lanRecovery.reason,
        "lan_fallback_started" to lanRecovery.fallbackStarted,
        "lan_fallback_relay_open_ms" to lanRecovery.relayOpenMs,
        "lan_fallback_first_video_ms" to lanRecovery.firstVideoMs,
        "rtt_ms" to rttMs,
        "relay_socket_opens" to relaySocketOpens,
        "viewer_lan_video_bytes" to transportVideo.lanBytes.get(),
        "viewer_relay_video_bytes" to transportVideo.relayBytes.get(),
        "viewer_lan_video_frames" to transportVideo.lanFrames.get(),
        "viewer_relay_video_frames" to transportVideo.relayFrames.get(),
        "relay_retry_events" to relayRetryEvents,
        "udp_report" to udpReport,
        "stun_report" to stunReport,
        "udp_attempted" to udpAttempted,
        "frame_width" to if (h264Active) h264Width else (frame?.width ?: 0),
        "frame_height" to if (h264Active) h264Height else (frame?.height ?: 0),
        "h264_active" to (h264Active && receivingFrames),
        "active_video_codec" to when {
            !online || !receivingFrames -> "none"
            h264Active -> "H264"
            else -> "JPEG"
        },
        "host_metadata_current" to hostMetadataCurrent,
        "avc_rendered_frames" to renderedAvcFrames,
        "avc_received_bytes" to receivedAvcBytes,
        // Identity is the last actual output in the current generation.
        // A same-format decoder rebuild is separately visible as recovering.
        "avc_decoder" to (decoderOutput?.lastOutput?.name ?: ""),
        "avc_decoder_hardware" to decoderOutput?.lastOutput?.hardware,
        "avc_decoder_recovering" to decoderOutput?.recovering,
        "avc_decoder_drops" to if(hasVideoSurface) avcDecoder.droppedFrames else 0L,
        "rendered_fps" to renderedFps,
        "measured_video_bps" to measuredBps,
        "video_target_bps" to videoTargetBps,
        "video_target_fps" to videoTargetFps,
        "full_hd_negotiation" to hostFullHdNegotiation,
        "full_hd_state" to fullHdPolicy.state.name.lowercase(),
        "video_encoder" to videoEncoder,
        "capture_backend" to captureBackend,
        "capture_avg_us" to captureAvgUs,
        "encode_avg_us" to encodeAvgUs,
        "send_avg_us" to sendAvgUs,
        "fresh_frames" to freshFrames,
        "reused_frames" to reusedFrames,
        "resize_cache_hits" to resizeCacheHits,
        "yuv_cache_hits" to yuvCacheHits,
        "host_encoded_frames" to hostEncodedFrames,
        "host_lan_video_bytes" to hostLanVideoBytes,
        "host_relay_video_bytes" to hostRelayVideoBytes,
        "host_relay_suppressed_frames" to hostRelaySuppressedFrames,
        "host_lan_video_frames" to hostLanVideoFrames,
        "host_relay_video_frames" to hostRelayVideoFrames,
        "host_relay_socket_open" to hostRelaySocketOpen,
        "host_relay_socket_opens" to hostRelaySocketOpens,
        "pairing_status" to if (pairingActive) status else "",
        "vault_error" to null,
        )
    }

    fun invoke(method: String, arguments: Map<String, Any?>): Map<String, Any?> {
        var ok = true
        var error: String? = null
        try {
            when (method) {
                "snapshot" -> Unit
                "configureRelay" -> configureRelay(
                    arguments["server"] as? String ?: "",
                    arguments["token"] as? String ?: "",
                )
                "startHost", "stopHost" -> throw UnsupportedOperationException("Android 当前仅提供远程控制端功能")
                "setRemoteInputAllowed", "setClipboardAllowed",
                "approvePairingRequest", "denyPairingRequest" ->
                    throw UnsupportedOperationException("该权限由 Windows 被控端本机用户管理")
                "requestPairing" -> requestPairing(
                    arguments["deviceNumber"] as? String ?: "",
                    arguments["pairingCode"] as? String ?: "",
                )
                "cancelPairing" -> cancelPairing()
                "connectLegacy" -> connectLegacy(arguments["sessionKey"] as? String ?: "")
                "disconnectViewer" -> disconnectViewer()
                "sendPointer" -> sendPointer(
                    arguments["kind"] as? String ?: "",
                    (arguments["x"] as? Number)?.toFloat() ?: 0.5f,
                    (arguments["y"] as? Number)?.toFloat() ?: 0.5f,
                )
                "sendScroll" -> sendScroll((arguments["delta"] as? Number)?.toInt() ?: 0)
                "sendText" -> sendControlText(arguments["text"] as? String ?: "")
                "sendKey" -> sendRemoteKey(arguments["key"] as? String ?: "")
                "selectMonitor" -> selectMonitor((arguments["index"] as? Number)?.toInt() ?: -1)
                "probeSTUN" -> probeSTUN(arguments["server"] as? String ?: relayUrl)
                "startUdpPunch" -> startUdpPunch()
                "getClipboardText" -> send(JSONObject().put("type", "clipboard_get"))
                "setClipboardText" -> sendClipboardSet(arguments["text"] as? String ?: "")
                else -> throw IllegalArgumentException("未知 Android 原生桥接命令：$method")
            }
        } catch (failure: Exception) {
            ok = false
            error = failure.message ?: "Android 原生桥接调用失败"
        }
        val response = mutableMapOf<String, Any?>("ok" to ok, "snapshot" to snapshotMap())
        if (error != null) response["error"] = error
        return response
    }

    private fun configureRelay(server: String, token: String) {
        val normalized = when {
            server.trim().startsWith("wss://") -> server.trim().trimEnd('/')
            server.trim().startsWith("https://") -> "wss://" + server.trim().removePrefix("https://").trimEnd('/')
            else -> throw IllegalArgumentException("中转服务器必须使用 HTTPS/WSS")
        }
        require(!requested && !pairingActive) { "请先断开当前连接或取消配对" }
        val uri = java.net.URI(normalized)
        require(uri.host != null && uri.userInfo == null && uri.query == null &&
            uri.fragment == null && (uri.path.isNullOrEmpty() || uri.path == "/")) { "中转地址只能包含 HTTPS/WSS 主机和端口" }
        val nextToken = token.trim().ifEmpty { relayToken }
        require(nextToken.length in 32..512) { "请输入有效的 Relay Access Token" }
        credentials.save(normalized, nextToken)
        relayUrl = normalized
        relayToken = nextToken
        status = "Relay 配置已通过 Android Keystore 加密保存"
    }

    private fun requestPairing(deviceNumber: String, code: String) {
        require(!pairingActive && !requested) { "当前已有配对或远程会话" }
        require(deviceNumber.matches(Regex("^[0-9]{9}$"))) { "设备号必须是9位数字" }
        require(code.matches(Regex("^[0-9]{6}$"))) { "动态验证码必须是6位数字" }
        require(relayToken.length >= 32) { "请先在网络连接中配置 Relay 访问口令" }
        pairingCancelled.set(false)
        pairingActive = true
        status = "正在发起数字配对…"
        Thread {
            try {
                val key = PairingApiClient(relayUrl, relayToken).pairAndDeriveSession(
                    deviceNumber,
                    code,
                    cancelled = { pairingCancelled.get() },
                    onProgress = { message ->
                        main.post {
                            if (pairingActive && !pairingCancelled.get()) {
                                status = message
                                emitSnapshot()
                            }
                        }
                    },
                )
                val keyHex = key.joinToString("") { "%02x".format(it.toInt() and 0xff) }
                key.fill(0)
                main.post {
                    pairingActive = false
                    if (pairingCancelled.get() || closed) {
                        status = "已取消配对"
                    } else {
                        status = "配对成功，正在建立远程会话…"
                        connect(relayUrl, relayToken, keyHex, "")
                    }
                    emitSnapshot()
                }
            } catch (failure: Exception) {
                main.post {
                    pairingActive = false
                    status = failure.message ?: "配对失败，请检查网络和验证码"
                    emitSnapshot()
                }
            }
        }.start()
    }

    private fun cancelPairing() {
        pairingCancelled.set(true)
        status = "正在取消配对…"
    }

    private fun connectLegacy(sessionKey: String) {
        require(!requested && !pairingActive) { "请先断开当前会话" }
        require(sessionKey.matches(Regex("^[0-9a-fA-F]{64}$"))) { "会话密钥必须是64位十六进制字符串" }
        require(relayToken.length >= 32) { "请先在网络连接中配置 Relay 访问口令" }
        connect(relayUrl, relayToken, sessionKey, "")
    }

    private fun disconnectViewer() {
        pairingCancelled.set(true)
        disconnect()
    }

    private fun sendPointer(kind: String, x: Float, y: Float) {
        require(kind in setOf("move", "down", "up", "right_click")) { "不支持的鼠标事件" }
        sendControl(kind, x.coerceIn(0f, 1f), y.coerceIn(0f, 1f))
    }

    private fun emitFrame(bytes: ByteArray, width: Int, height: Int) {
        if (bytes.isEmpty() || bytes.size > 3 * 1024 * 1024 || width <= 0 || height <= 0) return
        receivedFrames++
        onEvent(mapOf(
            "type" to "frame",
            "frame" to bytes.copyOf(),
            "width" to width,
            "height" to height,
        ))
    }

    private fun connect(server: String, relayToken: String, keyText: String, lanIp: String) {
        val key = try { decodeKey(keyText) } catch (_: Exception) {
            status = "会话密钥必须是 64 位十六进制字符串"
            return
        }
        val source = server.trim().trimEnd('/')
        val normalized = when {
            source.startsWith("https://") -> "wss://" + source.removePrefix("https://")
            source.startsWith("wss://") -> source
            else -> { status = "必须使用 HTTPS/WSS 中转服务器"; return }
        }
        if (normalized.contains("@") || normalized.contains("?") || normalized.contains("#")) {
            status = "中转 URL 不允许包含账号、查询参数或密钥"
            return
        }
        if (relayToken.length < 32 && LanEndpoint.url(lanIp) == null) {
            status = "请填写有效的 VPS 口令，或填写同一局域网内的电脑 IPv4 地址"
            return
        }
        val room = sha256("p2p-v02-room:".toByteArray(Charsets.UTF_8) + key).toHex()
        activeEndpoint = "$normalized/relay?room=$room&role=viewer"
        lanAttempt.reset()
        lanReadmission.reset()
        lanAdvertised = false
        if (LanEndpoint.url(lanIp) != null) {
            lanAttempt.begin(listOf(lanIp.trim()), SystemClock.elapsedRealtime())
        }
        lanRecovery.reset()
        lanFailureHint = null
        usingLan = false
        lanVerified = false
        transport = "连接中"
        activeToken = relayToken
        secret = key
        resetSessionTotals()
        requested = true
        hostAllowsInput = null
        retryAttempt = 0
        context.getSharedPreferences("public_settings", Context.MODE_PRIVATE).edit()
            .putString("relay_url", normalized)
            .putString("lan_ipv4", lanIp.trim()).apply()
        openConnection()
    }

    private fun nextLanCandidate(now: Long): LanCandidatePlan.Attempt? =
        lanAttempt.take(now) { address ->
            LanEndpoint.url(address) != null &&
                (!lanAdvertised || LanEndpoint.isOnLocalSubnet(address))
        }

    private fun openConnection(selected: LanCandidatePlan.Attempt? = null) {
        if (!LanAttemptPolicy.canOpen(requested, socket != null)) return
        retryTask?.let { main.removeCallbacks(it) }
        retryTask = null
        val candidate = selected ?: nextLanCandidate(SystemClock.elapsedRealtime())
        if (candidate == null && activeToken.length < 32) {
            handleLanFailure("connect_or_verify_timeout")
            return
        }
        val tryLan = candidate != null
        if (tryLan) {
            if (lanAttempt.attemptCount == 1) lanRecovery.beginLan(SystemClock.elapsedRealtime())
            lanFailureHint = null
        }
        clearLiveDiagnostics()
        usingLan = tryLan
        currentLanAddress = candidate?.address
        lanBusinessKey = null
        lanVerified = false
        transport = if(tryLan) "局域网加密直连 · 验证中" else "VPS WSS 中转 · 连接中"
        status = if(tryLan) "尝试局域网直连（第 ${lanAttempt.attemptCount} 个候选）..." else
            if (retryAttempt == 0) "正在连接中转服务器..." else "正在重新连接中转服务器..."
        val request = try {
            val builder = Request.Builder().url(if(candidate != null) requireNotNull(LanEndpoint.url(candidate.address)) else activeEndpoint)
            if(!tryLan) builder.header("Authorization", "Bearer $activeToken")
            builder.build()
        } catch (_: Exception) {
            status = "连接地址无效"
            if (tryLan) {
                handleLanFailure("invalid_endpoint")
            } else {
                usingLan=false
                lanBusinessKey=null
                lanSequence=null
                requested = false
                secret = null
                activeToken = ""
            }
            return
        }
        val handshake = if (tryLan) LanHandshake() else null
        val replayGuard = if (tryLan) LanSequence() else null
        lanSequence = replayGuard
        val ws = client.newWebSocket(request,
            sessionListener(tryLan, candidate, handshake, replayGuard, transportVideo))
        socket = ws
        if (candidate != null) main.postDelayed({
            if (socket === ws && requested && !lanVerified) expireLanCandidate(ws)
        }, (candidate.deadlineMs - SystemClock.elapsedRealtime()).coerceAtLeast(0L))
    }

    // The v5 candidate installs this same business listener after READY.
    // Its v4 handshake callback is never invoked for a promoted socket.
    private fun sessionListener(
        tryLan: Boolean,
        candidate: LanCandidatePlan.Attempt?,
        handshake: LanHandshake?,
        replayGuard: LanSequence?,
        videoCounters: TransportVideo,
    ): WebSocketListener = object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                main.post {
                    if (socket === webSocket && requested) {
                        receivingFrames = false
                        if(tryLan) {
                            status = "局域网连接建立 · 正在验证会话密钥"
                            val currentKey = secret
                            if(currentKey == null ||
                                !webSocket.send(
                                    encryptEnginePacket(
                                        currentKey, 6, requireNotNull(handshake).hello()
                                    ).toByteString()
                                )
                            ) {
                                webSocket.cancel()
                            }
                        }else{
                            relaySocketOpens++
                            lanRecovery.relayOpened(SystemClock.elapsedRealtime())
                            online = true
                            transport = "VPS WSS 中转"
                            status = "已连接中转服务器，等待家庭电脑画面"
                            startWatchdog()
                        }
                    }
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (socket !== webSocket || !requested) return
                // Signaling must not be dropped by a busy JPEG/H.264 decoder.
                // AES-GCM authenticates this outer message kind as AAD.
                if (bytes.size > 0 && (bytes[0].toInt() and 255) == 10) {
                    try {
                        val key = secret ?: return
                        val message = decryptEnginePacket(key, bytes.toByteArray())
                        if (message.first == 10 && message.second.size in 2..256 && !tryLan) {
                            val candidate = JSONObject(message.second.toString(Charsets.UTF_8))
                            main.post {
                                if (socket === webSocket && requested) {
                                    udpProbe?.setCandidate(candidate)
                                }
                            }
                        }
                    } catch (_: Exception) {}
                    return
                }
                // Handle LAN handshake before the decoder gate; all transcript
                // state is owned by the main thread and bound to this socket.
                if (tryLan && handshake != null && bytes.size > 0 && (bytes[0].toInt() and 255) == 7) {
                    try {
                        val key = secret ?: return
                        val (kind, payload) = decryptEnginePacket(key, bytes.toByteArray())
                        if (kind != 7) return
                        main.post {
                            if (socket === webSocket && requested && !lanVerified) {
                                if (SystemClock.elapsedRealtime() >= requireNotNull(candidate).deadlineMs) {
                                    expireLanCandidate(webSocket)
                                    return@post
                                }
                                val state = requireNotNull(handshake)
                                val proof = state.acceptChallenge(payload)
                                if (proof != null) {
                                    if (!webSocket.send(encryptEnginePacket(key, 6, proof).toByteString())) {
                                        webSocket.cancel()
                                    }
                                } else if (state.acceptReady(payload)) {
                                    lanBusinessKey = state.businessKey(key)
                                    lanVerified = true
                                    lanAttempt.verified()
                                    lanRecovery.verified(SystemClock.elapsedRealtime())
                                    online = true
                                    receivingFrames = false
                                    transport = "局域网 P2P 直连"
                                    status = "已通过局域网加密验证 · 等待画面"
                                    startWatchdog()
                                    startLanHeartbeat(webSocket, requireNotNull(candidate).address)
                                } else {
                                    webSocket.cancel()
                                }
                            }
                        }
                    } catch (_: Exception) { webSocket.cancel() }
                    return
                }
                // Authenticated LAN PONG must not be dropped by the video
                // decoder gate (which can still be busy with an older socket).
                if (tryLan && bytes.size > 0 && (bytes[0].toInt() and 255) == 9) {
                    try {
                        val key = lanBusinessKey ?: return
                        if (!lanVerified) return
                        val packet = bytes.toByteArray()
                        val (kind, body) = decryptEnginePacket(key, packet)
                        if (kind != 9 || body.size != 8 || socket !== webSocket ||
                            !requireNotNull(replayGuard).accept(packet)) return
                        val token = ByteBuffer.wrap(body).long
                        main.post {
                            if (socket === webSocket && requested && lanVerified) {
                                lanHeartbeat.pong(token, SystemClock.elapsedRealtime())?.let { rttMs = it }
                            }
                        }
                    } catch (_: Exception) {}
                    return
                }
                if (!decoding.compareAndSet(false, true)) return
                try {
                    val key = (if (tryLan) lanBusinessKey else secret) ?: return
                    val packet = bytes.toByteArray()
                    val original = decryptEnginePacket(key, packet)
                    if (tryLan && !lanVerified) return
                    if (replayGuard != null && !replayGuard.accept(packet)) return
                    if (socket !== webSocket) return
                    if (original.first == 1 || original.first == 5) videoCounters.record(tryLan, packet.size)
                    when (original.first) {
                        9 -> {
                            if(original.second.size == 8) {
                                val nonce = ByteBuffer.wrap(original.second).long
                                main.post {
                                    if(socket === webSocket && requested &&
                                        lastPingTimeMs > 0L && nonce == lastPingNonce
                                    ) {
                                        rttMs = (SystemClock.elapsedRealtime() - lastPingTimeMs)
                                            .coerceAtLeast(0)
                                        lastPingTimeMs = 0
                                    }
                                }
                            }
                        }
                        1 -> {
                            if (udpLastFrameAtMs != 0L &&
                                SystemClock.elapsedRealtime() - udpLastFrameAtMs < 1500L) return
                            val decoded = BitmapFactory.decodeByteArray(original.second, 0, original.second.size)
                            if (decoded != null) main.post {
                                if (socket === webSocket && requested) {
                                    if(h264Active) avcDecoder.reset()
                                    frame = decoded
                                    emitFrame(original.second, decoded.width, decoded.height)
                                    h264Active = false
                                    renderedFps = 0.0
                                    measuredBps = 0L
                                    if(udpLastFrameAtMs != 0L) {
                                        udpLastFrameAtMs = 0L
                                        transport = "VPS WSS 中转"
                                    }
                                    receivingFrames = true
                                    lastFrameAtMs = SystemClock.elapsedRealtime()
                                    if (!tryLan) lanRecovery.relayVideo(lastFrameAtMs)
                                    retryAttempt = 0
                                    status = "已连接家庭电脑 · 正在接收画面"
                                }
                            }
                        }
                        5 -> {
                            if (udpLastFrameAtMs != 0L &&
                                SystemClock.elapsedRealtime() - udpLastFrameAtMs < 1500L) return
                            val accessUnit = AvcPayload.parse(original.second)
                            if(accessUnit != null && hasVideoSurface) {
                                main.post {
                                    if(socket===webSocket && requested){
                                        if(udpLastFrameAtMs != 0L) {
                                            udpLastFrameAtMs = 0L
                                            transport = "VPS WSS 中转"
                                        }
                                        if (fullHdPolicy.rejectRetiredFormat(accessUnit.width,accessUnit.height)) return@post
                                        if (!h264Active || h264Width!=accessUnit.width || h264Height!=accessUnit.height) {
                                            // A new format is not connected until the current
                                            // decoder generation actually submits output.
                                            receivingFrames=false
                                            h264Width=accessUnit.width
                                            h264Height=accessUnit.height
                                            onVideoSize(h264Width,h264Height)
                                        }
                                        receivedAvcBytes += original.second.size.toLong()
                                        h264Active=true
                                        frame=null
                                        // Queue AVC only after the main-thread socket identity
                                        // check, ordered with disconnect/reset and codec changes.
                                        avcDecoder.offer(accessUnit.bytes,fullHdPolicy.requiresHardware())
                                    }
                                }
                            }
                        }
                        3 -> {
                            if (original.second.size <= 4096) {
                                val value = original.second.toString(Charsets.UTF_8)
                                main.post { if(socket===webSocket && requested) clipboardResult=value }
                            }
                        }
                        4 -> {
                            val metadata = JSONObject(original.second.toString(Charsets.UTF_8))
                            val count = metadata.optInt("count", 1).coerceIn(1, 16)
                            val index = metadata.optInt("selected", 0).coerceIn(0, count - 1)
                            val inputAllowed = if(metadata.has("allow_input"))
                                metadata.optBoolean("allow_input") else null
                            val clipboardAllowed = if(metadata.has("allow_clipboard"))
                                metadata.optBoolean("allow_clipboard") else null
                            main.post {
                                if(socket===webSocket && requested){
                                        monitorCount=count;selectedMonitor=index
                                        hostMetadataCurrent = true
                                        hostAllowsInput=inputAllowed
                                        hostAllowsClipboard=clipboardAllowed
                                        videoTargetBps=metadata.optInt("video_target_bps",0)
                                        videoTargetFps=metadata.optInt("video_target_fps",0)
                                        hostFullHdNegotiation=metadata.opt("video_full_hd_gpu_negotiation")==true
                                        videoEncoder=metadata.optString("video_encoder","")
                                        captureBackend=metadata.optString("capture_backend","unknown")
                                        captureAvgUs=metadata.optLong("capture_avg_us",0)
                                        encodeAvgUs=metadata.optLong("encode_avg_us",0)
                                        sendAvgUs=metadata.optLong("send_avg_us",0)
                                        freshFrames=metadata.optLong("fresh_frames",0)
                                        reusedFrames=metadata.optLong("reused_frames",0)
                                        resizeCacheHits=metadata.optLong("resize_cache_hits",0)
                                        yuvCacheHits=metadata.optLong("yuv_cache_hits",0)
                                        hostEncodedFrames=metadata.optLong("host_encoded_frames",0)
                                        hostLanVideoBytes=metadata.optLong("host_lan_video_bytes",-1).takeIf { it >= 0 }
                                        hostRelayVideoBytes=metadata.optLong("host_relay_video_bytes",-1).takeIf { it >= 0 }
                                        hostRelaySuppressedFrames=metadata.optLong("host_relay_suppressed_frames",-1).takeIf { it >= 0 }
                                        hostLanVideoFrames=metadata.optLong("host_lan_video_frames",-1).takeIf { it >= 0 }
                                        hostRelayVideoFrames=metadata.optLong("host_relay_video_frames",-1).takeIf { it >= 0 }
                                        hostRelaySocketOpen=if(metadata.has("host_relay_socket_open")) metadata.optBoolean("host_relay_socket_open") else null
                                        hostRelaySocketOpens=metadata.optLong("host_relay_socket_opens",-1).takeIf { it >= 0 }
                                        if (!tryLan) {
                                            val advertised = mutableListOf<String>()
                                            val array = metadata.optJSONArray("lan_ipv4_candidates")
                                            if (array != null) for (i in 0 until minOf(array.length(),16)) {
                                                val candidate = array.opt(i)
                                                if (candidate is String) advertised.add(candidate)
                                            }
                                            advertised.add(metadata.optString("lan_ipv4", ""))
                                            tryAdvertisedLan(webSocket, advertised,
                                                metadata.optBoolean("lan_readmission_v5", false))
                                        }
                                    }
                            }
                        }
                    }
                } catch (_: Exception) {
                    // Invalid or unauthenticated packets never affect the desktop.
                } finally {
                    decoding.set(false)
                }
            }

            override fun onFailure(webSocket: WebSocket, error: Throwable, response: Response?) {
                main.post {
                    if (socket !== webSocket || !requested) return@post
                    cancelReadmission()
                    lanReadmission.networkChanged(SystemClock.elapsedRealtime())
                    socket = null
                    lanBusinessKey = null
                    lanSequence = null
                    online = false
                    hostAllowsInput = null
                    hostAllowsClipboard = null
                    clearLiveDiagnostics()
                    udpProbe?.stop()
                    udpProbe = null
                    udpAttempted = false
                    udpReport = "连接已断开，待新会话重新验证"
                    udpLastFrameAtMs = 0L
                    rttMs = null
                    lastPingTimeMs = 0L
                    receivingFrames = false
                    frame = null
                    h264Active = false
                    avcDecoder.reset()
                    stopWatchdog()
                    if(tryLan) {
                        handleLanFailure(lanFailureHint ?: if (lanVerified) "transport_failure" else "connect_or_verify_failure")
                    } else if (response?.code == 401 || response?.code == 403) {
                        requested = false
                        secret = null
                        activeToken = ""
                        transport = "VPS WSS 中转 · 授权失败"
                        status = "中转访问口令错误（HTTP ${response.code}），已停止重试"
                    } else {
                        scheduleRetry("连接失败：${response?.code ?: error.localizedMessage ?: "网络错误"}")
                    }
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                // Complete the peer's close handshake instead of leaving the
                // client waiting for a watchdog timeout.
                if (!webSocket.close(1000, null)) webSocket.cancel()
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                main.post {
                    if (socket !== webSocket || !requested) return@post
                    cancelReadmission()
                    lanReadmission.networkChanged(SystemClock.elapsedRealtime())
                    socket = null
                    lanBusinessKey = null
                    lanSequence = null
                    online = false
                    hostAllowsInput = null
                    hostAllowsClipboard = null
                    clearLiveDiagnostics()
                    udpProbe?.stop()
                    udpProbe = null
                    udpAttempted = false
                    udpReport = "连接已断开，待新会话重新验证"
                    udpLastFrameAtMs = 0L
                    rttMs = null
                    lastPingTimeMs = 0L
                    receivingFrames = false
                    frame = null
                    h264Active = false
                    avcDecoder.reset()
                    stopWatchdog()
                    if (tryLan) {
                        handleLanFailure(lanFailureHint ?: if (lanVerified) "peer_closed" else "closed_before_verified")
                    } else {
                        scheduleRetry("连接已断开（$code）")
                    }
                }
            }
        }

    // Timeout is handled directly on the main thread; do not wait for an
    // OkHttp failure callback. A late callback cannot affect the next socket.
    private fun expireLanCandidate(ws: WebSocket) {
        if (socket !== ws || !requested || lanVerified) return
        socket = null
        lanBusinessKey = null
        lanSequence = null
        ws.cancel()
        handleLanFailure("connect_or_verify_timeout")
    }

    private fun handleLanFailure(reason: String) {
        val now = SystemClock.elapsedRealtime()
        val next = if (!lanVerified) nextLanCandidate(now) else null
        lanVerified = false
        usingLan = false
        currentLanAddress = null
        if (next != null) {
            openConnection(next)
            return
        }
        lanRecovery.failed(reason, lanAttempt.canFallback(activeToken), now)
        if (lanAttempt.canFallback(activeToken)) {
            lanReadmission.lanFailed(now)
            transport = "局域网不可达，准备切换 VPS"
            status = "局域网直连结束，改走 VPS 中转"
            openConnection()
        } else {
            requested = false
            secret?.fill(0)
            secret = null
            activeToken = ""
            transport = "局域网直连失败"
            status = "局域网直连不可用，且未配置 VPS 回退"
        }
    }

    // One bounded batch per session. Never probe arbitrary or off-subnet addresses.
    private fun tryAdvertisedLan(current: WebSocket, candidates: List<String>, stagedSupported: Boolean) {
        if (!requested || socket !== current || usingLan) return
        val eligible = LanEndpoint.eligibleAdvertised(candidates)
        if (lanAttempt.attempted) {
            val now = SystemClock.elapsedRealtime()
            lanReadmission.observe(now, healthyRelay(current, now) && stagedSupported, eligible)
            val pending = readmissionReservation
            if (pending != null && !lanReadmission.isFresh(pending, now)) cancelReadmission()
            if (readmissionCandidate == null) startReadmission(current, now)
            return
        }
        if (eligible.isEmpty()) return
        lanAdvertised = true
        lanAttempt.begin(eligible, SystemClock.elapsedRealtime())
        val selected = nextLanCandidate(SystemClock.elapsedRealtime()) ?: return
        socket = null
        current.cancel()
        stopWatchdog()
        udpProbe?.stop()
        udpProbe = null
        udpAttempted = false
        udpLastFrameAtMs = 0L
        online = false
        receivingFrames = false
        hostAllowsInput = null
        hostAllowsClipboard = null
        h264Active = false
        frame = null
        rttMs = null
        lastPingTimeMs = 0L
        avcDecoder.reset()
        openConnection(selected)
    }

    private fun healthyRelay(relay: WebSocket, now: Long): Boolean =
        requested && socket === relay && !usingLan && online && receivingFrames &&
            udpLastFrameAtMs == 0L && now - lastFrameAtMs <= 8000L

    private fun cancelReadmission() {
        val old = readmissionCandidate
        readmissionCandidate = null
        readmissionReservation?.let { lanReadmission.complete(it, SystemClock.elapsedRealtime(), false) }
        readmissionReservation = null
        old?.cancel()
    }

    private fun startReadmission(relay: WebSocket, now: Long) {
        if (!healthyRelay(relay, now) || readmissionCandidate != null) return
        val root = secret ?: return
        val reservation = lanReadmission.reserve(now) ?: return
        // One address per bounded batch; rotate through current ranked candidates.
        val address = reservation.candidates[(lanReadmission.reservations - 1) % reservation.candidates.size]
        val endpoint = LanEndpoint.url(address)
        if (endpoint == null || !LanEndpoint.isOnLocalSubnet(address)) {
            lanReadmission.complete(reservation, now, false)
            return
        }
        val deadline = now + 4000L
        val candidate = LanReadmissionCandidate(
            client, Request.Builder().url(endpoint).build(), root,
            { SystemClock.elapsedRealtime() }, deadline, { task -> main.post { task() } },
            onPrepared = { pending ->
                val t = SystemClock.elapsedRealtime()
                if (readmissionCandidate !== pending || !healthyRelay(relay, t) ||
                    !lanReadmission.isFresh(reservation, t) || !LanEndpoint.isOnLocalSubnet(address)) {
                    if (readmissionCandidate === pending) cancelReadmission() else pending.cancel()
                } else if (!pending.commit() && readmissionCandidate === pending) cancelReadmission()
            },
            onReady = { pending, key ->
                val t = SystemClock.elapsedRealtime()
                if (readmissionCandidate !== pending || !healthyRelay(relay, t) ||
                    !lanReadmission.isFresh(reservation, t) || !LanEndpoint.isOnLocalSubnet(address)) {
                    key.fill(0)
                    if (readmissionCandidate === pending) cancelReadmission() else pending.cancel()
                } else {
                    val sequence = LanSequence()
                    val listener = sessionListener(true, null, null, sequence, transportVideo)
                    val promoted = pending.promote(listener)
                    if (promoted == null) {
                        key.fill(0)
                        cancelReadmission()
                    } else {
                        // Main-thread linearization: retire WSS identity before its
                        // close callbacks; clear permissions and decoder generation.
                        readmissionCandidate = null
                        readmissionReservation = null
                        lanReadmission.complete(reservation, t, true)
                        stopWatchdog()
                        udpProbe?.stop()
                        udpProbe = null
                        udpAttempted = false
                        udpLastFrameAtMs = 0L
                        receivingFrames = false
                        hostAllowsInput = null
                        hostAllowsClipboard = null
                        clearLiveDiagnostics()
                        h264Active = false
                        frame = null
                        rttMs = null
                        lastPingTimeMs = 0L
                        avcDecoder.reset()
                        usingLan = true
                        lanAdvertised = true
                        currentLanAddress = address
                        lanBusinessKey = key
                        lanSequence = sequence
                        lanVerified = true
                        lanFailureHint = null
                        online = true
                        socket = promoted
                        transport = "局域网 P2P 直连"
                        status = "局域网已恢复 · 等待已验证画面"
                        relay.cancel()
                        startWatchdog()
                        startLanHeartbeat(promoted, address)
                    }
                }
            },
            onFailed = { pending, _ ->
                if (readmissionCandidate === pending) cancelReadmission()
            },
        )
        readmissionCandidate = candidate
        readmissionReservation = reservation
        candidate.start()
        main.postDelayed({ if (readmissionCandidate === candidate) candidate.expire() },
            (deadline - SystemClock.elapsedRealtime()).coerceAtLeast(0L))
    }

    private fun onNetworkChanged(change: NetworkChangePolicy.Change) {
        if (closed || !requested) return
        networkChanges++
        cancelReadmission()
        lanReadmission.networkChanged(SystemClock.elapsedRealtime())
        val ws = socket
        val address = currentLanAddress
        if (usingLan && lanAdvertised && ws != null && address != null) {
            // Confirm twice. A default-route handoff does not itself prove
            // the LAN path is gone; the existing heartbeat remains authoritative.
            if (!LanEndpoint.isOnLocalSubnet(address)) main.postDelayed({
                if (requested && socket === ws && usingLan &&
                    !LanEndpoint.isOnLocalSubnet(address)) {
                    if (lanVerified) failVerifiedLan(ws, "lan_subnet_changed")
                    else {
                        socket = null
                        lanBusinessKey = null
                        lanSequence = null
                        ws.cancel()
                        handleLanFailure("lan_subnet_changed")
                    }
                }
            }, 1000L)
        } else if (change.available && !usingLan && socket == null && retryTask != null) {
            // Wake a pending retry on an actual new usable default network.
            // Never interrupt a healthy WSS or bypass explicit disconnect.
            networkRetryWakes++
            openConnection()
        }
    }

    private fun scheduleRetry(reason: String) {
        if (!requested) return
        relayRetryEvents++
        retryTask?.let { main.removeCallbacks(it) }
        val waitMs = ReconnectPolicy.delayMs(retryAttempt)
        retryAttempt = (retryAttempt + 1).coerceAtMost(31)
        transport = "VPS WSS 中转 · 等待重连"
        status = "$reason · ${waitMs / 1000} 秒后重连"
        retryTask = object : Runnable {
            override fun run() {
                if (retryTask !== this) return
                retryTask = null
                if (LanAttemptPolicy.canOpen(requested, socket != null)) openConnection()
            }
        }.also { main.postDelayed(it, waitMs) }
    }

    private fun sendVideoControl(message:JSONObject) {
        val key=businessKey() ?: return
        val ws=socket ?: return
        if(!requested || !online) return
        ws.send(encryptBusiness(key,2,message.toString().toByteArray()).toByteString())
    }

    private fun startWatchdog() {
        stopWatchdog()
        lastFrameAtMs = SystemClock.elapsedRealtime()
        sampleAt=lastFrameAtMs
        sampleFrames=renderedAvcFrames
        sampleBytes=receivedAvcBytes
        sampleDrops=if(hasVideoSurface) avcDecoder.droppedFrames else 0
        renderedFps=0.0
        measuredBps=0
        fullHdPolicy.reset()
        hostFullHdNegotiation=false
        avcRequested=hasVideoSurface
        if(avcRequested) sendVideoControl(JSONObject().put("type","video_codec").put("codec","h264"))
        watchdogTask = object : Runnable {
            override fun run() {
                if (watchdogTask !== this || !requested || !online) return
                if (SystemClock.elapsedRealtime() - lastFrameAtMs > 20_000L) {
                    status = "画面超过 20 秒未更新，正在恢复连接"
                    if (usingLan) lanFailureHint = "video_watchdog"
                    socket?.cancel()
                    return
                }
                if(avcRequested && hasVideoSurface) {
                    val now=SystemClock.elapsedRealtime()
                    val elapsed=(now-sampleAt).coerceAtLeast(1)
                    if (fullHdPolicy.expired(now)) {
                        if (fullHdPolicy.downgrade()) {
                            // Keep a working 720p decoder if the host already fell back.
                            if(h264Width>1280 || h264Height>720) {
                                receivingFrames=false
                                avcDecoder.reset()
                            }
                            sendVideoControl(JSONObject().put("type","video_codec").put("codec","h264"))
                        }
                    } else if (hostFullHdNegotiation && h264Active && receivingFrames &&
                        fullHdPolicy.canAttempt() && localFullHdCapable &&
                        fullHdPolicy.request(now)) {
                        sendVideoControl(JSONObject().put("type","video_codec").put("codec","h264_1080p"))
                    }
                    val drops=avcDecoder.droppedFrames
                    renderedFps=if(h264Active && receivingFrames) (renderedAvcFrames-sampleFrames)*1000.0/elapsed else 0.0
                    measuredBps=if(h264Active && receivingFrames) (receivedAvcBytes-sampleBytes)*8000L/elapsed else 0L
                    val rtt=rttMs
                    if(h264Active && rtt!=null) sendVideoControl(JSONObject()
                        .put("type","video_feedback").put("rtt_ms",rtt)
                        .put("rendered_fps",renderedFps).put("decoder_drops",(drops-sampleDrops).coerceAtLeast(0)))
                    sampleAt=now
                    sampleFrames=renderedAvcFrames
                    sampleBytes=receivedAvcBytes
                    sampleDrops=drops
                }
                if (!usingLan) {
                    val now = SystemClock.elapsedRealtime()
                    val relay = socket
                    if (relay == null || !healthyRelay(relay, now)) {
                        cancelReadmission()
                        lanReadmission.observe(now, false, emptyList())
                    }
                    sendPing()
                }
                main.postDelayed(this, 5_000L)
            }
        }.also { main.postDelayed(it, 5_000L) }
    }

    // Independent of frame delivery and WebSocket transport-level Ping/Pong.
    // Only a matching authenticated business PONG re-arms the deadline.
    private fun startLanHeartbeat(ws: WebSocket, address: String) {
        lanHeartbeat.reset(SystemClock.elapsedRealtime())
        var nextSubnetCheck = SystemClock.elapsedRealtime() + 4000L
        var subnetMisses = 0
        lanHeartbeatTask = object : Runnable {
            override fun run() {
                if (lanHeartbeatTask !== this || socket !== ws || !requested || !lanVerified) return
                val now = SystemClock.elapsedRealtime()
                if (lanHeartbeat.expired(now)) {
                    failVerifiedLan(ws, "lan_heartbeat_timeout")
                    return
                }
                if (lanAdvertised && now >= nextSubnetCheck) {
                    nextSubnetCheck = now + 4000L
                    subnetMisses = if (LanEndpoint.isOnLocalSubnet(address)) 0 else subnetMisses + 1
                    if (subnetMisses >= 2) {
                        failVerifiedLan(ws, "lan_subnet_changed")
                        return
                    }
                }
                val ping = lanHeartbeat.poll(now)
                if (ping != null) {
                    val key = lanBusinessKey
                    val sent = try {
                        key != null && ws.send(encryptBusiness(key, 8,
                            ByteBuffer.allocate(8).putLong(ping.token).array()).toByteString())
                    } catch (_: Exception) { false }
                    if (!sent) {
                        failVerifiedLan(ws, "lan_heartbeat_send_failed")
                        return
                    }
                }
                main.postDelayed(this, 1000L)
            }
        }.also { main.post(it) }
    }

    private fun failVerifiedLan(ws: WebSocket, reason: String) {
        if (socket !== ws || !requested || !lanVerified) return
        // Retire the socket before cancel. Do not wait for asynchronous failure.
        socket = null
        lanBusinessKey = null
        lanSequence = null
        online = false
        receivingFrames = false
        hostAllowsInput = null
        hostAllowsClipboard = null
        clearLiveDiagnostics()
        rttMs = null
        lastPingTimeMs = 0L
        frame = null
        h264Active = false
        avcDecoder.reset()
        stopWatchdog()
        ws.cancel()
        handleLanFailure(reason)
    }

    private fun startUdpPunch() {
        val ws = socket ?: return
        val key = secret?.copyOf() ?: return
        if (!requested || !online || usingLan || activeToken.length < 32 || udpAttempted) return
        udpAttempted = true
        udpPreviewFrames = 0
        udpLastFrameAtMs = 0L
        udpReport = "已申请双方加密 UDP 连通性探测"
        udpProbe?.stop()
        val probe = UdpPunchProbe(
            relayUrl = activeEndpoint.substringBeforeLast("/relay?"),
            secret = key,
            signal = { json ->
                main.post {
                    if (requested && socket === ws) {
                        try { ws.send(encryptEnginePacket(key, 10,
                            json.toByteArray(Charsets.UTF_8)).toByteString()) }
                        catch (_: Exception) {}
                    }
                }
            },
            updateStatus = { result ->
                main.post {
                    if (requested && socket === ws) {
                        udpReport = result
                        if (result.contains("恢复 VPS") || result.contains("继续 VPS") ||
                            result.contains("保留 VPS")) {
                            udpLastFrameAtMs = 0L
                            transport = "VPS WSS 中转"
                        }
                    }
                }
            },
            onJpeg = { jpeg ->
                // Decode on UDP worker, update Compose state on the UI thread.
                val decoded = BitmapFactory.decodeByteArray(jpeg,0,jpeg.size)
                if (decoded != null) {
                    main.post {
                        if(requested && socket === ws && online) {
                            frame = decoded
                            emitFrame(jpeg, decoded.width, decoded.height)
                            if(h264Active) avcDecoder.reset()
                            h264Active = false
                            renderedFps = 0.0
                            measuredBps = 0L
                            receivingFrames = true
                            udpPreviewFrames++
                            udpLastFrameAtMs = SystemClock.elapsedRealtime()
                            lastFrameAtMs = udpLastFrameAtMs
                            transport = "公网加密 UDP JPEG 预览（输入继续走 WSS）"
                            status = "正在接收 UDP JPEG 画面 · VPS 备份已就绪"
                        }
                    }
                }
            },
        )
        udpProbe = probe
        try {
            // Send request BEFORE STUN: the host must create its receive queue
            // before our encrypted candidate can arrive through the relay.
            val request = JSONObject().put("v",1).put("type","request").toString()
            val accepted = ws.send(
                encryptEnginePacket(key,10,request.toByteArray(Charsets.UTF_8)).toByteString()
            )
            if (accepted) {
                probe.start()
            } else {
                udpReport = "UDP 请求未发送，保留 VPS"
                probe.stop()
            }
        } catch (_: Exception) {
            udpReport = "UDP 请求未发送，保留 VPS"
            probe.stop()
        }
    }

    private fun sendPing() {
        val currentKey = businessKey() ?: return
        val ws = socket ?: return
        if(!requested || !online) return
        val nonce = SecureRandom().nextLong()
        lastPingNonce = nonce
        lastPingTimeMs = SystemClock.elapsedRealtime()
        try {
            ws.send(encryptBusiness(currentKey, 8,
                ByteBuffer.allocate(8).putLong(nonce).array()).toByteString())
        } catch (_: Exception) {}
    }

    private fun probeSTUN(relay: String) {
        stunReport = "探测 UDP 3478 中..."
        Thread {
            val result = try {
                val outcome = StunProbe.probe(relay)
                "映射 ${outcome.address.address.hostAddress}:${outcome.address.port} · ${outcome.millis}ms（仅诊断）"
            } catch (_: Exception) {
                "探测失败：请检查 VPS 是否开放 UDP 3478"
            }
            main.post { stunReport = result }
        }.start()
    }

    // Host statistics describe the current socket, not a previous peer.
    // Session totals and LAN recovery history survive automatic reconnect.
    private fun clearLiveDiagnostics() {
        hostMetadataCurrent = false
        hostFullHdNegotiation=false
        fullHdPolicy.reset()
        videoTargetBps = 0
        videoTargetFps = 0
        videoEncoder = ""
        captureBackend = "unknown"
        captureAvgUs = 0L
        encodeAvgUs = 0L
        sendAvgUs = 0L
        freshFrames = 0L
        reusedFrames = 0L
        resizeCacheHits = 0L
        yuvCacheHits = 0L
        hostEncodedFrames = 0L
        hostLanVideoBytes = null
        hostRelayVideoBytes = null
        hostRelaySuppressedFrames = null
        hostLanVideoFrames = null
        hostRelayVideoFrames = null
        hostRelaySocketOpen = null
        hostRelaySocketOpens = null
        renderedFps = 0.0
        measuredBps = 0L
        clipboardResult = ""
    }

    private fun stopWatchdog() {
        watchdogTask?.let { main.removeCallbacks(it) }
        watchdogTask = null
        lanHeartbeatTask?.let { main.removeCallbacks(it) }
        lanHeartbeatTask = null
        lanHeartbeat.reset(SystemClock.elapsedRealtime())
    }

    private fun resetSessionTotals() {
        transportVideo = TransportVideo()
        relaySocketOpens = 0L
        relayRetryEvents = 0L
        networkChanges = 0L
        networkRetryWakes = 0L
        receivedFrames = 0L
        renderedAvcFrames = 0L
        receivedAvcBytes = 0L
    }

    private fun disconnect() {
        clearLiveDiagnostics()
        requested = false
        cancelReadmission()
        lanBusinessKey = null
        lanSequence = null
        udpProbe?.stop()
        udpProbe = null
        udpAttempted = false
        udpReport = "未执行公网 UDP 双端验证"
        udpLastFrameAtMs = 0L
        udpPreviewFrames = 0
        retryTask?.let { main.removeCallbacks(it) }
        retryTask = null
        stopWatchdog()
        val old = socket
        socket = null
        old?.close(1000, "User disconnected")
        online = false
        receivingFrames = false
        retryAttempt = 0
        frame = null
        h264Active = false
        avcDecoder.reset()
        resetSessionTotals()
        clipboardResult = ""
        monitorCount = 1
        selectedMonitor = 0
        pointerValid = false
        hostAllowsInput = null
        hostAllowsClipboard = null
        secret?.fill(0)
        secret = null
        activeToken = ""
        activeEndpoint = ""
        lanAdvertised = false
        currentLanAddress = null
        lanAttempt.reset()
        lanReadmission.reset()
        lanRecovery.reset()
        lanFailureHint = null
        usingLan = false
        lanVerified = false
        transport = "未连接"
        rttMs = null
        lastPingTimeMs = 0L
        status = "已断开，不会后台自动连接"
    }

    private fun selectMonitor(index: Int) {
        require(index in 0 until monitorCount) { "显示器索引无效" }
        send(JSONObject().put("type","monitor").put("index",index))
    }
    private fun sendScroll(delta: Int) {
        require(delta in -10..10 && delta != 0) { "滚动幅度无效" }
        send(JSONObject().put("type","scroll").put("delta",delta))
    }
    private fun sendClipboardSet(value: String) {
        require(value.isNotEmpty() && value.toByteArray(Charsets.UTF_8).size <= 4096) { "剪贴板内容无效" }
        send(JSONObject().put("type","clipboard_set").put("text",value))
    }

    private fun sendControl(type:String,x:Float,y:Float){
        val packet=JSONObject().put("type",type).put("x",x.toDouble()).put("y",y.toDouble())
        send(packet)
    }
    private fun sendControlText(text:String){
        // Text injection is explicit; never forward private IME/clipboard contents.
        require(text.isNotEmpty() && text.codePointCount(0,text.length) in 1..250) { "文字长度无效" }
        send(JSONObject().put("type","text").put("text",text))
        status="文字指令已发送，等待远端执行"
    }
    private fun sendRemoteKey(name:String){
        require(RemoteKeys.isAllowed(name)) { "按键不在白名单中" }
        send(JSONObject().put("type","key").put("key",name))
    }
    private fun send(data:JSONObject){
        val s = requireNotNull(businessKey()) { "尚未建立加密会话" }
        val ws = requireNotNull(socket) { "远程连接已断开" }
        require(requested && online && receivingFrames) { "等待已验证的远程画面" }
        when (data.optString("type")) {
            "move", "down", "up", "right_click", "scroll", "text", "key" ->
                require(hostAllowsInput == true) { "被控端尚未授权鼠标键盘" }
            "clipboard_get", "clipboard_set" ->
                require(hostAllowsClipboard == true) { "被控端尚未授权剪贴板" }
        }
        check(ws.send(encryptBusiness(s,2,data.toString().toByteArray()).toByteString())) { "控制指令发送失败" }
    }
    fun close() {
        if (closed) return
        pairingCancelled.set(true)
        disconnect()
        closed = true
        networkObserver.stop()
        main.removeCallbacks(statePump)
        avcDecoder.close()
        client.dispatcher.executorService.shutdown()
        client.connectionPool.evictAll()
    }
}

private fun decodeKey(hex:String):ByteArray{
    require(hex.length==64 && hex.all {it in '0'..'9'||it in 'a'..'f'||it in 'A'..'F'})
    return ByteArray(32){hex.substring(it*2,it*2+2).toInt(16).toByte()}
}
private fun sha256(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes)
private fun ByteArray.toHex():String=joinToString("") {"%02x".format(it.toInt() and 0xff)}
internal fun encryptEnginePacket(key:ByteArray,kind:Int,plain:ByteArray, suppliedNonce:ByteArray? = null):ByteArray{
    val nonce=suppliedNonce ?: ByteArray(12).also {SecureRandom().nextBytes(it)}
    val cipher=javax.crypto.Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(javax.crypto.Cipher.ENCRYPT_MODE,javax.crypto.spec.SecretKeySpec(key,"AES"),javax.crypto.spec.GCMParameterSpec(128,nonce))
    cipher.updateAAD(byteArrayOf(kind.toByte()))
    return byteArrayOf(kind.toByte())+nonce+cipher.doFinal(plain)
}
internal fun decryptEnginePacket(key:ByteArray,packet:ByteArray):Pair<Int,ByteArray>{
    require(packet.size in 30..(3*1024*1024))
    val cipher=javax.crypto.Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(javax.crypto.Cipher.DECRYPT_MODE,javax.crypto.spec.SecretKeySpec(key,"AES"),javax.crypto.spec.GCMParameterSpec(128,packet.copyOfRange(1,13)))
    cipher.updateAAD(packet.copyOfRange(0,1))
    return Pair(packet[0].toInt() and 0xff,cipher.doFinal(packet.copyOfRange(13,packet.size)))
}
