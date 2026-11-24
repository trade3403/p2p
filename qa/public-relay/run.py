"""Cross-runner encrypted rendezvous for a real Windows Host and Android native viewer.
Only ephemeral CI identities; no tokens/codes/room IDs in logs or artifacts.
"""
import ctypes
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time
import urllib.request
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import websocket

HOST = "p2ptest.aikissgo.com"
TOKEN = os.environ.get("RELAY_ACCESS_TOKEN", "").strip()
OUT = Path("evidence/public-relay-" + sys.argv[1])
OUT.mkdir(parents=True, exist_ok=True)
ROLE = sys.argv[1]
REPORT = {"role": ROLE, "passed": False}
STAGE = "setup"
WS = None
KEY = None

def require(ok, reason):
    if not ok:
        raise AssertionError(reason)

def rendezvous(role):
    global KEY
    require(os.environ.get("GITHUB_ACTIONS") == "true", "CI only")
    require(32 <= len(TOKEN) <= 512, "Configure RELAY_ACCESS_TOKEN repository Actions Secret")
    with urllib.request.urlopen("https://" + HOST + "/healthz", timeout=15) as response:
        require(response.status == 200 and response.read().strip() == b"ok", "HTTPS healthz failed")
    context = "|".join(["p2p-public-ci-v1",os.environ["GITHUB_REPOSITORY"],
                        os.environ["GITHUB_RUN_ID"],os.environ["GITHUB_RUN_ATTEMPT"]])
    KEY = hmac.digest(TOKEN.encode(),context.encode(),"sha256")
    room = hashlib.sha256(b"p2p-ci-coordination:"+KEY).hexdigest()
    ws = websocket.create_connection("wss://"+HOST+"/relay?room="+room+"&role="+role,
                                     header=["Authorization: Bearer "+TOKEN],timeout=12)
    ws.settimeout(.2)
    return ws

def send(value):
    nonce = secrets.token_bytes(12)
    WS.send_binary(nonce + AESGCM(KEY).encrypt(nonce,json.dumps(value).encode(),b"p2p-ci-v1"))

def receive():
    try:
        raw=WS.recv()
    except websocket.WebSocketTimeoutException:
        return None
    if not isinstance(raw,bytes) or len(raw)<29:
        return None
    return json.loads(AESGCM(KEY).decrypt(raw[:12],raw[12:],b"p2p-ci-v1"))

class Bridge:
    def __init__(self):
        self.dll=ctypes.CDLL(str(Path("desktop/target/release/p2p_remote.dll").resolve()))
        self.dll.p2p_bridge_init.restype=ctypes.c_int
        self.dll.p2p_bridge_command_json.argtypes=[ctypes.c_char_p]
        self.dll.p2p_bridge_command_json.restype=ctypes.c_void_p
        self.dll.p2p_bridge_free_string.argtypes=[ctypes.c_void_p]
        self.dll.p2p_bridge_free_string.restype=None
        self.dll.p2p_bridge_shutdown.restype=None
        require(self.dll.p2p_bridge_init()==1,"Native host initialization failed")
    def cmd(self,method,**args):
        p=self.dll.p2p_bridge_command_json(json.dumps({"method":method,**args}).encode())
        require(p,"Native command returned null")
        try:
            result=json.loads(ctypes.string_at(p))
        finally:
            self.dll.p2p_bridge_free_string(p)
        require(result.get("ok"),"Native host command failed: "+method)
        return result["snapshot"]

def host():
    global STAGE, WS
    import tkinter as tk
    require(os.name=="nt","Windows runner required")
    ctypes.windll.user32.SetProcessDPIAware()
    counts={"clicks":0,"keys":0}
    colors={"ready":"#19559b","clicked":"#209148","key":"#9b4823","recovered":"#5f2d91"}
    root=tk.Tk()
    root.attributes("-fullscreen",True)
    root.attributes("-topmost",True)
    canvas=tk.Canvas(root,highlightthickness=0)
    canvas.pack(fill="both",expand=True)
    def draw(state):
        canvas.delete("all")
        canvas.configure(bg=colors[state])
        w,h=root.winfo_screenwidth(),root.winfo_screenheight()
        for x in (.08,.92):
            for y in (.08,.92):
                canvas.create_rectangle(w*x-32,h*y-32,w*x+32,h*y+32,fill="#e6be23",outline="")
        canvas.create_text(w*.5,h*.35,text="P2P PUBLIC RELAY E2E",fill="white",font=("Arial",28,"bold"))
        canvas.create_text(w*.5,h*.5,text=state.upper(),fill="white",font=("Arial",24))
        canvas.create_text(w*.5,h*.65,text="Synthetic CI test desktop",fill="white",font=("Arial",16))
    def click(_):
        counts["clicks"]+=1
        draw("clicked" if counts["clicks"]==1 else "recovered")
    def key(_):
        counts["keys"]+=1
        draw("key")
    canvas.bind("<Button-1>",click)
    root.bind("<Return>",key)
    draw("ready")
    root.update()
    root.focus_force()
    canvas.focus_set()
    profile=tempfile.TemporaryDirectory(prefix="p2p-ci-profile-")
    os.environ["APPDATA"]=profile.name
    os.environ["COMPUTERNAME"]="P2P-CI-"+os.environ.get("GITHUB_RUN_ID","test")
    bridge=None
    try:
        STAGE="rendezvous"
        WS=rendezvous("host")
        STAGE="registration"
        bridge=Bridge()
        bridge.cmd("configureRelay",server="wss://"+HOST,token=TOKEN)
        bridge.cmd("startHost")
        approved=False
        enabled=False
        offered=False
        end=time.monotonic()+32*60
        last=0
        while time.monotonic()<end:
            root.update()
            hwnd=ctypes.windll.user32.GetAncestor(root.winfo_id(),2)
            if enabled:
                require(hwnd and ctypes.windll.user32.GetForegroundWindow()==hwnd,
                        "Synthetic host desktop lost foreground; stop remote input")
            message=receive()
            now=time.monotonic()
            if message and message.get("type")=="ready" and not approved:
                s=bridge.cmd("snapshot")
                if s.get("pairing_code") and s.get("pairing_expires_at",0)>time.time()+45:
                    send({"type":"pair","number":s["device_number"],"code":s["pairing_code"]})
                    offered=True
            if offered and not approved and now-last>.5:
                s=bridge.cmd("snapshot")
                if s.get("pending_pairing_request_id"):
                    # Only the fresh CI identity and its privately delivered OTP are approved.
                    bridge.cmd("approvePairingRequest",requestId=s["pending_pairing_request_id"])
                    approved=True
                    STAGE="waiting_for_android"
                last=now
            if message and message.get("type")=="enable" and approved and not enabled:
                require(counts["clicks"]==0 and counts["keys"]==0,"Input occurred before permission")
                require(hwnd and ctypes.windll.user32.GetForegroundWindow()==hwnd,
                        "No interactive test desktop")
                bridge.cmd("setRemoteInputAllowed",allowed=True)
                bridge.cmd("setClipboardAllowed",allowed=True)
                enabled=True
            if message and message.get("type")=="done":
                require(approved and enabled and counts=={"clicks":2,"keys":1},
                        "Host did not observe exactly two remote clicks and one key")
                REPORT["observed_input"]=counts
                REPORT["passed"]=True
                send({"type":"ack"})
                return
            if message and message.get("type")=="failed":
                raise AssertionError("Android E2E failed; inspect Android evidence")
        raise AssertionError("Timed out waiting for Android runner")
    finally:
        if bridge:
            try:
                bridge.cmd("setRemoteInputAllowed",allowed=False)
                bridge.cmd("setClipboardAllowed",allowed=False)
                bridge.cmd("stopHost")
            finally:
                bridge.dll.p2p_bridge_shutdown()
        root.destroy()
        REPORT["observed_input"]=counts
        profile.cleanup()

def adb(*args, data=None, check=True):
    return subprocess.run(["adb",*args],input=data,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=check)

def pull_evidence():
    listing=adb("shell","run-as","com.p2p.remote","ls","files/e2e",check=False)
    for name in listing.stdout.decode(errors="replace").splitlines():
        name=name.strip()
        if not (name in ("report.json","phase.txt","ready.jpg","clicked.jpg","key.jpg","recovered.jpg")
                or (name.startswith("frame-") and name.endswith(".jpg") and name[6:-4].isdigit())):
            continue
        data=adb("exec-out","run-as","com.p2p.remote","cat","files/e2e/"+name,check=False)
        if data.returncode==0 and data.stdout:
            (OUT/name).write_bytes(data.stdout)
    if (OUT/"frame-00000.jpg").exists():
        result=subprocess.run(["ffmpeg","-hide_banner","-loglevel","error","-y","-framerate","5",
            "-i",str(OUT/"frame-%05d.jpg"),"-c:v","libx264","-pix_fmt","yuv420p",
            str(OUT/"Android-Received-Remote-Video.mp4")],capture_output=True)
        require(result.returncode==0,"Could not encode received video MP4")
    for frame in OUT.glob("frame-*.jpg"):
        frame.unlink()

def android():
    global STAGE,WS
    STAGE="install"
    adb("install","-r","android/app/build/outputs/apk/debug/app-debug.apk")
    adb("install","-r","android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk")
    STAGE="rendezvous"
    WS=rendezvous("viewer")
    until=time.monotonic()+20*60
    pair=None
    last=0
    while time.monotonic()<until:
        if time.monotonic()-last>2:
            send({"type":"ready"})
            last=time.monotonic()
        m=receive()
        if m and m.get("type")=="pair":
            pair=m
            break
    require(pair,"Host did not provide a current CI pairing window")
    config={"server":"wss://"+HOST,"token":TOKEN,"number":pair["number"],"code":pair["code"]}
    # Transfer over adb stdin into the disposable app's private directory.
    # Never pass credentials as CLI arguments, test properties or artifacts.
    adb("shell","-T","run-as com.p2p.remote sh -c 'mkdir -p files; cat > files/ci-private.json'",
        data=json.dumps(config).encode())
    STAGE="native_instrumentation"
    with (OUT/"instrumentation.txt").open("wb") as log:
        process=subprocess.Popen(["adb","shell","am","instrument","-w","-r","-e","class",
            "com.p2p.remote.PublicRelayE2eTest","com.p2p.remote.test/androidx.test.runner.AndroidJUnitRunner"],
            stdout=log,stderr=subprocess.STDOUT)
        end=time.monotonic()+240
        try:
            while process.poll() is None and time.monotonic()<end:
                phase=adb("shell","run-as","com.p2p.remote","cat","files/e2e/phase.txt",check=False).stdout.strip()
                if phase==b"enable":
                    send({"type":"enable"})
                time.sleep(1)
            require(process.poll() is not None,"Android instrumentation timed out")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            pull_evidence()
    native=json.loads((OUT/"report.json").read_text())
    require(process.returncode==0 and native.get("passed") is True,"Android native E2E assertions failed")
    require((OUT/"Android-Received-Remote-Video.mp4").stat().st_size>0,"No recorded received video")
    REPORT["metrics"]=native["metrics"]
    STAGE="host_confirmation"
    end=time.monotonic()+15
    while time.monotonic()<end:
        send({"type":"done"})
        m=receive()
        if m and m.get("type")=="ack":
            REPORT["passed"]=True
            return
        time.sleep(.5)
    raise AssertionError("Host did not confirm observed input")

try:
    {"host":host,"android":android}[ROLE]()
except Exception as exc:
    REPORT["failed_stage"]=STAGE
    REPORT["error_type"]=type(exc).__name__
    if isinstance(exc,AssertionError):
        REPORT["error"]=str(exc)
    if ROLE=="android" and WS:
        try: send({"type":"failed"})
        except Exception: pass
    print("FAIL",ROLE,STAGE,REPORT.get("error",REPORT["error_type"]),flush=True)
finally:
    if WS:
        try: WS.close()
        except Exception: pass
    (OUT/"job-report.json").write_text(json.dumps(REPORT,indent=2),encoding="utf-8")
    print("PASS" if REPORT["passed"] else "FAILED",ROLE,flush=True)
sys.exit(0 if REPORT["passed"] else 1)
