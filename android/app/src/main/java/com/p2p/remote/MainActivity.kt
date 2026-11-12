package com.p2p.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.graphics.BitmapFactory
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import java.nio.ByteBuffer
import android.view.Surface
import android.view.TextureView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import kotlin.math.min

private val Ink = Color(0xFF111827)
private val Panel = Color(0xFF202D44)
private val Accent = Color(0xFF6B9BFF)
private val Muted = Color(0xFFB5C1D5)

class MainActivity : ComponentActivity() {
    private val trustedStore by lazy { TrustedPairingStore(this) }
    private val main = Handler(Looper.getMainLooper())
    private val client = OkHttpClient.Builder().readTimeout(0,TimeUnit.MILLISECONDS).pingInterval(20,TimeUnit.SECONDS).build()
    private var socket: WebSocket? = null
    private var secret: ByteArray? = null
    private val decoding = AtomicBoolean(false)
    private var online by mutableStateOf(false)
    private var requested by mutableStateOf(false)
    private var receivingFrames by mutableStateOf(false)
    private var activeEndpoint = ""
    private var lanEndpoint = ""
    private var lanAttempted = false
    private var usingLan = false
    private var lanVerified = false
    private var transport by mutableStateOf("未连接")
    private var activeToken = ""
    private var retryAttempt = 0
    private var retryTask: Runnable? = null
    private var watchdogTask: Runnable? = null
    private var lastFrameAtMs = 0L
    private var status by mutableStateOf("等待配置服务器")
    private var stunReport by mutableStateOf("STUN UDP 尚未探测")
    private var rttMs by mutableStateOf<Long?>(null)
    private var lastPingNonce = 0L
    private var lastPingTimeMs = 0L
    private var frame by mutableStateOf<Bitmap?>(null)
    private var h264Active by mutableStateOf(false)
    private var h264Width by mutableIntStateOf(1280)
    private var h264Height by mutableIntStateOf(720)
    private val avcDecoder by lazy {
        H264SurfaceDecoder(
            onRendered = { main.post {
                if (requested && h264Active) {
                    receivingFrames = true
                    lastFrameAtMs = SystemClock.elapsedRealtime()
                    retryAttempt = 0
                    status = "已连接家庭电脑 · H.264 正在播放"
                }
            } },
            onDecoderError = { message -> main.post { if (requested) status = message } },
        )
    }
    private var clipboardResult by mutableStateOf("")
    private var monitorCount by mutableIntStateOf(1)
    private var selectedMonitor by mutableIntStateOf(0)
    private var pointerX by mutableFloatStateOf(0.5f)
    private var pointerY by mutableFloatStateOf(0.5f)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            var server by remember { mutableStateOf(
                getSharedPreferences("public_settings", MODE_PRIVATE).getString("relay_url", "wss://remote.example.com")
                    ?: "wss://remote.example.com"
            ) }
            var relayToken by remember { mutableStateOf("") }
            var lanIp by remember { mutableStateOf(
                getSharedPreferences("public_settings", MODE_PRIVATE)
                    .getString("lan_ipv4", "") ?: ""
            ) }
            var sessionKey by remember { mutableStateOf("") }
            var textToSend by remember { mutableStateOf("") }
            var clipboardToSend by remember { mutableStateOf("") }
            var savedPairAvailable by remember { mutableStateOf(trustedStore.hasSaved()) }
            MaterialTheme(colorScheme=darkColorScheme(primary=Accent, background=Ink,surface=Panel,onSurface=Color.White,onBackground=Color.White)) {
                Column(Modifier.fillMaxSize().background(Ink).verticalScroll(rememberScrollState()).padding(20.dp),verticalArrangement=Arrangement.spacedBy(15.dp)) {
                    Spacer(Modifier.height(8.dp))
                    Text("P2P  /  REMOTE",fontSize=12.sp,color=Accent,letterSpacing=2.sp)
                    Text("我的远程桌面",fontSize=29.sp,fontWeight=FontWeight.Bold)
                    Text("Android 控制端  ·  V0.8 测试版",fontSize=13.sp,color=Muted)
                    PanelCard {
                        Text("01   已授权的家庭电脑",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                        Text(if(savedPairAvailable)"已在 Android Keystore 加密保存一台电脑"
                            else "尚未保存可信配对",fontSize=13.sp,color=Muted)
                        Button(
                            onClick={
                                val saved=trustedStore.load()
                                if(saved==null){
                                    savedPairAvailable=false
                                    status="无法解锁已保存的配对。请重新在电脑端配对。"
                                }else{
                                    server=saved.relayUrl
                                    relayToken=saved.relayToken
                                    sessionKey=saved.sessionKey
                                    if(!requested) connect(server,relayToken,sessionKey,lanIp)
                                }
                            },
                            enabled=savedPairAvailable && !requested,
                            modifier=Modifier.fillMaxWidth()
                        ){Text("连接已保存的家庭电脑")}
                        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)){
                            Button(
                                onClick={
                                    try{
                                        trustedStore.save(TrustedPairingStore.Profile(
                                            server.trim(),relayToken.trim(),sessionKey.trim()
                                        ))
                                        savedPairAvailable=true
                                        status="已加密保存可信电脑，今后可直接连接"
                                    }catch(_:Exception){
                                        status="保存失败：请检查 HTTPS/WSS 地址、口令和 64 位密钥"
                                    }
                                },
                                enabled=!requested,
                                modifier=Modifier.weight(1f)
                            ){Text("加密保存配对")}
                            Button(
                                onClick={
                                    disconnect()
                                    trustedStore.clear()
                                    savedPairAvailable=false
                                    relayToken=""
                                    sessionKey=""
                                    status="已清除此手机的绑定；若要撤销远端访问请在 Windows 轮换密钥"
                                },
                                modifier=Modifier.weight(1f)
                            ){Text("清除此手机")}
                        }
                        Text("保存仅发生在你主动点击之后；不会自动连接或后台启动。",fontSize=12.sp,color=Muted)
                    }
                    PanelCard {
                        Text("02   手动连接家庭电脑",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                        Text("跨公网使用 VPS 中转，无需家庭固定 IP",fontSize=12.sp,color=Muted)
                        OutlinedTextField(value=server,onValueChange={server=it},modifier=Modifier.fillMaxWidth(),singleLine=true,label={Text("中转服务器 WSS 地址")})
                        OutlinedTextField(
                            value=lanIp,
                            onValueChange={lanIp=it.take(24)},
                            modifier=Modifier.fillMaxWidth(),
                            singleLine=true,
                            label={Text("可选：家中电脑的局域网 IPv4")},
                            placeholder={Text("例如 192.168.1.100")}
                        )
                        Text("局域网直连优先，跨公网使用 VPS；STUN 仅探测 NAT 映射，尚未实现 ICE/UDP 穿透。",
                            color=Muted,fontSize=12.sp)
                        Row(verticalAlignment=Alignment.CenterVertically,
                            horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            Button(onClick={probeSTUN(server)}) { Text("检测公网 UDP") }
                            Text(stunReport,color=Muted,fontSize=12.sp,modifier=Modifier.weight(1f))
                        }
                        OutlinedTextField(value=relayToken,onValueChange={relayToken=it},modifier=Modifier.fillMaxWidth(),singleLine=true,visualTransformation=PasswordVisualTransformation(),label={Text("中转服务器访问口令")})
                        OutlinedTextField(value=sessionKey,onValueChange={sessionKey=it},modifier=Modifier.fillMaxWidth(),singleLine=true,visualTransformation=PasswordVisualTransformation(),label={Text("Windows 显示的 64 位会话密钥")})
                        Button(onClick={ if (requested) disconnect() else connect(server, relayToken, sessionKey, lanIp) },modifier=Modifier.fillMaxWidth()) {
                            Text(if(requested)"停止连接 / 取消重试" else "连接电脑")
                        }
                        Text(status,color=if(receivingFrames)Color(0xFF72D9AB) else Muted,fontSize=13.sp)
                        Text("当前链路：$transport",color=Accent,fontSize=13.sp)
                        Text("端到端信令 RTT：${rttMs?.let{"$it ms"} ?: "等待响应"}",
                            color=Muted,fontSize=12.sp)
                        Text("只有主动点击加密保存才会存储可信配对；网络中断期间允许当前会话自动重连",color=Muted,fontSize=12.sp)
                    }
                    PanelCard {
                        Row(verticalAlignment=Alignment.CenterVertically) {
                            Text("03   远程画面",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                            Spacer(Modifier.weight(1f))
                            Text(if(receivingFrames)"● 正在接收画面" else if(online)"○ 等待家庭电脑" else "○ 未连接",fontSize=12.sp,color=if(receivingFrames)Color(0xFF72D9AB) else Muted)
                        }
                        val bitmap = frame
                        val displayWidth = if(h264Active) h264Width else (bitmap?.width ?: 1280)
                        val displayHeight = if(h264Active) h264Height else (bitmap?.height ?: 720)
                        Box(
                            Modifier.fillMaxWidth().height(260.dp)
                                .background(Color(0xFF0F1625),RoundedCornerShape(12.dp))
                                .pointerInput(online, h264Active, displayWidth, displayHeight) {
                                    if (!online) return@pointerInput
                                    awaitEachGesture {
                                        val down = awaitFirstDown(requireUnconsumed=false)
                                        val first = coordinate(
                                            down.position,size.width.toFloat(),size.height.toFloat(),
                                            displayWidth,displayHeight
                                        )
                                        pointerX=first.first; pointerY=first.second
                                        sendControl("down",first.first,first.second)
                                        do {
                                            val event=awaitPointerEvent()
                                            val change=event.changes.firstOrNull() ?: break
                                            val position=coordinate(
                                                change.position,size.width.toFloat(),size.height.toFloat(),
                                                displayWidth,displayHeight
                                            )
                                            pointerX=position.first; pointerY=position.second
                                            if(change.pressed) sendControl("move",position.first,position.second)
                                            else {
                                                sendControl("up",position.first,position.second)
                                                break
                                            }
                                        } while(true)
                                    }
                                },
                            contentAlignment=Alignment.Center
                        ) {
                            if(h264Active){
                                AndroidView(
                                    factory={ context ->
                                        TextureView(context).apply {
                                            surfaceTextureListener=object:TextureView.SurfaceTextureListener {
                                                override fun onSurfaceTextureAvailable(texture:SurfaceTexture,width:Int,height:Int) {
                                                    avcDecoder.attachSurface(Surface(texture))
                                                }
                                                override fun onSurfaceTextureSizeChanged(texture:SurfaceTexture,width:Int,height:Int) {}
                                                override fun onSurfaceTextureDestroyed(texture:SurfaceTexture):Boolean {
                                                    avcDecoder.attachSurface(null)
                                                    return true
                                                }
                                                override fun onSurfaceTextureUpdated(texture:SurfaceTexture) {}
                                            }
                                        }
                                    },
                                    update={ view ->
                                        view.post {
                                            if(view.width>0 && view.height>0 && displayWidth>0 && displayHeight>0){
                                                val factor=min(view.width.toFloat()/displayWidth,view.height.toFloat()/displayHeight)
                                                val xScale=displayWidth*factor/view.width
                                                val yScale=displayHeight*factor/view.height
                                                val matrix=Matrix()
                                                matrix.setScale(xScale,yScale,view.width/2f,view.height/2f)
                                                view.setTransform(matrix)
                                            }
                                        }
                                    },
                                    modifier=Modifier.fillMaxSize()
                                )
                                if(!receivingFrames) Text("H.264 解码中；首次关键帧可能需要数秒",color=Muted,fontSize=12.sp)
                            }else if(bitmap!=null){
                                Image(bitmap=bitmap.asImageBitmap(),contentDescription="授权的 Windows 桌面",
                                    modifier=Modifier.fillMaxSize(),contentScale=androidx.compose.ui.layout.ContentScale.Fit)
                            }else{
                                Text("连接后在这里显示电脑屏幕",color=Muted,fontSize=13.sp)
                            }
                        }
                        Text(if(h264Active)"视频模式：H.264 · Android MediaCodec"
                            else "视频模式：JPEG（兼容模式）",color=Muted,fontSize=12.sp)
                        Text("触摸 = 左键 · 滑动 = 拖拽（电脑端需要授权）",color=Muted,fontSize=12.sp)
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(7.dp)) {
                            Text("显示器 ${selectedMonitor+1}/$monitorCount",fontSize=13.sp,color=Muted,modifier=Modifier.weight(1f))
                            Button(onClick={selectMonitor((selectedMonitor-1+monitorCount)%monitorCount)},enabled=online && monitorCount>1) {Text("上一屏")}
                            Button(onClick={selectMonitor((selectedMonitor+1)%monitorCount)},enabled=online && monitorCount>1) {Text("下一屏")}
                        }
                        Row(horizontalArrangement=Arrangement.spacedBy(6.dp)){
                            Button(onClick={sendControl("right_click",pointerX,pointerY)},enabled=online,modifier=Modifier.weight(1f)){Text("右键")}
                            Button(onClick={sendScroll(-3)},enabled=online,modifier=Modifier.weight(1f)){Text("上滚")}
                            Button(onClick={sendScroll(3)},enabled=online,modifier=Modifier.weight(1f)){Text("下滚")}
                        }
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            OutlinedTextField(value=textToSend,onValueChange={textToSend=it},modifier=Modifier.weight(1f),singleLine=true,label={Text("发送文字到电脑")})
                            Button(onClick={sendControlText(textToSend);textToSend=""},enabled=online && textToSend.isNotBlank()){Text("发送")}
                        }
                    }
                    PanelCard {
                        Text("04   按需传递剪贴板文字",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
                        Text("电脑端须单独授权。不会自动监听或同步任何剪贴板内容。",fontSize=12.sp,color=Muted)
                        OutlinedTextField(value=clipboardToSend,onValueChange={clipboardToSend=it.take(2048)},
                            modifier=Modifier.fillMaxWidth(),label={Text("输入要放进电脑剪贴板的文字")},maxLines=3)
                        Row(horizontalArrangement=Arrangement.spacedBy(7.dp)){
                            Button(onClick={sendClipboardSet(clipboardToSend)},
                                enabled=online && clipboardToSend.isNotEmpty() && clipboardToSend.toByteArray(Charsets.UTF_8).size<=4096,
                                modifier=Modifier.weight(1f)){Text("发送至电脑")}
                            Button(onClick={send(JSONObject().put("type","clipboard_get"))},enabled=online,
                                modifier=Modifier.weight(1f)){Text("读取电脑文字")}
                        }
                        if(clipboardResult.isNotEmpty()){
                            Text(clipboardResult,color=Muted,fontSize=12.sp,maxLines=3)
                            Button(onClick={
                                val manager=getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                manager.setPrimaryClip(ClipData.newPlainText("P2P Remote",clipboardResult))
                            }){Text("复制至手机剪贴板")}
                        }
                    }
                    PanelCard {
                        Text("05   安全提示",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
                        Text("桌面画面采用 AES-256-GCM 端到端加密；中转服务器无法解密图像。首次测试请仅在你自己的电脑上使用。",fontSize=13.sp,color=Muted,lineHeight=20.sp)
                    }
                    Spacer(Modifier.height(15.dp))
                }
            }
        }
    }

    @Composable private fun PanelCard(content:@Composable ColumnScope.()->Unit) {
        Column(Modifier.fillMaxWidth().background(Panel,RoundedCornerShape(18.dp)).padding(17.dp),verticalArrangement=Arrangement.spacedBy(12.dp),content=content)
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
        lanEndpoint = LanEndpoint.url(lanIp) ?: ""
        lanAttempted = false
        usingLan = false
        lanVerified = false
        transport = "连接中"
        activeToken = relayToken
        secret = key
        requested = true
        retryAttempt = 0
        getSharedPreferences("public_settings", MODE_PRIVATE).edit()
            .putString("relay_url", normalized)
            .putString("lan_ipv4", lanIp.trim()).apply()
        openConnection()
    }

    private fun openConnection() {
        if (!requested) return
        val tryLan = !lanAttempted && lanEndpoint.isNotEmpty()
        if (tryLan) lanAttempted = true
        usingLan = tryLan
        lanVerified = false
        transport = if(tryLan) "局域网加密直连 · 验证中" else "VPS WSS 中转"
        status = if(tryLan) "尝试局域网直连..." else
            if (retryAttempt == 0) "正在连接中转服务器..." else "正在重新连接中转服务器..."
        val request = try {
            val builder = Request.Builder().url(if(tryLan) lanEndpoint else activeEndpoint)
            if(!tryLan) builder.header("Authorization", "Bearer $activeToken")
            builder.build()
        } catch (_: Exception) {
            status = "连接地址无效"
            if(tryLan){
                usingLan=false
                openConnection()
            }else{
                requested = false
                secret = null
                activeToken = ""
            }
            return
        }
        val ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                main.post {
                    if (socket === webSocket && requested) {
                        receivingFrames = false
                        if(tryLan) {
                            status = "局域网连接建立 · 正在验证会话密钥"
                            val currentKey = secret
                            if(currentKey == null ||
                                !webSocket.send(
                                    encryptPacket(
                                        currentKey, 6, "P2P-LAN-HELLO-v1".toByteArray(Charsets.UTF_8)
                                    ).toByteString()
                                )
                            ) {
                                webSocket.cancel()
                            } else {
                                main.postDelayed({
                                    if(socket === webSocket && requested && !lanVerified) {
                                        status = "局域网验证超时，准备切换 VPS"
                                        webSocket.cancel()
                                    }
                                }, 4000L)
                            }
                        }else{
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
                if (!decoding.compareAndSet(false, true)) return
                try {
                    val key = secret ?: return
                    val original = decryptPacket(key, bytes.toByteArray())
                    if (tryLan && !lanVerified && original.first != 7) return
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
                        7 -> {
                            if(tryLan &&
                                original.second.contentEquals(
                                    "P2P-LAN-READY-v1".toByteArray(Charsets.UTF_8)
                                )
                            ){
                                main.post {
                                    if(socket===webSocket && requested){
                                        lanVerified = true
                                        online = true
                                        receivingFrames = false
                                        transport = "局域网 P2P 直连"
                                        status = "已通过局域网加密验证 · 等待画面"
                                        startWatchdog()
                                    }
                                }
                            }
                        }
                        1 -> {
                            val decoded = BitmapFactory.decodeByteArray(original.second, 0, original.second.size)
                            if (decoded != null) main.post {
                                if (socket === webSocket && requested) {
                                    if(h264Active) avcDecoder.reset()
                                    frame = decoded
                                    h264Active = false
                                    receivingFrames = true
                                    lastFrameAtMs = SystemClock.elapsedRealtime()
                                    retryAttempt = 0
                                    status = "已连接家庭电脑 · 正在接收画面"
                                }
                            }
                        }
                        5 -> {
                            val accessUnit = AvcPayload.parse(original.second)
                            if(accessUnit != null) {
                                main.post {
                                    if(socket===webSocket && requested){
                                        h264Width=accessUnit.width
                                        h264Height=accessUnit.height
                                        h264Active=true
                                        frame=null
                                    }
                                }
                                avcDecoder.offer(accessUnit.bytes)
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
                            main.post {
                                if(socket===webSocket && requested){ monitorCount=count;selectedMonitor=index }
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
                    socket = null
                    online = false
                    rttMs = null
                    lastPingTimeMs = 0L
                    receivingFrames = false
                    frame = null
                    h264Active = false
                    avcDecoder.reset()
                    stopWatchdog()
                    if(tryLan) {
                        lanVerified = false
                        usingLan = false
                        transport = "局域网不可达，已切换 VPS"
                        status = "局域网直连失败，改走 VPS 中转"
                        if(activeToken.length >= 32){
                            openConnection()
                        }else{
                            requested=false
                            secret=null
                            transport="局域网直连失败"
                            status="局域网直连不可用，且未配置 VPS 访问口令"
                        }
                    } else if (response?.code == 401 || response?.code == 403) {
                        requested = false
                        secret = null
                        activeToken = ""
                        status = "中转访问口令错误（HTTP ${response.code}），已停止重试"
                    } else {
                        scheduleRetry("连接失败：${response?.code ?: error.localizedMessage ?: "网络错误"}")
                    }
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                main.post {
                    if (socket !== webSocket || !requested) return@post
                    socket = null
                    online = false
                    rttMs = null
                    lastPingTimeMs = 0L
                    receivingFrames = false
                    frame = null
                    h264Active = false
                    avcDecoder.reset()
                    stopWatchdog()
                    if (tryLan) {
                        lanVerified = false
                        usingLan = false
                        transport = "局域网已断开，切换 VPS"
                        status = "局域网连接断开，改走 VPS 中转"
                        if(activeToken.length >= 32){
                            openConnection()
                        }else{
                            requested=false
                            secret=null
                            transport="局域网连接中断"
                            status="局域网直连已断开，且未配置 VPS 回退"
                        }
                    } else {
                        scheduleRetry("连接已断开（$code）")
                    }
                }
            }
        })
        socket = ws
    }

    private fun scheduleRetry(reason: String) {
        if (!requested) return
        retryTask?.let { main.removeCallbacks(it) }
        val waitMs = ReconnectPolicy.delayMs(retryAttempt)
        retryAttempt = (retryAttempt + 1).coerceAtMost(31)
        status = "$reason · ${waitMs / 1000} 秒后重连"
        retryTask = Runnable {
            retryTask = null
            if (requested) openConnection()
        }.also { main.postDelayed(it, waitMs) }
    }

    private fun startWatchdog() {
        stopWatchdog()
        lastFrameAtMs = 0L
        watchdogTask = object : Runnable {
            override fun run() {
                if (!requested || !online) return
                if (receivingFrames && SystemClock.elapsedRealtime() - lastFrameAtMs > 20_000L) {
                    status = "画面超过 20 秒未更新，正在恢复连接"
                    socket?.cancel()
                    return
                }
                sendPing()
                main.postDelayed(this, 5_000L)
            }
        }.also { main.postDelayed(it, 5_000L) }
    }

    private fun sendPing() {
        val currentKey = secret ?: return
        val ws = socket ?: return
        if(!requested || !online) return
        val nonce = SecureRandom().nextLong()
        lastPingNonce = nonce
        lastPingTimeMs = SystemClock.elapsedRealtime()
        try {
            ws.send(encryptPacket(currentKey, 8,
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

    private fun stopWatchdog() {
        watchdogTask?.let { main.removeCallbacks(it) }
        watchdogTask = null
    }

    private fun disconnect() {
        requested = false
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
        clipboardResult = ""
        monitorCount = 1
        selectedMonitor = 0
        secret = null
        activeToken = ""
        activeEndpoint = ""
        lanEndpoint = ""
        lanAttempted = false
        usingLan = false
        lanVerified = false
        transport = "未连接"
        rttMs = null
        lastPingTimeMs = 0L
        status = "已断开，不会后台自动连接"
    }

    private fun selectMonitor(index: Int) {
        if(index < 0 || index >= monitorCount) return
        send(JSONObject().put("type","monitor").put("index",index))
    }
    private fun sendScroll(delta: Int) {
        if(delta !in -10..10 || delta == 0) return
        send(JSONObject().put("type","scroll").put("delta",delta))
    }
    private fun sendClipboardSet(value: String) {
        if(value.isEmpty() || value.toByteArray(Charsets.UTF_8).size > 4096) return
        send(JSONObject().put("type","clipboard_set").put("text",value))
    }

    private fun sendControl(type:String,x:Float,y:Float){
        val packet=JSONObject().put("type",type).put("x",x.toDouble()).put("y",y.toDouble())
        send(packet)
    }
    private fun sendControlText(text:String){if(text.length in 1..250)send(JSONObject().put("type","text").put("text",text))}
    private fun send(data:JSONObject){
        val s=secret ?:return
        val ws=socket?:return
        if(!online)return
        try{ws.send(encryptPacket(s,2,data.toString().toByteArray()).toByteString())}catch(_:Exception){}
    }
    override fun onDestroy(){
        disconnect()
        avcDecoder.close()
        client.dispatcher.executorService.shutdown()
        super.onDestroy()
    }
}

private fun coordinate(point:Offset,viewW:Float,viewH:Float,imageW:Int,imageH:Int):Pair<Float,Float>{
    val factor=min(viewW/imageW.toFloat(),viewH/imageH.toFloat())
    val drawnW=imageW*factor;val drawnH=imageH*factor
    return Pair(((point.x-(viewW-drawnW)/2)/drawnW).coerceIn(0f,1f),((point.y-(viewH-drawnH)/2)/drawnH).coerceIn(0f,1f))
}
private fun decodeKey(hex:String):ByteArray{
    require(hex.length==64 && hex.all {it in '0'..'9'||it in 'a'..'f'||it in 'A'..'F'})
    return ByteArray(32){hex.substring(it*2,it*2+2).toInt(16).toByte()}
}
private fun sha256(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes)
private fun ByteArray.toHex():String=joinToString("") {"%02x".format(it.toInt() and 0xff)}
private fun encryptPacket(key:ByteArray,kind:Int,plain:ByteArray):ByteArray{
    val nonce=ByteArray(12).also {SecureRandom().nextBytes(it)}
    val cipher=Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE,SecretKeySpec(key,"AES"),GCMParameterSpec(128,nonce))
    cipher.updateAAD(byteArrayOf(kind.toByte()))
    return byteArrayOf(kind.toByte())+nonce+cipher.doFinal(plain)
}
private fun decryptPacket(key:ByteArray,packet:ByteArray):Pair<Int,ByteArray>{
    require(packet.size in 30..(3*1024*1024))
    val cipher=Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE,SecretKeySpec(key,"AES"),GCMParameterSpec(128,packet.copyOfRange(1,13)))
    cipher.updateAAD(packet.copyOfRange(0,1))
    return Pair(packet[0].toInt() and 0xff,cipher.doFinal(packet.copyOfRange(13,packet.size)))
}
