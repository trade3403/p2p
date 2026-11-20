$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$flutterAndroid = Join-Path $repoRoot 'flutter_app\android'
if (-not (Test-Path $flutterAndroid)) {
    throw "Flutter Android platform has not been generated: $flutterAndroid"
}

$nativeSource = Join-Path $repoRoot 'android\app\src\main\java\com\p2p\remote'
$targetSource = Join-Path $flutterAndroid 'app\src\main\kotlin\com\p2p\remote'
if (-not (Test-Path $nativeSource)) {
    throw "Existing Android native source directory is missing: $nativeSource"
}
New-Item -ItemType Directory -Force -Path $targetSource | Out-Null
Get-ChildItem -LiteralPath $nativeSource -Filter '*.kt' -File |
    Where-Object { $_.Name -ne 'MainActivity.kt' } |
    ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $targetSource -Force }

$activity = Get-ChildItem -Path (Join-Path $flutterAndroid 'app\src\main\kotlin') -Filter 'MainActivity.kt' -Recurse |
    Select-Object -First 1
if (-not $activity) {
    throw 'Flutter generated Android MainActivity.kt was not found.'
}

$mainActivity = @'
package com.p2p.remote.p2p_remote_ui

import android.view.Surface
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry
import com.p2p.remote.AndroidRemoteEngine

class MainActivity : FlutterActivity() {
    private var remoteEngine: AndroidRemoteEngine? = null
    private var eventSink: EventChannel.EventSink? = null
    private var textureEntry: TextureRegistry.SurfaceTextureEntry? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        val entry = flutterEngine.renderer.createSurfaceTexture()
        textureEntry = entry
        entry.surfaceTexture().setDefaultBufferSize(1280, 720)

        val engine = AndroidRemoteEngine(
            applicationContext,
            onEvent = { event ->
                runOnUiThread {
                    val payload = HashMap<String, Any?>(event)
                    if (payload["type"] == "snapshot") {
                        payload["texture_id"] = entry.id()
                    }
                    eventSink?.success(payload)
                }
            },
            onVideoSize = { width, height ->
                runOnUiThread {
                    entry.surfaceTexture().setDefaultBufferSize(
                        width.coerceIn(1, 8192),
                        height.coerceIn(1, 8192)
                    )
                }
            }
        )
        remoteEngine = engine
        engine.attachVideoSurface(Surface(entry.surfaceTexture()))

        MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "com.p2p.remote/bridge"
        ).setMethodCallHandler { call, result ->
            val arguments = call.arguments as? Map<String, Any?> ?: emptyMap()
            val response = engine.invoke(call.method, arguments).toMutableMap()
            val snapshot = (response["snapshot"] as? Map<String, Any?>)?.toMutableMap()
            if (snapshot != null) {
                snapshot["texture_id"] = entry.id()
                response["snapshot"] = snapshot
            }
            result.success(response)
        }

        EventChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "com.p2p.remote/events"
        ).setStreamHandler(object : EventChannel.StreamHandler {
            override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
                eventSink = events
                engine.emitSnapshot(force = true)
            }

            override fun onCancel(arguments: Any?) {
                eventSink = null
            }
        })
    }

    override fun onDestroy() {
        remoteEngine?.close()
        remoteEngine = null
        eventSink = null
        textureEntry?.release()
        textureEntry = null
        super.onDestroy()
    }
}
'@
Set-Content -LiteralPath $activity.FullName -Value $mainActivity -Encoding UTF8

$gradle = Join-Path $flutterAndroid 'app\build.gradle.kts'
$gradleText = Get-Content -LiteralPath $gradle -Raw
$gradleText = $gradleText -replace 'minSdk\s*=\s*flutter\.minSdkVersion', 'minSdk = 26'
if ($gradleText -notmatch 'com\.squareup\.okhttp3:okhttp') {
    $gradleText += @'

dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.bouncycastle:bcprov-jdk18on:1.80")
}
'@
}
Set-Content -LiteralPath $gradle -Value $gradleText -Encoding UTF8

$manifest = Join-Path $flutterAndroid 'app\src\main\AndroidManifest.xml'
$manifestText = Get-Content -LiteralPath $manifest -Raw
if ($manifestText -notmatch 'android\.permission\.INTERNET') {
    $manifestText = [regex]::Replace(
        $manifestText,
        '(<manifest[^>]*>)',
        ('$1' + [Environment]::NewLine + '    <uses-permission android:name="android.permission.INTERNET" />'),
        1
    )
}
$manifestText = $manifestText -replace 'android:label="[^"]+"', 'android:label="P2P Remote"'
Set-Content -LiteralPath $manifest -Value $manifestText -Encoding UTF8

Write-Host 'Flutter Android platform is linked to the existing Kotlin pairing, crypto, WSS, JPEG/H.264 and input modules.'
Write-Host ('Android activity: ' + $activity.FullName)
