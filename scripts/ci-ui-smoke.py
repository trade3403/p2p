"""CI UI smoke test. Only synthetic offline app navigation; no credentials."""
import io
import json
import pathlib
import subprocess
import sys
import time
import traceback
from PIL import Image, ImageChops, ImageStat
from android_ui_ready import foreground_state, start_release_app

platform = sys.argv[1]
out = pathlib.Path("evidence") / platform
out.mkdir(parents=True, exist_ok=True)
report = {"platform": platform, "scope": "offline navigation smoke; no remote session", "steps": [], "passed": False}
previous = None

def check(name, image):
    global previous
    image = image.convert("RGB")
    image.save(out / (name + ".png"))
    crop = image.crop((image.width // 4, image.height // 8, image.width * 9 // 10, image.height * 4 // 5)).resize((160, 120))
    if ImageStat.Stat(crop).stddev[0] < 2:
        raise AssertionError("Blank capture: " + name)
    if previous is not None:
        difference = sum(ImageStat.Stat(ImageChops.difference(previous, crop)).mean) / 3
        if difference < 0.8:
            raise AssertionError("No meaningful page change: " + name)
    previous = crop
    report["steps"].append(name)

def adb(*args, **kwargs):
    return subprocess.run(["adb", *args], check=True, **kwargs)

def android():
    apk = next(pathlib.Path("apk").rglob("app-release.apk"))
    adb("install", "-r", str(apk))
    package = "com.p2p.remote.p2p_remote_ui"
    recording = subprocess.Popen(["adb", "shell", "screenrecord", "--time-limit", "90", "/sdcard/p2p-ci.mp4"])
    try:
        def capture():
            return Image.open(io.BytesIO(adb("exec-out", "screencap", "-p", stdout=subprocess.PIPE).stdout)).convert("RGB")
        try:
            start_release_app(package,report)
        except Exception:
            capture().save(out/"00-startup-focus-failed.png")
            raise
        image = capture()
        check("01-home", image)
        for name, fraction in [("02-devices", .375), ("03-remote", .625), ("04-settings", .875), ("05-home", .125)]:
            adb("shell", "input", "tap", str(int(image.width * fraction)), str(int(image.height * .925)))
            time.sleep(3)
            current=capture()
            current.save(out/(name+".png"))
            ok,state=foreground_state(package)
            report["last_focus"]=state
            if not ok:
                raise AssertionError("App lost foreground input focus at "+name)
            check(name,current)
    finally:
        subprocess.run(["adb", "shell", "pkill", "-2", "screenrecord"], check=False)
        try:
            recording.wait(timeout=15)
        except subprocess.TimeoutExpired:
            recording.terminate()
        adb("pull", "/sdcard/p2p-ci.mp4", str(out / "Android-UI-Test.mp4"))
        with (out / "logcat.txt").open("w", encoding="utf-8") as log:
            subprocess.run(["adb", "logcat", "-d", "-t", "1500"], stdout=log, check=False)

def windows():
    import pyautogui as gui
    import imageio_ffmpeg
    import threading
    exe = next(pathlib.Path("dist/v0.21-windows").glob("p2p_remote_ui.exe")).resolve()
    proc = subprocess.Popen([str(exe)], cwd=exe.parent)
    stop = threading.Event()
    errors = []
    def record():
        writer = None
        try:
            first = gui.screenshot().convert("RGB")
            w, h = first.size
            w, h = w // 2 * 2, h // 2 * 2
            writer = imageio_ffmpeg.write_frames(str(out / "Windows-UI-Test.mp4"), (w, h), fps=4, codec="libx264", pix_fmt_in="rgb24", pix_fmt_out="yuv420p")
            writer.send(None)
            while not stop.is_set():
                start = time.monotonic()
                writer.send(gui.screenshot().convert("RGB").crop((0, 0, w, h)).tobytes())
                stop.wait(max(0, .25 - (time.monotonic() - start)))
        except Exception as exc:
            errors.append(str(exc))
        finally:
            if writer:
                writer.close()
    worker = threading.Thread(target=record, daemon=True)
    worker.start()
    try:
        window = None
        for _ in range(40):
            if proc.poll() is not None:
                raise AssertionError("Application exited during startup")
            matches = [w for w in gui.getAllWindows() if "P2P Remote" in w.title and w.width > 300]
            if matches:
                window = matches[0]
                break
            time.sleep(1)
        if window is None:
            raise AssertionError("No interactive P2P window found on runner")
        window.maximize()
        window.activate()
        time.sleep(4)
        # Sidebar layout uses logical pixels. Use the actual client origin, not the outer window border.
        import ctypes
        from ctypes import wintypes
        point = wintypes.POINT(0, 0)
        if not ctypes.windll.user32.ClientToScreen(window._hWnd, ctypes.byref(point)):
            raise AssertionError("Unable to locate Windows client area")
        rect = wintypes.RECT()
        if not ctypes.windll.user32.GetClientRect(window._hWnd, ctypes.byref(rect)):
            raise AssertionError("Unable to measure Windows client area")
        dpi = ctypes.windll.user32.GetDpiForWindow(window._hWnd) or 96
        scale = dpi / 96
        def capture():
            return gui.screenshot(region=(point.x, point.y, rect.right, rect.bottom))
        check("01-home", capture())
        # Use the actual Windows UI Automation tree, as in the release UI E2E.
        # Pixel-pitch extrapolation missed Security on run 45425889014:
        # both device/security screenshots were identical. Never mark a
        # navigation successful merely because the click returned.
        from pywinauto import Desktop
        desktop = Desktop(backend="uia")
        hwnd = window._hWnd
        def visible_control(label, sidebar):
            try:
                candidates=[]
                for obj in desktop.window(handle=hwnd).descendants():
                    name=obj.element_info.name or ""
                    if label not in name.splitlines():
                        continue
                    r=obj.rectangle()
                    if not obj.is_visible() or r.width()<4 or r.height()<4:
                        continue
                    x=(r.left+r.right)//2
                    y=(r.top+r.bottom)//2
                    if not (point.x<=x<point.x+rect.right and point.y<=y<point.y+rect.bottom):
                        continue
                    if sidebar:
                        if not (x<point.x+int(238*scale) and y>point.y+int(140*scale) and obj.is_enabled()):
                            continue
                    elif x<=point.x+int(238*scale):
                        continue
                    candidates.append((r.width()*r.height(),x,y))
                return min(candidates) if candidates else None
            except Exception:
                return None
        def until(predicate, seconds):
            deadline=time.monotonic()+seconds
            while time.monotonic()<deadline:
                if proc.poll() is not None:
                    raise AssertionError("Application exited during navigation")
                value=predicate()
                if value:
                    return value
                time.sleep(.35)
            return None
        for name,label,marker in [
            ("02-remote","远程控制","REMOTE SESSION  /  V0.21"),
            ("03-devices","设备管理","我的设备"),
            ("04-security","安全与权限","允许远程鼠标和键盘"),
            ("05-network","网络连接","保存中转配置"),
            ("06-settings","设置","PREFERENCES"),
        ]:
            target=until(lambda: visible_control(label,True),15)
            if target is None:
                capture().save(out/(name+"-missing-nav.png"))
                raise AssertionError("Could not locate actual navigation control: "+label)
            _,x,y=target
            # Retry only the real target if an input/focus event was lost;
            # retain the original meaningful screenshot difference threshold.
            observed=None
            for attempt in range(3):
                window.activate()
                gui.click(x,y)
                observed=until(lambda: visible_control(marker,False),6)
                if observed:
                    break
            if not observed:
                capture().save(out/(name+"-wrong-page.png"))
                raise AssertionError("Expected visible page control not found: "+marker)
            time.sleep(.6)
            check(name,capture())
            report.setdefault("verified_windows_pages",[]).append({"page":name,"label":label,"marker":marker})
    finally:
        stop.set()
        worker.join(20)
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=15)
    if errors or not (out / "Windows-UI-Test.mp4").exists():
        raise AssertionError("Recording failed: " + repr(errors))

try:
    {"windows": windows, "android": android}[platform]()
    report["passed"] = True
except Exception:
    report["error"] = traceback.format_exc()
    raise
finally:
    (out / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
