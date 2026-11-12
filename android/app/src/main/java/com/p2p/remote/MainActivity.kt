package com.p2p.remote

import android.app.Activity
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.graphics.RectF
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class MainActivity : Activity() {
    private val ui = Handler(Looper.getMainLooper())
    private val client = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .build()
    private var socket: WebSocket? = null
    private lateinit var status: TextView
    private lateinit var image: ImageView
    private lateinit var toggle: Button
    private val framePending = AtomicBoolean(false)
    private var connected = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(14.dp, 12.dp, 14.dp, 12.dp) }
        val heading = TextView(this).apply { text = "P2P Remote · 个人测试版 V0.1"; textSize = 19f }
        root.addView(heading)
        val ip = EditText(this).apply { hint = "电脑 Tailscale IP，例如 100.101.102.103"; setSingleLine(true) }
        val token = EditText(this).apply { hint = "Windows 端显示的 64 位配对密钥"; setSingleLine(true) }
        root.addView(ip)
        root.addView(token)
        toggle = Button(this).apply { text = "连接电脑" }
        root.addView(toggle)
        status = TextView(this).apply { text = "请先连接 Tailscale，再输入地址与配对密钥。" }
        root.addView(status)
        image = ImageView(this).apply {
            setBackgroundColor(0xff1e1e1e.toInt())
            scaleType = ImageView.ScaleType.FIT_CENTER
            adjustViewBounds = false
        }
        root.addView(image, LinearLayout.LayoutParams(-1, 0, 1f))
        val sendLine = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        val message = EditText(this).apply { hint = "在电脑输入文本"; setSingleLine(true) }
        sendLine.addView(message, LinearLayout.LayoutParams(0, -2, 1f))
        val send = Button(this).apply { text = "发送" }
        sendLine.addView(send)
        root.addView(sendLine)
        val foot = TextView(this).apply { text = "远程鼠标：触摸为左键，滑动为拖动。需 Windows 端勾选允许远程输入。" }
        root.addView(foot)
        setContentView(root)

        toggle.setOnClickListener {
            if (connected || socket != null) {
                socket?.close(1000, "User disconnected")
                socket = null
                showDisconnected("已断开")
            } else {
                val host = ip.text.toString().trim()
                val key = token.text.toString().trim()
                if (!isTailnetAddress(host) || !Regex("[0-9a-fA-F]{64}").matches(key)) {
                    status.text = "请输入合法的 Tailscale 100.64.0.0/10 地址及 64 位密钥。"
                    return@setOnClickListener
                }
                status.text = "正在连接 $host ..."
                toggle.text = "断开连接"
                connect(host, key)
            }
        }
        send.setOnClickListener {
            val value = message.text.toString()
            if (value.isNotEmpty() && value.length <= 500) {
                socket?.send(JSONObject().put("type", "text").put("text", value).toString())
                message.text.clear()
            }
        }
        image.setOnTouchListener { _, event ->
            val ws = socket ?: return@setOnTouchListener false
            if (!connected) return@setOnTouchListener false
            val drawable = image.drawable ?: return@setOnTouchListener false
            val bounds = RectF(0f, 0f, drawable.intrinsicWidth.toFloat(), drawable.intrinsicHeight.toFloat())
            image.imageMatrix.mapRect(bounds)
            if (bounds.width() <= 0 || bounds.height() <= 0) return@setOnTouchListener false
            val x = ((event.x - bounds.left) / bounds.width()).coerceIn(0f, 1f)
            val y = ((event.y - bounds.top) / bounds.height()).coerceIn(0f, 1f)
            val type = when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> "down"
                MotionEvent.ACTION_MOVE -> "move"
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> "up"
                else -> return@setOnTouchListener true
            }
            ws.send(JSONObject().put("type", type).put("x", x.toDouble()).put("y", y.toDouble()).toString())
            true
        }
    }

    private fun connect(host: String, key: String) {
        val request = Request.Builder().url("ws://$host:22119/ws").header("Authorization", "Bearer $key").build()
        socket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                ui.post { connected = true; status.text = "已连接：$host"; toggle.text = "断开连接" }
            }
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (!framePending.compareAndSet(false, true)) return
                val raw = bytes.toByteArray()
                val bitmap = BitmapFactory.decodeByteArray(raw, 0, raw.size)
                ui.post {
                    if (bitmap != null && !isFinishing) image.setImageBitmap(bitmap)
                    framePending.set(false)
                }
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                ui.post { if (socket === webSocket) { socket = null; showDisconnected("连接失败：${response?.code ?: t.message}") } }
            }
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                ui.post { if (socket === webSocket) { socket = null; showDisconnected("连接已关闭") } }
            }
        })
    }

    private fun showDisconnected(message: String) {
        connected = false
        toggle.text = "连接电脑"
        status.text = message
    }

    private fun isTailnetAddress(host: String): Boolean {
        val parts = host.split(".")
        if (parts.size != 4) return false
        val octets = parts.map { it.toIntOrNull() ?: return false }
        if (octets.any { it !in 0..255 }) return false
        return octets[0] == 100 && octets[1] in 64..127
    }

    private val Int.dp: Int get() = (this * resources.displayMetrics.density).toInt()

    override fun onDestroy() {
        socket?.close(1000, "App exited")
        client.dispatcher.executorService.shutdown()
        super.onDestroy()
    }
}
