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
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
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

// V0.15 visual system: deep navy surfaces, mint interaction accent and
// high-contrast typography shared by the pairing and remote-control screens.
private val Ink = Color(0xFF0A1220)
private val Panel = Color(0xFF142238)
private val Accent = Color(0xFF54D6C5)
private val Muted = Color(0xFFA8B8CC)
private val PanelRaised = Color(0xFF1A2B43)
private val OutlineSoft = Color(0xFF2B405B)

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
    private var udpProbe: UdpPunchProbe? = null
    private var udpAttempted by mutableStateOf(false)
    private var udpReport by mutableStateOf("未执行公网 UDP 双端验证")
    private var udpPreviewFrames by mutableIntStateOf(0)
    @Volatile private var udpLastFrameAtMs = 0L
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
    private var pointerValid by mutableStateOf(false)
    private var hostAllowsInput by mutableStateOf<Boolean?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            var server by remember { mutableStateOf(
                getSharedPreferences("public_settings", MODE_PRIVATE).getString("relay_url", "wss://p2ptest.aikissgo.com")
                    ?: "wss://p2ptest.aikissgo.com"
            ) }
            var relayToken by remember { mutableStateOf("") }
            var lanIp by remember { mutableStateOf(
                getSharedPreferences("public_settings", MODE_PRIVATE)
                    .getString("lan_ipv4", "") ?: ""
            ) }
            var sessionKey by remember { mutableStateOf("") }
            var deviceNumber by remember { mutableStateOf("") }
            var pairingCode by remember { mutableStateOf("") }
            var pairingBusy by remember { mutableStateOf(false) }
            var pairingStatus by remember { mutableStateOf("输入电脑显示的9位设备号和6位动态验证码") }
            var showLegacyPairing by remember { mutableStateOf(false) }
            val pairingCancelled = remember { AtomicBoolean(false) }
            var textToSend by remember { mutableStateOf("") }
            var clipboardToSend by remember { mutableStateOf("") }
            var savedPairAvailable by remember { mutableStateOf(trustedStore.hasSaved()) }
            var mobilePage by remember { mutableIntStateOf(0) }
            var showAdvancedNetwork by remember { mutableStateOf(false) }
            var remoteExpanded by remember { mutableStateOf(false) }
            var remoteZoom by remember { mutableFloatStateOf(1f) }
            var remotePanX by remember { mutableFloatStateOf(0f) }
            var remotePanY by remember { mutableFloatStateOf(0f) }
            val screenHeightDp = LocalConfiguration.current.screenHeightDp
            LaunchedEffect(selectedMonitor, online) {
                remoteZoom = 1f; remotePanX = 0f; remotePanY = 0f
            }
            MaterialTheme(colorScheme=darkColorScheme(
                primary=Accent, onPrimary=Color(0xFF06201E),
                secondary=Color(0xFFFFC58B), onSecondary=Color(0xFF24180B),
                background=Ink, onBackground=Color(0xFFF2F6FC),
                surface=Panel, onSurface=Color(0xFFF2F6FC),
                surfaceVariant=PanelRaised, onSurfaceVariant=Muted,
                outline=OutlineSoft
            )) {
                Scaffold(
                    containerColor=Ink,
                    bottomBar={
                        NavigationBar(containerColor=Panel, tonalElevation=8.dp) {
                            listOf("设备","远程桌面","设置").forEachIndexed { index, label ->
                                NavigationBarItem(
                                    selected=mobilePage==index,
                                    onClick={mobilePage=index},
                                    icon={Text(listOf("⌂","▣","⚙")[index],fontSize=19.sp,fontWeight=FontWeight.SemiBold)},
                                    label={Text(label,fontSize=11.sp,fontWeight=if(mobilePage==index) FontWeight.SemiBold else FontWeight.Normal)},
                                    alwaysShowLabel=true
                                )
                            }
                        }
                    }
                ) { contentPadding ->
                Column(
                    Modifier.fillMaxSize().background(Ink).padding(contentPadding)
                        .verticalScroll(rememberScrollState()).padding(horizontal=16.dp,vertical=12.dp),
                    verticalArrangement=Arrangement.spacedBy(15.dp)
                ) {
                    Row(verticalAlignment=Alignment.CenterVertically) {
                        Column(modifier=Modifier.weight(1f), verticalArrangement=Arrangement.spacedBy(5.dp)) {
                            Surface(
                                shape=RoundedCornerShape(8.dp),
                                color=Color(0xFF163B43)
                            ) {
                                Text("P2P REMOTE  /  V0.15",modifier=Modifier.padding(horizontal=10.dp,vertical=5.dp),
                                    fontSize=10.sp,color=Accent,letterSpacing=1.4.sp,fontWeight=FontWeight.Bold)
                            }
                            Text("远程桌面",fontSize=29.sp,fontWeight=FontWeight.Bold,color=Color(0xFFF2F6FC))
                            Text("安全连接并控制你的 Windows 设备",fontSize=13.sp,color=Muted)
                        }
                        Surface(
                            shape=RoundedCornerShape(12.dp),
                            color=if(online) Color(0xFF153E38) else PanelRaised,
                            border=androidx.compose.foundation.BorderStroke(1.dp,if(online) Color(0xFF28685B) else OutlineSoft)
                        ) {
                            Text(
                                if(online) "● 已连接" else "○ 未连接",
                                modifier=Modifier.padding(horizontal=12.dp, vertical=9.dp),
                                fontSize=11.sp,fontWeight=FontWeight.SemiBold,
                                color=if(online) Color(0xFF82E4C1) else Muted
                            )
                        }
                    }
                    // Persistent bottom navigation keeps the remote screen one tap away.
                    if (mobilePage==0) {
                    PanelCard {
                        Text("已授权的电脑",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
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
                        Text("添加或连接电脑",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                        Text("跨公网通过加密中转连接；首次配对需要被控电脑端确认。",fontSize=12.sp,color=Muted)
                        OutlinedTextField(value=server,onValueChange={server=it},modifier=Modifier.fillMaxWidth(),singleLine=true,label={Text("中转服务器 WSS 地址")})
                        TextButton(onClick={showAdvancedNetwork=!showAdvancedNetwork}) {
                            Text(if(showAdvancedNetwork) "收起高级网络设置" else "高级网络设置（LAN / STUN）")
                        }
                        if(showAdvancedNetwork) {
                            OutlinedTextField(
                                value=relayToken,
                                onValueChange={relayToken=it.take(512)},
                                modifier=Modifier.fillMaxWidth(),
                                singleLine=true,
                                visualTransformation=PasswordVisualTransformation(),
                                label={Text("私测中转访问口令（仅开发测试）")}
                            )
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
                        }
                        Text("数字验证码配对",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
                        Text("在被控电脑上查看设备号和动态验证码。连接请求仍需电脑端用户批准。",
                            color=Muted,fontSize=12.sp)
                        OutlinedTextField(
                            value=deviceNumber,
                            onValueChange={deviceNumber=it.filter { c -> c in '0'..'9' }.take(9)},
                            modifier=Modifier.fillMaxWidth(),
                            singleLine=true,
                            label={Text("9位设备号")},
                            placeholder={Text("例如 123456789")}
                        )
                        OutlinedTextField(
                            value=pairingCode,
                            onValueChange={pairingCode=it.filter { c -> c in '0'..'9' }.take(6)},
                            modifier=Modifier.fillMaxWidth(),
                            singleLine=true,
                            visualTransformation=PasswordVisualTransformation(),
                            label={Text("6位动态验证码")}
                        )
                        Button(
                            onClick={
                                if(pairingBusy) {
                                    pairingCancelled.set(true)
                                    pairingStatus="正在取消配对…"
                                } else {
                                    pairingCancelled.set(false)
                                    pairingBusy=true
                                    pairingStatus="正在发起安全配对…"
                                    Thread {
                                        try {
                                            val derived = PairingApiClient(server,relayToken)
                                                .pairAndDeriveSession(
                                                    deviceNumber,
                                                    pairingCode,
                                                    cancelled={pairingCancelled.get()},
                                                    onProgress={ message -> main.post { pairingStatus=message } }
                                                )
                                            val keyHex=derived.joinToString("") { "%02x".format(it) }
                                            main.post {
                                                pairingBusy=false
                                                pairingCode=""
                                                sessionKey=keyHex
                                                pairingStatus="配对成功，正在连接远程桌面…"
                                                connect(server,relayToken,keyHex,lanIp)
                                                mobilePage=1
                                            }
                                        } catch(error:Exception) {
                                            main.post {
                                                pairingBusy=false
                                                pairingStatus=error.message ?: "配对失败，请检查网络和验证码"
                                            }
                                        }
                                    }.start()
                                }
                            },
                            enabled=pairingBusy || (!requested && deviceNumber.length==9 && pairingCode.length==6),
                            modifier=Modifier.fillMaxWidth()
                        ) {
                            Text(if(pairingBusy) "取消配对" else "请求连接电脑")
                        }
                        Text(pairingStatus,color=if(pairingStatus.contains("成功"))Color(0xFF72D9AB) else Muted,fontSize=13.sp)

                        TextButton(onClick={showLegacyPairing=!showLegacyPairing}) {
                            Text(if(showLegacyPairing) "收起旧版密钥连接" else "旧版兼容连接（64位密钥）")
                        }
                        if(showLegacyPairing) {
                            OutlinedTextField(value=sessionKey,onValueChange={sessionKey=it.trim().take(64)},
                                modifier=Modifier.fillMaxWidth(),singleLine=true,
                                visualTransformation=PasswordVisualTransformation(),
                                label={Text("Windows 64位会话密钥")})
                            Row(verticalAlignment=Alignment.CenterVertically) {
                                Text("旧版设备号仅由密钥本地计算，不是服务端注册编号。",
                                    modifier=Modifier.weight(1f),color=Muted,fontSize=11.sp)
                                TextButton(onClick={
                                    val mgr=getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                    val pasted=mgr.primaryClip?.getItemAt(0)?.coerceToText(this@MainActivity)?.toString()?.trim() ?: ""
                                    if(pasted.length==64 && pasted.all{it in "0123456789abcdefABCDEF"}) {
                                        sessionKey=pasted
                                        status="已从手机剪贴板填入旧版配对密钥"
                                    } else {
                                        status="剪贴板没有完整的64位密钥"
                                    }
                                }) { Text("粘贴密钥") }
                            }
                            val parsedNumber = remember(sessionKey) {
                                runCatching { DeviceNumber.fromSecret(decodeKey(sessionKey)) }.getOrNull()
                            }
                            if(parsedNumber!=null) {
                                Text("旧版本地识别号：$parsedNumber",color=Accent,fontSize=13.sp)
                            }
                            Button(
                                onClick={ if (requested) disconnect() else connect(server,relayToken,sessionKey,lanIp) },
                                modifier=Modifier.fillMaxWidth()
                            ) { Text(if(requested)"停止连接 / 取消重试" else "使用旧版密钥连接") }
                        }
                        Text(status,color=if(receivingFrames)Color(0xFF72D9AB) else Muted,fontSize=13.sp)
                        Text("当前链路：$transport",color=Accent,fontSize=13.sp)
                        Button(onClick={mobilePage=1},enabled=requested,modifier=Modifier.fillMaxWidth()){
                            Text(if(receivingFrames) "查看远程桌面画面" else "进入远程画面")
                        }
                        if(showAdvancedNetwork) {
                        Row(verticalAlignment=Alignment.CenterVertically,
                            horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            Button(onClick={startUdpPunch()},
                                enabled=online && !usingLan && activeToken.length >= 32 && !udpAttempted
                            ) { Text("测试 UDP 并申请预览") }
                            Text(udpReport,color=Muted,fontSize=12.sp,modifier=Modifier.weight(1f))
                        }
                        Text("UDP 预览帧数：$udpPreviewFrames · 低画质测试最多20秒，失败自动回退 WSS",color=Muted,fontSize=12.sp)
                        Text("仅在 Windows 单独授权时切换 JPEG 预览；远程控制始终通过 WSS。",
                            color=Muted,fontSize=12.sp)
                        Text("端到端信令 RTT：${rttMs?.let{"$it ms"} ?: "等待响应"}",
                            color=Muted,fontSize=12.sp)
                        }
                        Text("只有主动点击加密保存才会存储可信配对；网络中断期间允许当前会话自动重连",color=Muted,fontSize=12.sp)
                    }
                    }
                    if (mobilePage==1) {
                    PanelCard {
                        Row(verticalAlignment=Alignment.CenterVertically) {
                            Text("远程画面",fontSize=20.sp,fontWeight=FontWeight.SemiBold)
                            Spacer(Modifier.weight(1f))
                            Text(if(receivingFrames)"● 正在接收画面" else if(online)"○ 等待家庭电脑" else "○ 未连接",fontSize=12.sp,color=if(receivingFrames)Color(0xFF72D9AB) else Muted)
                        }
                        val bitmap = frame
                        val displayWidth = if(h264Active) h264Width else (bitmap?.width ?: 1280)
                        val displayHeight = if(h264Active) h264Height else (bitmap?.height ?: 720)
                        Row(verticalAlignment=Alignment.CenterVertically,
                            horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                            Text("缩放 ${"%.1f".format(remoteZoom)}×",color=Accent,fontSize=13.sp,
                                modifier=Modifier.weight(1f))
                            TextButton(onClick={remoteExpanded=!remoteExpanded}) {
                                Text(if(remoteExpanded) "缩小画面区域" else "展开大画面")
                            }
                            TextButton(onClick={
                                remoteZoom=1f;remotePanX=0f;remotePanY=0f
                            }) { Text("还原") }
                        }
                        Box(
                            Modifier.fillMaxWidth()
                                .height(if(remoteExpanded) (screenHeightDp-180).coerceAtLeast(390).dp else 360.dp)
                                .clip(RoundedCornerShape(12.dp))
                                .background(Color(0xFF0F1625)),
                            contentAlignment=Alignment.Center
                        ) {
                            // Zoom applies only to video; overlay receives unscaled coordinates.
                            Box(Modifier.fillMaxSize().graphicsLayer {
                                scaleX=remoteZoom; scaleY=remoteZoom
                                translationX=remotePanX; translationY=remotePanY
                            },contentAlignment=Alignment.Center) {
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
                            // On top of native TextureView so video cannot swallow taps.
                            Box(Modifier.matchParentSize()
                                .pointerInput(online,h264Active,displayWidth,displayHeight) {
                                    if(!online) return@pointerInput
                                    awaitEachGesture {
                                        val down=awaitFirstDown(requireUnconsumed=false)
                                        val startX=down.position.x
                                        val startY=down.position.y
                                        var dragging=false
                                        var pinching=false
                                        try {
                                            while(true) {
                                                val event=awaitPointerEvent()
                                                val active=event.changes.filter{it.pressed}
                                                if(active.size>=2) {
                                                    if(dragging) {
                                                        sendControl("up",pointerX,pointerY)
                                                        dragging=false
                                                    }
                                                    pinching=true
                                                    remoteZoom=(remoteZoom*event.calculateZoom()).coerceIn(1f,4f)
                                                    val pan=event.calculatePan()
                                                    remotePanX=RemoteGeometry.boundPan(
                                                        remotePanX+pan.x,remoteZoom,size.width.toFloat())
                                                    remotePanY=RemoteGeometry.boundPan(
                                                        remotePanY+pan.y,remoteZoom,size.height.toFloat())
                                                    event.changes.forEach{it.consume()}
                                                    continue
                                                }
                                                if(active.isEmpty()) {
                                                    if(!pinching && !dragging) {
                                                        val hit=RemoteGeometry.transformedPointer(
                                                            startX,startY,size.width.toFloat(),size.height.toFloat(),
                                                            displayWidth,displayHeight,remoteZoom,remotePanX,remotePanY
                                                        )
                                                        if(hit!=null) {
                                                            pointerX=hit.first;pointerY=hit.second
                                                            pointerValid=true
                                                            sendControl("down",pointerX,pointerY)
                                                            sendControl("up",pointerX,pointerY)
                                                        }
                                                    }
                                                    event.changes.forEach{it.consume()}
                                                    break
                                                }
                                                if(!pinching) {
                                                    val point=active.first().position
                                                    val hit=RemoteGeometry.transformedPointer(
                                                        point.x,point.y,size.width.toFloat(),size.height.toFloat(),
                                                        displayWidth,displayHeight,remoteZoom,remotePanX,remotePanY
                                                    )
                                                    val distance=kotlin.math.hypot(point.x-startX,point.y-startY)
                                                    if(!dragging && distance>9f) {
                                                        val first=RemoteGeometry.transformedPointer(
                                                            startX,startY,size.width.toFloat(),size.height.toFloat(),
                                                            displayWidth,displayHeight,remoteZoom,remotePanX,remotePanY
                                                        )
                                                        if(first!=null) {
                                                            pointerX=first.first;pointerY=first.second
                                                            pointerValid=true
                                                            sendControl("down",pointerX,pointerY)
                                                            dragging=true
                                                        }
                                                    }
                                                    if(dragging && hit!=null) {
                                                        pointerX=hit.first;pointerY=hit.second
                                                        sendControl("move",pointerX,pointerY)
                                                    }
                                                }
                                                event.changes.forEach{it.consume()}
                                            }
                                        } finally {
                                            if(dragging) sendControl("up",pointerX,pointerY)
                                        }
                                    }
                                })
                        }
                        Text(if(h264Active)"视频模式：H.264 · Android MediaCodec"
                            else "视频模式：JPEG（兼容模式）",color=Muted,fontSize=12.sp)
                        Text("轻点=左键 · 单指拖动=鼠标拖拽 · 双指缩放/平移（最高 4×）",
                            color=Muted,fontSize=12.sp)
                        Text(when(hostAllowsInput){
                            true -> "电脑端已允许远程鼠标和键盘操作"
                            false -> "电脑端尚未允许输入：请在 Windows【安全与画质】打开授权"
                            null -> "等待电脑返回输入权限状态"
                        },color=if(hostAllowsInput==true) Color(0xFF72D9AB) else Color(0xFFFFB785),
                            fontSize=12.sp)
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(7.dp)) {
                            Text("显示器 ${selectedMonitor+1}/$monitorCount",fontSize=13.sp,color=Muted,modifier=Modifier.weight(1f))
                            Button(onClick={selectMonitor((selectedMonitor-1+monitorCount)%monitorCount)},enabled=online && monitorCount>1) {Text("上一屏")}
                            Button(onClick={selectMonitor((selectedMonitor+1)%monitorCount)},enabled=online && monitorCount>1) {Text("下一屏")}
                        }
                        if(monitorCount > 1) {
                            Row(
                                modifier=Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                                horizontalArrangement=Arrangement.spacedBy(6.dp)
                            ) {
                                (0 until monitorCount.coerceAtMost(16)).forEach { index ->
                                    FilterChip(
                                        selected=selectedMonitor==index,
                                        onClick={selectMonitor(index)},
                                        enabled=online,
                                        label={Text("屏 ${index+1}")}
                                    )
                                }
                            }
                        }
                        Row(horizontalArrangement=Arrangement.spacedBy(6.dp)){
                            Button(onClick={sendControl("right_click",pointerX,pointerY)},
                                enabled=online && pointerValid,modifier=Modifier.weight(1f)){Text("右键")}
                            Button(onClick={sendScroll(-3)},enabled=online,modifier=Modifier.weight(1f)){Text("上滚")}
                            Button(onClick={sendScroll(3)},enabled=online,modifier=Modifier.weight(1f)){Text("下滚")}
                        }
                        Text("键盘导航",fontWeight=FontWeight.SemiBold,fontSize=15.sp)
                        Row(horizontalArrangement=Arrangement.spacedBy(5.dp)){
                            listOf("↑" to "up","↓" to "down","←" to "left","→" to "right").forEach { (label,key) ->
                                Button(onClick={sendRemoteKey(key)},enabled=online,modifier=Modifier.weight(1f)){
                                    Text(label,fontSize=15.sp)
                                }
                            }
                        }
                        Row(horizontalArrangement=Arrangement.spacedBy(5.dp)){
                            listOf("Home" to "home","End" to "end","PgUp" to "page_up","PgDn" to "page_down").forEach { (label,key) ->
                                Button(onClick={sendRemoteKey(key)},enabled=online,modifier=Modifier.weight(1f)){
                                    Text(label,fontSize=11.sp)
                                }
                            }
                        }
                        Row(horizontalArrangement=Arrangement.spacedBy(5.dp)){
                            listOf("Enter" to "enter","Tab" to "tab","Esc" to "escape","Del" to "delete").forEach { (label,key) ->
                                Button(onClick={sendRemoteKey(key)},enabled=online,modifier=Modifier.weight(1f)){
                                    Text(label,fontSize=12.sp)
                                }
                            }
                        }
                        Text("编辑快捷键（被控电脑需要允许远程输入）",fontWeight=FontWeight.SemiBold,fontSize=14.sp)
                        Row(horizontalArrangement=Arrangement.spacedBy(5.dp)){
                            listOf("全选" to "ctrl_a","复制" to "ctrl_c",
                                "粘贴" to "ctrl_v","撤销" to "ctrl_z").forEach { (label,key) ->
                                Button(onClick={sendRemoteKey(key)},enabled=online,modifier=Modifier.weight(1f)){
                                    Text(label,fontSize=12.sp)
                                }
                            }
                        }
                        Text("微信聊天：先在上方远程画面点选聊天输入框，再输入文字；是否按回车发送由你决定。",
                            fontSize=12.sp,color=Muted)
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                            OutlinedTextField(value=textToSend,onValueChange={textToSend=it.take(500)},
                                modifier=Modifier.weight(1f),singleLine=true,label={Text("输入到远程聊天框")})
                            Button(onClick={sendControlText(textToSend);textToSend=""},
                                enabled=online && hostAllowsInput!=false && textToSend.isNotBlank()){
                                Text("输入")
                            }
                        }
                        Button(
                            onClick={
                                val message=textToSend
                                sendControlText(message)
                                sendRemoteKey("enter")
                                textToSend=""
                            },
                            enabled=online && hostAllowsInput!=false && textToSend.isNotBlank(),
                            modifier=Modifier.fillMaxWidth()
                        ) { Text("输入并按 Enter 发送聊天") }
                    }
                    }
                    if (mobilePage==2) {
                    PanelCard {
                        Text("剪贴板同步",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
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
                        Text("安全与隐私",fontSize=18.sp,fontWeight=FontWeight.SemiBold)
                        Text("桌面画面采用 AES-256-GCM 端到端加密；中转服务器无法解密图像。首次测试请仅在你自己的电脑上使用。",fontSize=13.sp,color=Muted,lineHeight=20.sp)
                    }
                    }
                    Spacer(Modifier.height(15.dp))
                }
                }
            }
        }
    }

    @Composable private fun PanelCard(content:@Composable ColumnScope.()->Unit) {
        Surface(
            modifier=Modifier.fillMaxWidth(),
            shape=RoundedCornerShape(20.dp),
            color=Panel,
            tonalElevation=2.dp,
            border=androidx.compose.foundation.BorderStroke(1.dp,OutlineSoft)
        ) {
            Column(
                Modifier.fillMaxWidth().padding(20.dp),
                verticalArrangement=Arrangement.spacedBy(14.dp),
                content=content
            )
        }
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
        hostAllowsInput = null
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
        val handshake = if (tryLan) LanHandshake() else null
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
                                        currentKey, 6, requireNotNull(handshake).hello()
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
                // Signaling must not be dropped by a busy JPEG/H.264 decoder.
                // AES-GCM authenticates this outer message kind as AAD.
                if (bytes.size > 0 && (bytes[0].toInt() and 255) == 10) {
                    try {
                        val key = secret ?: return
                        val message = decryptPacket(key, bytes.toByteArray())
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
                if (tryLan && bytes.size > 0 && (bytes[0].toInt() and 255) == 7) {
                    try {
                        val key = secret ?: return
                        val (kind, payload) = decryptPacket(key, bytes.toByteArray())
                        if (kind != 7) return
                        main.post {
                            if (socket === webSocket && requested && !lanVerified) {
                                val state = requireNotNull(handshake)
                                val proof = state.acceptChallenge(payload)
                                if (proof != null) {
                                    if (!webSocket.send(encryptPacket(key, 6, proof).toByteString())) {
                                        webSocket.cancel()
                                    }
                                } else if (state.acceptReady(payload)) {
                                    lanVerified = true
                                    online = true
                                    receivingFrames = false
                                    transport = "局域网 P2P 直连"
                                    status = "已通过局域网加密验证 · 等待画面"
                                    startWatchdog()
                                } else {
                                    webSocket.cancel()
                                }
                            }
                        }
                    } catch (_: Exception) { webSocket.cancel() }
                    return
                }
                if (!decoding.compareAndSet(false, true)) return
                try {
                    val key = secret ?: return
                    val original = decryptPacket(key, bytes.toByteArray())
                    if (tryLan && !lanVerified) return
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
                                    h264Active = false
                                    if(udpLastFrameAtMs != 0L) {
                                        udpLastFrameAtMs = 0L
                                        transport = "VPS WSS 中转"
                                    }
                                    receivingFrames = true
                                    lastFrameAtMs = SystemClock.elapsedRealtime()
                                    retryAttempt = 0
                                    status = "已连接家庭电脑 · 正在接收画面"
                                }
                            }
                        }
                        5 -> {
                            if (udpLastFrameAtMs != 0L &&
                                SystemClock.elapsedRealtime() - udpLastFrameAtMs < 1500L) return
                            val accessUnit = AvcPayload.parse(original.second)
                            if(accessUnit != null) {
                                main.post {
                                    if(socket===webSocket && requested){
                                        if(udpLastFrameAtMs != 0L) {
                                            udpLastFrameAtMs = 0L
                                            transport = "VPS WSS 中转"
                                        }
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
                            val inputAllowed = if(metadata.has("allow_input"))
                                metadata.optBoolean("allow_input") else null
                            main.post {
                                if(socket===webSocket && requested){
                                        monitorCount=count;selectedMonitor=index
                                        hostAllowsInput=inputAllowed
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
                    socket = null
                    online = false
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
                        try { ws.send(encryptPacket(key, 10,
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
                            if(h264Active) avcDecoder.reset()
                            h264Active = false
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
                encryptPacket(key,10,request.toByteArray(Charsets.UTF_8)).toByteString()
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
        clipboardResult = ""
        monitorCount = 1
        selectedMonitor = 0
        pointerValid = false
        hostAllowsInput = null
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
    private fun sendControlText(text:String){
        // Text injection is explicit; never forward private IME/clipboard contents.
        if(text.isNotBlank() && text.codePointCount(0,text.length) in 1..250) {
            send(JSONObject().put("type","text").put("text",text))
            status="已发送文字；请先点击电脑端的微信/聊天输入框并确认允许远控"
        }
    }
    private fun sendRemoteKey(name:String){
        if(!RemoteKeys.isAllowed(name))return
        send(JSONObject().put("type","key").put("key",name))
    }
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
