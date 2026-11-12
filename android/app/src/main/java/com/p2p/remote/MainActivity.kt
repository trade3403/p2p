package com.p2p.remote

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
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
    private val main = Handler(Looper.getMainLooper())
    private val client = OkHttpClient.Builder().readTimeout(0,TimeUnit.MILLISECONDS).pingInterval(20,TimeUnit.SECONDS).build()
    private var socket: WebSocket? = null
    private var secret: ByteArray? = null
    private val decoding = AtomicBoolean(false)
    private var online by mutableStateOf(false)
    private var requested by mutableStateOf(false)
    private var receivingFrames by mutableStateOf(false)
    private var activeEndpoint = ""
    private var activeToken = ""
    private var retryAttempt = 0
    private var retryTask: Runnable? = null
    private var watchdogTask: Runnable? = null
    private var lastFrameAtMs = 0L
    private var status by mutableStateOf("等待配置服务器")
    private var frame by mutableStateOf<Bitmap?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            var server by remember { mutableStateOf(
                getSharedPreferences("public_settings", MODE_PRIVATE).getString("relay_url", "wss://remote.example.com")
                    ?: "wss://remote.example.com"
            ) }
            var relayToken by remember { mutableStateOf("") }
            var sessionKey by remember { mutableStateOf("") }
            var textToSend by remember { mutableStateOf("") }
            MaterialTheme(colorScheme=darkColorScheme(primary=Accent, background=Ink,surface=Panel,onSurface=Color.White,onBackground=Color.White)) {
                Column(Modifier.fillMaxSize().background(Ink).verticalScroll(rememberScrollState()).padding(20.dp),verticalArrangement=Arrangement.spacedBy(15.dp)) {
                    Spacer(Modifier.height(8.dp))
                    Text("P2P  /  REMOTE",fontSize=12.sp,color=Accent,letterSpacing=2.sp)
                    Text("我的远程桌面",fontSize=29.sp,fontWeight=FontWeight.Bold)
                    Text("Android 控制端  ·  V0.3 测试版",fontSize=13.sp,color=Muted)
                    PanelCard {
                        Text("01   连接家庭电脑",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                        Text("跨公网使用 VPS 中转，无需家庭固定 IP",fontSize=12.sp,color=Muted)
                        OutlinedTextField(value=server,onValueChange={server=it},modifier=Modifier.fillMaxWidth(),singleLine=true,label={Text("中转服务器 WSS 地址")})
                        OutlinedTextField(value=relayToken,onValueChange={relayToken=it},modifier=Modifier.fillMaxWidth(),singleLine=true,visualTransformation=PasswordVisualTransformation(),label={Text("中转服务器访问口令")})
                        OutlinedTextField(value=sessionKey,onValueChange={sessionKey=it},modifier=Modifier.fillMaxWidth(),singleLine=true,visualTransformation=PasswordVisualTransformation(),label={Text("Windows 显示的 64 位会话密钥")})
                        Button(onClick={ if (requested) disconnect() else connect(server, relayToken, sessionKey) },modifier=Modifier.fillMaxWidth()) {
                            Text(if(requested)"停止连接 / 取消重试" else "连接电脑")
                        }
                        Text(status,color=if(receivingFrames)Color(0xFF72D9AB) else Muted,fontSize=13.sp)
                        Text("已记住服务器地址 · 不保存访问口令或会话密钥 · 网络中断后自动重连",color=Muted,fontSize=12.sp)
                    }
                    PanelCard {
                        Row(verticalAlignment=Alignment.CenterVertically) {
                            Text("02   远程画面",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                            Spacer(Modifier.weight(1f))
                            Text(if(receivingFrames)"● 正在接收画面" else if(online)"○ 等待家庭电脑" else "○ 未连接",fontSize=12.sp,color=if(receivingFrames)Color(0xFF72D9AB) else Muted)
                        }
                        val bitmap = frame
                        Box(Modifier.fillMaxWidth().height(260.dp).background(Color(0xFF0F1625),RoundedCornerShape(12.dp)),contentAlignment=Alignment.Center) {
                            if(bitmap!=null) {
                                Image(bitmap=bitmap.asImageBitmap(),contentDescription="授权的 Windows 桌面",modifier=Modifier.fillMaxSize().pointerInput(bitmap.width,bitmap.height,online){
                                    if(!online)return@pointerInput
                                    awaitEachGesture {
                                        val down=awaitFirstDown(requireUnconsumed=false)
                                        val first=coordinate(down.position,size.width.toFloat(),size.height.toFloat(),bitmap.width,bitmap.height)
                                        sendControl("down",first.first,first.second)
                                        do {
                                            val e=awaitPointerEvent()
                                            val change=e.changes.firstOrNull() ?: break
                                            val p=coordinate(change.position,size.width.toFloat(),size.height.toFloat(),bitmap.width,bitmap.height)
                                            if(change.pressed) sendControl("move",p.first,p.second)
                                            else {sendControl("up",p.first,p.second);break}
                                        }while(true)
                                    }
                                },contentScale=androidx.compose.ui.layout.ContentScale.Fit)
                            } else Text("连接后在这里显示电脑屏幕",color=Muted,fontSize=13.sp)
                        }
                        Text("触摸 = 左键 · 滑动 = 拖拽（电脑端需要授权）",color=Muted,fontSize=12.sp)
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            OutlinedTextField(value=textToSend,onValueChange={textToSend=it},modifier=Modifier.weight(1f),singleLine=true,label={Text("发送文字到电脑")})
                            Button(onClick={sendControlText(textToSend);textToSend=""},enabled=online && textToSend.isNotBlank()){Text("发送")}
                        }
                    }
                    PanelCard {
                        Text("03   安全提示",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
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

    private fun connect(server: String, relayToken: String, keyText: String) {
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
        if (relayToken.length < 32) {
            status = "请填写至少 32 字符的服务器访问口令"
            return
        }
        val room = sha256("p2p-v02-room:".toByteArray(Charsets.UTF_8) + key).toHex()
        activeEndpoint = "$normalized/relay?room=$room&role=viewer"
        activeToken = relayToken
        secret = key
        requested = true
        retryAttempt = 0
        getSharedPreferences("public_settings", MODE_PRIVATE).edit()
            .putString("relay_url", normalized).apply()
        openConnection()
    }

    private fun openConnection() {
        if (!requested) return
        status = if (retryAttempt == 0) "正在连接中转服务器..." else "正在重新连接中转服务器..."
        val request = try {
            Request.Builder().url(activeEndpoint).header("Authorization", "Bearer $activeToken").build()
        } catch (_: Exception) {
            status = "中转服务器地址无效"
            requested = false
            secret = null
            activeToken = ""
            return
        }
        val ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                main.post {
                    if (socket === webSocket && requested) {
                        online = true
                        receivingFrames = false
                        status = "已连接中转服务器，等待家庭电脑画面"
                        startWatchdog()
                    }
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (socket !== webSocket || !requested) return
                if (!decoding.compareAndSet(false, true)) return
                try {
                    val key = secret ?: return
                    val original = decryptPacket(key, bytes.toByteArray())
                    if (original.first == 1) {
                        val decoded = BitmapFactory.decodeByteArray(original.second, 0, original.second.size)
                        if (decoded != null) main.post {
                            if (socket === webSocket && requested) {
                                frame = decoded
                                receivingFrames = true
                                lastFrameAtMs = SystemClock.elapsedRealtime()
                                retryAttempt = 0
                                status = "已连接家庭电脑 · 正在接收画面"
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
                    receivingFrames = false
                    frame = null
                    stopWatchdog()
                    if (response?.code == 401 || response?.code == 403) {
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
                    receivingFrames = false
                    frame = null
                    stopWatchdog()
                    scheduleRetry("连接已断开（$code）")
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
                main.postDelayed(this, 5_000L)
            }
        }.also { main.postDelayed(it, 5_000L) }
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
        secret = null
        activeToken = ""
        activeEndpoint = ""
        status = "已断开，不会后台自动连接"
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
    override fun onDestroy(){disconnect();client.dispatcher.executorService.shutdown();super.onDestroy()}
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
