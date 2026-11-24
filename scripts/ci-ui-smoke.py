"""CI UI smoke test. Only synthetic offline app navigation; no credentials."""
import io
import json
import pathlib
import subprocess
import sys
import time
import traceback
from PIL import Image, ImageChops, ImageStat

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
        adb("shell", "am", "start", "-W", "-n", package + "/.MainActivity")
        time.sleep(8)
        def capture():
            return Image.open(io.BytesIO(adb("exec-out", "screencap", "-p", stdout=subprocess.PIPE).stdout)).convert("RGB")
        image = capture()
        check("01-home", image)
        for name, fraction in [("02-devices", .375), ("03-remote", .625), ("04-settings", .875), ("05-home", .125)]:
            adb("shell", "input", "tap", str(int(image.width * fraction)), str(int(image.height * .925)))
            time.sleep(3)
            check(name, capture())
        focused = adb("shell", "dumpsys", "activity", "activities", stdout=subprocess.PIPE, text=True).stdout
        if not any(package in line and ("ResumedActivity" in line or "topResumedActivity" in line) for line in focused.splitlines()):
            raise AssertionError("App is not the resumed activity")
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
    exe = next(pathlib.Path("dist/v0.20-windows").glob("p2p_remote_ui.exe")).resolve()
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
        # Sidebar starts below the custom title bar, with six navigation rows.
        # Detect the selected Home row from its exact Flutter fill color
        # (#193B49), then infer the navigation row pitch from its height
        # and the six logical pixel inter-row gap.
        home = capture().convert("RGB")
        x = int(30 * scale)
        selected = [y for y in range(int(140 * scale), min(home.height, int(260 * scale)))
                    if home.getpixel((x, y)) == (25, 59, 73)]
        if len(selected) < int(20 * scale):
            raise AssertionError("Could not identify selected Home navigation row")
        center = (min(selected) + max(selected)) / 2
        pitch = (max(selected) - min(selected) + 1) + int(8 * scale)
        for index, name in enumerate(["02-remote", "03-devices", "04-security", "05-network", "06-settings"], 1):
            gui.click(point.x + int(120 * scale), point.y + round(center + index * pitch))
            time.sleep(3)
            check(name, capture())
            if proc.poll() is not None:
                raise AssertionError("Application exited during navigation")
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
