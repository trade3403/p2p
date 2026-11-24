"""Release UI E2E: installed Windows EXE and unmodified release Flutter APK.

Only UI automation, real public WSS and independent OS observations.
No Rust C ABI, Kotlin instrumentation, debug IPC or synthetic video.
Secrets entered through UI but never recorded or written to evidence.
"""
import hashlib
import hmac
import io
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import subprocess
import sys
import threading
import time
import urllib.request
import xml.etree.ElementTree as ET

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import imageio_ffmpeg
from PIL import Image, ImageChops, ImageStat
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/"scripts"))
from android_ui_ready import foreground_state, start_release_app
import websocket

HOST="p2ptest.aikissgo.com"
TOKEN=os.environ.get("RELAY_ACCESS_TOKEN","").strip()
ROLE=sys.argv[1]
OUT=Path("evidence/release-ui-"+ROLE)
OUT.mkdir(parents=True,exist_ok=True)
REPORT={"role":ROLE,"scope":"actual release client UI on dedicated GitHub runner","passed":False,"steps":[]}
STAGE="initialization"
WS=None
KEY=None
MARKER="P2P-V021-PUBLIC-UI-"+os.environ.get("GITHUB_RUN_ID","local")
PACKAGE="com.p2p.remote.p2p_remote_ui"

def require(value,reason):
    if not value: raise AssertionError(reason)

def passed(name):
    REPORT["steps"].append(name)
    print("PASS:",name,flush=True)

def wait(label,predicate,seconds=40):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        result=predicate()
        if result: return result
        time.sleep(.3)
    raise AssertionError("Timed out: "+label)

def rendezvous(role):
    global KEY,WS
    require(os.environ.get("GITHUB_ACTIONS")=="true","Dedicated CI runner only")
    require(32<=len(TOKEN)<=512,"Missing RELAY_ACCESS_TOKEN secret")
    with urllib.request.urlopen("https://"+HOST+"/healthz",timeout=15) as r:
        require(r.status==200 and r.read().strip()==b"ok","Public Relay healthz failed")
    ctx="|".join(["p2p-release-ui-v1",os.environ["GITHUB_REPOSITORY"],
                 os.environ["GITHUB_RUN_ID"],os.environ["GITHUB_RUN_ATTEMPT"]])
    KEY=hmac.digest(TOKEN.encode(),ctx.encode(),"sha256")
    room=hashlib.sha256(b"p2p-release-ui:"+KEY).hexdigest()
    WS=websocket.create_connection("wss://"+HOST+"/relay?room="+room+"&role="+role,
        header=["Authorization: Bearer "+TOKEN],timeout=15)
    WS.settimeout(.3)

def send(kind,**payload):
    nonce=secrets.token_bytes(12)
    WS.send_binary(nonce+AESGCM(KEY).encrypt(nonce,json.dumps({"type":kind,**payload}).encode(),b"release-ui-v1"))

def recv():
    try:
        packet=WS.recv()
    except websocket.WebSocketTimeoutException:
        return None
    if not isinstance(packet,bytes) or len(packet)<29: return None
    result=json.loads(AESGCM(KEY).decrypt(packet[:12],packet[12:],b"release-ui-v1"))
    if result.get("type")=="failed": raise AssertionError("Peer UI test failed: "+str(result.get("stage","unknown")))
    return result

def await_message(kind,seconds=90,repeat=None):
    end=time.monotonic()+seconds
    last=0
    while time.monotonic()<end:
        if repeat and time.monotonic()-last>2:
            send(repeat)
            last=time.monotonic()
        m=recv()
        if m and m.get("type")==kind: return m
        if not repeat and time.monotonic()-last>20:
            WS.ping()
            last=time.monotonic()
    raise AssertionError("Timed out waiting for "+kind)

class Recording:
    """Actual OS screen captures at a fixed sampling rate; no remote-frame fabrication."""
    def __init__(self,capture,name):
        self.capture=capture
        self.name=name
        self.stop=threading.Event()
        self.error=None
        self.frames=0
        self.worker=None
    def start(self):
        self.worker=threading.Thread(target=self.run,daemon=True)
        self.worker.start()
    def run(self):
        writer=None
        try:
            img=self.capture().convert("RGB")
            w,h=img.width//2*2,img.height//2*2
            writer=imageio_ffmpeg.write_frames(str(OUT/self.name),(w,h),fps=4,
                codec="libx264",pix_fmt_in="rgb24",pix_fmt_out="yuv420p",
                output_params=["-movflags","+faststart","-crf","19"])
            writer.send(None)
            while not self.stop.is_set():
                start=time.monotonic()
                img=self.capture().convert("RGB")
                # Keep original screen proportions across real portrait/landscape changes.
                img.thumbnail((w,h))
                canvas=Image.new("RGB",(w,h))
                canvas.paste(img,((w-img.width)//2,(h-img.height)//2))
                writer.send(canvas.tobytes())
                self.frames+=1
                self.stop.wait(max(0,.25-(time.monotonic()-start)))
        except Exception as e:
            self.error=type(e).__name__
        finally:
            if writer:
                try: writer.close()
                except Exception as e: self.error=type(e).__name__
    def finish(self,strict):
        self.stop.set()
        if self.worker: self.worker.join(30)
        if strict:
            require(self.worker is not None and not self.worker.is_alive() and not self.error,"OS screen recording failed")
            require(self.frames>=16,"Screen recording too short")
            count,duration=imageio_ffmpeg.count_frames_and_secs(str(OUT/self.name))
            require(count==self.frames and duration>0,"Recording decode verification failed")
            REPORT["recording"]={"file":self.name,"frames":count,"duration_seconds":duration,"capture_fps":4}
        else:
            REPORT["partial_recording_frames"]=self.frames

def adb(*args,data=None,check=True):
    return subprocess.run(["adb",*args],input=data,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=check,timeout=30)

def android_capture():
    return Image.open(io.BytesIO(adb("exec-out","screencap","-p").stdout)).convert("RGB")

def snapshot(capture,name):
    img=capture()
    img.save(OUT/(name+".png"))
    return img

def android_tree():
    # Never export raw UI XML: it may contain sensitive fields.
    adb("shell","rm","-f","/sdcard/p2p-ui.xml",check=False)
    result=adb("shell","uiautomator","dump","/sdcard/p2p-ui.xml",check=False)
    if result.returncode:
        REPORT["ui_dump_failures"]=REPORT.get("ui_dump_failures",0)+1
        return []
    raw=adb("exec-out","cat","/sdcard/p2p-ui.xml",check=False)
    try:
        nodes=list(ET.fromstring(raw.stdout).iter("node"))
        REPORT["last_ui_node_count"]=len(nodes)
        return nodes
    except ET.ParseError:
        REPORT["ui_xml_parse_failures"]=REPORT.get("ui_xml_parse_failures",0)+1
        return []

def android_foreground():
    ok,state=foreground_state(PACKAGE)
    REPORT["last_focus"]=state
    return ok

def android_nav(index,step):
    # Identical real touch positions to the successful release APK smoke test.
    # The page transition must be visible; no internal navigation call.
    require(android_foreground(),"Release Flutter activity not foreground")
    before=android_capture()
    w,h=before.size
    require(h>w,"Expected portrait release app on emulator")
    adb("shell","input","tap",str(int(w*((index+.5)/4))),str(int(h*.925)))
    time.sleep(3)
    after=snapshot(android_capture,step)
    box=(w//12,h//14,w*11//12,h*4//5)
    delta=ImageStat.Stat(ImageChops.difference(before.crop(box).resize((160,160)),
                 after.crop(box).resize((160,160)))).mean
    require(sum(delta)/3>.8 and android_foreground(),"Actual Flutter navigation did not change page")


def find_android(label,enabled=True,seconds=35,scroll=False,identifier=None):
    attempts=0
    def find():
        nonlocal attempts
        attempts+=1
        candidates=[]
        for node in android_tree():
            a=node.attrib
            if enabled and a.get("enabled")!="true": continue
            fields=[a.get("text",""),a.get("content-desc",""),a.get("resource-id","")]
            exact=any(label==v or label in v.splitlines() or (label=="p2p_remote_video" and v.endswith(label)) for v in fields)
            # Flutter often merges a row's title, subtitle and role into one label.
            if label in ("网络连接","保存中转配置","请求连接","全屏远控","远程输入文字","发送远程文字","断开连接"):
                exact=exact or any(label in v for v in fields[:2])
            # Flutter EditText accessibility names may combine label and hint.
            edit_match=a.get("class","").endswith("EditText") and any(label in v for v in fields)
            resource=a.get("resource-id","")
            id_match=bool(identifier and (resource==identifier or resource.endswith("/"+identifier)))
            if exact or edit_match or id_match:
                nums=list(map(int,re.findall(r"\d+",a.get("bounds",""))))
                if len(nums)==4 and nums[2]>nums[0] and nums[3]>nums[1]:
                    candidates.append((0 if id_match else 1 if edit_match else 2, nums))
        if candidates:
            return min(candidates,key=lambda item:item[0])[1]
        if scroll:
            w,h=android_capture().size
            # Reset to the top initially, then scan in modest increments.
            y1,y2=(.34,.79) if attempts in (1,2) else (.77,.46)
            adb("shell","input","swipe",str(int(w*.85)),str(int(h*y1)),str(int(w*.85)),str(int(h*y2)),"450")
        return None
    return wait("Android visible UI: "+label,find,seconds)

def tap_bounds(b):
    adb("shell","input","tap",str((b[0]+b[2])//2),str((b[1]+b[3])//2))

def android_tap(label,**kwargs):
    tap_bounds(find_android(label,**kwargs))
    time.sleep(.4)

def android_type(text):
    # Do not put credentials in the subprocess argument vector or log.
    # Android input text supports the configured printable ASCII bearer token.
    require(all(32<=ord(c)<127 for c in text),"CI credential must be printable ASCII")
    escaped=text.replace(" ","%s")
    cmd="input text "+shlex.quote(escaped)+"\nexit\n"
    adb("shell","-T",data=cmd.encode())

def android_field(label,value,scroll=False,identifier=None):
    android_tap(label,scroll=scroll,identifier=identifier)
    android_type(value)

def remote_point(bounds,xy):
    return [str(round(bounds[0]+xy[0]*(bounds[2]-bounds[0]))),
            str(round(bounds[1]+xy[1]*(bounds[3]-bounds[1])))]

def read_avc_metrics():
    android_tap("视频性能")
    def extract():
        fields=[]
        for node in android_tree():
            fields.extend([node.get("text",""),node.get("content-desc","")])
        for field in fields:
            if "codec=" in field and "decoder_mode=" in field:
                items=dict(re.findall(r"(?m)^([a-z_]+)=([^\n]+)",field))
                if len(items)>=10:
                    return items
        return None
    data=wait("Actual release video diagnostic data",extract,25)
    android_tap("关闭性能数据")
    for key in ("frames","bps","rtt","target_bps","target_fps","drops"):
        data[key]=int(data[key])
    data["fps"]=float(data["fps"])
    require(data["codec"]=="H264" and data["frames"]>0,"Actual MediaCodec AVC playback not verified")
    require(data["encoder"]=="OpenH264 software" and data["decoder"] not in ("","unknown"),"Actual encoder and decoder not reported")
    require(data["decoder_mode"] in ("hardware","software","unknown"),"Invalid decoder classification")
    return data

def android():
    global STAGE
    rec=None
    success=False
    try:
        STAGE="install_release_apk"
        apk=next(Path("apk").rglob("app-release.apk"))
        adb("install","-r",str(apk))
        # Open the coordinator before startup so peer receives explicit failures.
        rendezvous("viewer")
        try:
            start_release_app(PACKAGE,REPORT)
        except Exception:
            snapshot(android_capture,"00-startup-focus-failed")
            raise
        snapshot(android_capture,"00-home-before-credential")
        STAGE="configure_via_flutter_ui"
        android_nav(3,"00-settings-before-credential")
        android_tap("网络连接",identifier="p2p_settings_network",scroll=True)
        # Safe pre-credential evidence; do not record the secret entry.
        snapshot(android_capture,"00-network-before-credential")
        android_field("Relay Access Token",TOKEN,scroll=True,identifier="p2p_relay_token")
        adb("shell","input","keyevent","4")
        android_tap("保存中转配置",scroll=True)
        find_android("已通过 Android Keystore 加密保存",enabled=False,scroll=True,
                     identifier="p2p_relay_token_saved",seconds=55)
        android_nav(2,"00-remote-after-credential-saved")
        passed("Installed release Flutter APK and configured public relay through actual UI")
        STAGE="pairing"
        pair=await_message("pair",seconds=240,repeat="ready")
        rec=Recording(android_capture,"Android-Release-UI.mp4")
        rec.start()
        snapshot(android_capture,"01-pairing-form")
        android_field("9位设备号",pair["number"],identifier="p2p_remote_device_number")
        android_field("6位动态验证码",pair["code"],identifier="p2p_remote_pairing_code")
        adb("shell","input","keyevent","4")
        android_tap("请求连接")
        send("pair_submitted")
        # Full-screen button exists only after native engine receives authenticated video.
        android_tap("全屏远控",seconds=65,scroll=True,identifier="p2p_remote_fullscreen")
        send("connected")
        find_android("远程输入文字",seconds=45)
        find_android("p2p_remote_video",enabled=False)
        passed("Real numeric pairing and first video in the release Flutter UI")
        require(android_capture().width>android_capture().height,"Android full-screen did not rotate to landscape")
        STAGE="desktop"
        targets=await_message("targets",seconds=60)
        time.sleep(2)
        b=find_android("p2p_remote_video",enabled=False)
        snapshot(android_capture,"02-fullscreen-windows-desktop")
        adb("shell","input","tap",*remote_point(b,targets["click"]))
        send("click_sent")
        await_message("click_ok")
        passed("Touch on remote desktop reached actual Windows Notepad")
        STAGE="drag"
        b=find_android("p2p_remote_video",enabled=False)
        adb("shell","input","swipe",*remote_point(b,targets["drag_from"]),
            *remote_point(b,targets["drag_to"]),"1400")
        send("drag_sent")
        moved=await_message("drag_ok")
        time.sleep(1)
        snapshot(android_capture,"03-notepad-moved")
        passed("Actual Notepad window dragged remotely")
        STAGE="text"
        b=find_android("p2p_remote_video",enabled=False)
        adb("shell","input","tap",*remote_point(b,moved["edit"]))
        android_tap("远程输入文字")
        android_field("远程文字内容",MARKER,identifier="p2p_remote_text")
        text_started=time.monotonic()
        android_tap("发送远程文字")
        send("text_sent")
        await_message("text_ok")
        REPORT["text_ui_to_host_confirmation_ms"]=round((time.monotonic()-text_started)*1000)
        # Includes automation and rendezvous acknowledgment; not glass-to-glass latency.
        time.sleep(2)
        snapshot(android_capture,"04-notepad-remote-text")
        passed("Remote text verified by Windows independently")
        STAGE="actual_avc_performance"
        # Read only user-visible diagnostics backed by actual decoder callbacks.
        # Existing functional assertions and videos remain mandatory.
        first=read_avc_metrics()
        time.sleep(10)
        second=read_avc_metrics()
        require(second["frames"]>first["frames"] and second["fps"]>0 and second["bps"]>0
                and second["rtt"]>=0,"No measured live AVC progress")
        REPORT["actual_avc_metrics"]={"first":first,"second":second,
            "scope":"MediaCodec rendered output, received AVC payload bitrate, authenticated RTT; no synthetic data"}
        passed("Live AVC rendering, actual throughput and RTT measured in landscape release APK")
        STAGE="disconnect"
        android_tap("断开连接")
        find_android("请求连接",enabled=False,seconds=30,scroll=True)
        snapshot(android_capture,"05-disconnected")
        passed("Disconnected from full-screen actual app")
        STAGE="recording"
        rec.finish(True)
        rec=None
        STAGE="host_confirmation"
        send("done")
        await_message("ack")
        success=True
        REPORT["passed"]=True
    finally:
        if rec: rec.finish(False)
        # Clear test-only credential from the disposable installed app after capture.
        adb("shell","am","force-stop",PACKAGE,check=False)
        adb("shell","pm","clear",PACKAGE,check=False)
        adb("shell","rm","-f","/sdcard/p2p-ui.xml",check=False)
        if not success: REPORT["passed"]=False

def windows():
    global STAGE
    import ctypes
    import pyautogui as gui
    import pyperclip
    from pywinauto import Desktop
    from ctypes import wintypes
    # UI automation is limited to the isolated GitHub Windows desktop.
    require(os.name=="nt","Windows runner required")
    ctypes.windll.user32.SetProcessDPIAware()
    rec=None
    proc=None
    note=None
    window=None
    success=False
    desktop=Desktop(backend="uia")
    def capture(): return gui.screenshot().convert("RGB")
    def find_window(title):
        matches=[w for w in gui.getAllWindows() if title in w.title and w.width>200]
        return matches[0] if matches else None
    def find_control(label,seconds=30,scroll=False,enabled=True,identifier=None,prefer_edit=False):
        def lookup():
            try:
                parent=desktop.window(handle=window._hWnd)
                candidates=[]
                for obj in parent.descendants():
                    name=obj.element_info.name or ""
                    automation_id=obj.element_info.automation_id or ""
                    id_match=bool(identifier and automation_id==identifier)
                    edit_match=obj.element_info.control_type=="Edit" and label in name
                    if id_match or edit_match or label==name or label in name.splitlines():
                        r=obj.rectangle()
                        if r.width()>2 and r.height()>2 and obj.is_visible() and (not enabled or obj.is_enabled()):
                            candidates.append((obj,id_match))
                if candidates:
                    candidates.sort(key=lambda a: (
                        (a[0].element_info.control_type!="Edit") if prefer_edit else not a[1],
                        not a[1],
                        a[0].element_info.control_type not in ("Button","Edit","CheckBox"),
                        a[0].rectangle().width()*a[0].rectangle().height()))
                    return candidates[0][0]
            except Exception:
                pass
            if scroll:
                gui.moveTo(window.left+window.width*3//4,window.top+window.height*3//4)
                gui.scroll(-3)
            return None
        return wait("Windows visible UI: "+label,lookup,seconds)
    def click(label,**kwargs):
        obj=find_control(label,**kwargs)
        r=obj.rectangle()
        gui.click((r.left+r.right)//2,(r.top+r.bottom)//2)
        time.sleep(.5)
    def clipboard_from(label,pattern):
        pyperclip.copy("")
        click(label,scroll=True)
        value=wait("Clipboard copied from UI",lambda: (v if re.fullmatch(pattern,v:=pyperclip.paste().strip()) else None),8)
        pyperclip.copy("")
        return value
    def normal(x,y):
        w,h=gui.size()
        require(0<=x<w and 0<=y<h,"Remote test target outside desktop")
        return [x/w,y/h]
    def rect(w):
        return [w.left,w.top,w.width,w.height]
    try:
        STAGE="install_release_exe"
        setup=next(Path("windows-package").rglob("P2PRemote-Setup-v0.21.0.exe"))
        installer=subprocess.run([str(setup.resolve()),"/S"],timeout=120)
        REPORT["installer_exit_code"]=installer.returncode
        print("Release NSIS installer exit code:",installer.returncode,flush=True)
        require(installer.returncode==0,"Release NSIS installer failed with exit code "+str(installer.returncode))
        exe=Path(os.environ["LOCALAPPDATA"])/"Programs"/"P2P Remote"/"p2p_remote_ui.exe"
        require(exe.is_file(),"Installed release executable missing")
        require((exe.parent/"p2p_remote.dll").is_file(),"Installed native remote bridge missing")
        require((exe.parent/"Uninstall.exe").is_file(),"Installed uninstaller missing")
        passed("Real NSIS installer exited successfully and installed application, native bridge and uninstaller")
        proc=subprocess.Popen([str(exe)],cwd=exe.parent)
        window=wait("Actual installed P2P window",lambda:find_window("P2P Remote"),40)
        window.maximize()
        window.activate()
        time.sleep(3)
        STAGE="configure_via_flutter_ui"
        click("网络连接")
        snapshot(capture,"00-network-before-credential")
        click("Relay Access Token",identifier="p2p_relay_token",prefer_edit=True)
        gui.hotkey("ctrl","a")
        # ASCII hex/alphanumeric secrets can be typed without clipboard timing issues.
        # For other printable credentials, retain the clipboard until native save completes.
        if re.fullmatch(r"[A-Za-z0-9]+",TOKEN):
            gui.write(TOKEN,interval=.018)
        else:
            pyperclip.copy(TOKEN)
            gui.hotkey("ctrl","v")
            time.sleep(2)
        click("保存中转配置",scroll=True)
        find_control("已通过 DPAPI 加密保存",enabled=False,scroll=True,seconds=55,
                     identifier="p2p_relay_token_saved")
        pyperclip.copy("")
        click("首页")
        passed("Installed release Windows EXE and configured public relay through actual UI")
        STAGE="pairing"
        rendezvous("host")
        await_message("ready",seconds=300)
        rec=Recording(capture,"Windows-Release-UI.mp4")
        rec.start()
        snapshot(capture,"01-host-home")
        click("启动本机共享")
        number=clipboard_from("复制设备号",r"\d{9}")
        code=clipboard_from("复制验证码",r"\d{6}")
        snapshot(capture,"02-host-pairing-code-masked")
        # Repeat the encrypted delivery until the viewer submits its UI form.
        end=time.monotonic()+70
        last=0
        while time.monotonic()<end:
            if time.monotonic()-last>2:
                send("pair",number=number,code=code)
                last=time.monotonic()
            m=recv()
            if m and m.get("type")=="pair_submitted": break
        else: raise AssertionError("Android did not submit pairing form")
        # The real approval button is clicked, not a bridge command.
        find_control("允许本次连接",scroll=True,seconds=35)
        snapshot(capture,"03-real-approval-request")
        click("允许本次连接",scroll=True)
        await_message("connected",seconds=75)
        passed("Device number/OTP copied from UI and actual pairing approved")
        STAGE="authorize_input"
        click("安全与权限")
        click("允许远程鼠标和键盘")
        snapshot(capture,"04-input-authorized")
        # Keep the real Host running while displaying an ordinary Windows desktop.
        window.minimize()
        STAGE="notepad"
        file=Path(os.environ["RUNNER_TEMP"])/("P2P-Remote-QA-"+os.environ["GITHUB_RUN_ID"]+".txt")
        file.write_text("",encoding="utf-8")
        note_proc=subprocess.Popen(["notepad.exe",str(file)])
        note=wait("Actual Windows Notepad",lambda:find_window(file.stem),40)
        w,h=gui.size()
        note.restore()
        # A real resizable Notepad window with space to drag it on the desktop.
        nw,nh=int(w*.57),int(h*.56)
        note.resizeTo(nw,nh)
        note.moveTo(int(w*.10),int(h*.14))
        note.activate()
        time.sleep(2)
        initial=rect(note)
        require(initial[2]>250 and initial[3]>180,"Notepad window too small")
        # Use actual OS window coordinates, not a fabricated remote image.
        x,y,ww,hh=initial
        clickxy=normal(x+int(ww*.35),y+int(hh*.55))
        dragfrom=normal(x+int(ww*.68),y+15)
        dragto=normal(x+int(ww*.68)+int(w*.16),y+15+int(h*.12))
        snapshot(capture,"05-normal-windows-desktop")
        send("targets",click=clickxy,drag_from=dragfrom,drag_to=dragto)
        STAGE="remote_click"
        await_message("click_sent")
        def observed_click():
            pos=gui.position()
            return abs(pos.x-clickxy[0]*w)<22 and abs(pos.y-clickxy[1]*h)<22 and ctypes.windll.user32.GetForegroundWindow()==note._hWnd
        wait("Actual Notepad foreground and remote pointer position",observed_click,12)
        send("click_ok")
        passed("Actual remote mouse reached Notepad window")
        STAGE="remote_drag"
        await_message("drag_sent")
        def moved():
            current=rect(note)
            return current if abs(current[0]-initial[0])+abs(current[1]-initial[1])>=60 else None
        new=wait("Notepad window moved by remote drag",moved,15)
        REPORT["notepad_window_before"]=initial
        REPORT["notepad_window_after"]=new
        snapshot(capture,"06-notepad-dragged")
        send("drag_ok",edit=normal(new[0]+int(new[2]*.35),new[1]+int(new[3]*.55)))
        passed("Actual Notepad window position changed")
        STAGE="remote_text"
        await_message("text_sent")
        # Independent OS readback: local selection/copy only, no text injection.
        def readback():
            if ctypes.windll.user32.GetForegroundWindow()!=note._hWnd: return False
            pyperclip.copy("")
            gui.hotkey("ctrl","a")
            gui.hotkey("ctrl","c")
            time.sleep(.25)
            value=pyperclip.paste()
            pyperclip.copy("")
            return value.strip()==MARKER
        wait("Exact text typed into Notepad by Android",readback,15)
        # Remove selection visually so the actual typed text is legible in evidence.
        gui.press("end")
        snapshot(capture,"07-real-notepad-text")
        send("text_ok")
        passed("Exact remote text read back from actual Notepad")
        STAGE="host_confirmation"
        await_message("done",seconds=90)
        # Recording must be valid before the peer receives final success.
        rec.finish(True)
        rec=None
        send("ack")
        success=True
        REPORT["passed"]=True
    finally:
        if rec: rec.finish(False)
        pyperclip.copy("")
        if note:
            # Disposable CI document only. Do not save user content.
            try: note.close()
            except Exception: pass
        if proc and proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=15)
        if not success: REPORT["passed"]=False

try:
    {"windows":windows,"android":android}[ROLE]()
except Exception as exc:
    REPORT["failed_stage"]=STAGE
    REPORT["error_type"]=type(exc).__name__
    if isinstance(exc,AssertionError): REPORT["error"]=str(exc)
    print("FAIL",ROLE,STAGE,REPORT.get("error",REPORT["error_type"]),flush=True)
    if WS:
        try: send("failed",stage=STAGE)
        except Exception: pass
finally:
    if WS:
        try: WS.close()
        except Exception: pass
    (OUT/"report.json").write_text(json.dumps(REPORT,indent=2,ensure_ascii=False),encoding="utf-8")
    print("PASS" if REPORT["passed"] else "FAILED",ROLE,flush=True)
sys.exit(0 if REPORT["passed"] else 1)
