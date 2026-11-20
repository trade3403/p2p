from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[1]
ANDROID = ROOT / "flutter_app" / "android"
NATIVE = ROOT / "android" / "app" / "src" / "main" / "java" / "com" / "p2p" / "remote"
TARGET = ANDROID / "app" / "src" / "main" / "kotlin" / "com" / "p2p" / "remote"

if not ANDROID.is_dir():
    raise SystemExit(f"Flutter Android platform has not been generated: {ANDROID}")
if not NATIVE.is_dir():
    raise SystemExit(f"Existing Android native sources are missing: {NATIVE}")

TARGET.mkdir(parents=True, exist_ok=True)
for source in NATIVE.glob("*.kt"):
    if source.name == "MainActivity.kt":
        continue
    shutil.copy2(source, TARGET / source.name)

activities = list((ANDROID / "app" / "src" / "main" / "kotlin").rglob("MainActivity.kt"))
if not activities:
    raise SystemExit("Flutter generated Android MainActivity.kt was not found.")

main_activity = r'''package com.p2p.remote.p2p_remote_ui

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
            @Suppress("UNCHECKED_CAST")
            val arguments = call.arguments as? Map<String, Any?> ?: emptyMap()
            val response = engine.invoke(call.method, arguments).toMutableMap()
            @Suppress("UNCHECKED_CAST")
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
'''
activities[0].write_text(main_activity, encoding="utf-8")

gradle = ANDROID / "app" / "build.gradle.kts"
gradle_text = gradle.read_text(encoding="utf-8")
gradle_text = re.sub(r"minSdk\s*=\s*flutter\.minSdkVersion", "minSdk = 26", gradle_text)
if "com.squareup.okhttp3:okhttp" not in gradle_text:
    gradle_text += """
 
dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.bouncycastle:bcprov-jdk18on:1.80")
}
"""
gradle.write_text(gradle_text, encoding="utf-8")

manifest = ANDROID / "app" / "src" / "main" / "AndroidManifest.xml"
manifest_text = manifest.read_text(encoding="utf-8")
if "android.permission.INTERNET" not in manifest_text:
    manifest_text = re.sub(
        r"(<manifest[^>]*>)",
        r'\1\n    <uses-permission android:name="android.permission.INTERNET" />',
        manifest_text,
        count=1,
    )
manifest_text = re.sub(r'android:label="[^"]+"', 'android:label="P2P Remote"', manifest_text)
manifest.write_text(manifest_text, encoding="utf-8")

print("Flutter Android platform is linked to the existing Kotlin pairing, crypto, WSS, JPEG/H.264 and input modules.")
print(f"Android activity: {activities[0]}")
