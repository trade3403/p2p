"""Cross-runner encrypted rendezvous for a real Windows Host and Android native viewer.
Only ephemeral CI identities; no tokens/codes/room IDs in logs or artifacts.
"""
import ctypes
import hashlib
import hmac
import json
import os
import queue
import threading
import re
import struct
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
COORD_INBOX = queue.Queue(maxsize=256)
COORD_STOP = threading.Event()
COORD_READER = None
COORD_ERROR = None

def require(ok, reason):
    if not ok:
        raise AssertionError(reason)

def rendezvous(role):
    global KEY, COORD_READER
    require(os.environ.get("GITHUB_ACTIONS") == "true", "CI only")
    require(32 <= len(TOKEN) <= 512, "Configure RELAY_ACCESS_TOKEN repository Actions Secret")
    with urllib.request.urlopen("https://" + HOST + "/healthz", timeout=15) as response:
        require(response.status == 200 and response.read().strip() == b"ok", "HTTPS healthz failed")
    context = "|".join(["p2p-public-ci-v1",os.environ["GITHUB_REPOSITORY"],
                        os.environ["GITHUB_RUN_ID"],os.environ["GITHUB_RUN_ATTEMPT"]])
    KEY = hmac.digest(TOKEN.encode(),context.encode(),"sha256")
    room = hashlib.sha256(b"p2p-ci-coordination:"+KEY).hexdigest()
    ws = websocket.create_connection("wss://"+HOST+"/relay?room="+room+"&role="+role,
                                     header=["Authorization: Bearer "+TOKEN],timeout=12,
                                      enable_multithread=True)
    ws.settimeout(.2)
    # The server renews its read deadline on PONG. Keep reading even during
    # instrumentation, ADB evidence collection and video encoding.
    COORD_READER=threading.Thread(target=coordinator_reader,args=(ws,),
                                  name="public-ci-coordinator",daemon=True)
    COORD_READER.start()
    return ws

def send(value):
    nonce = secrets.token_bytes(12)
    WS.send_binary(nonce + AESGCM(KEY).encrypt(nonce,json.dumps(value).encode(),b"p2p-ci-v1"))

def coordinator_reader(ws):
    global COORD_ERROR
    try:
        while not COORD_STOP.is_set():
            try:
                raw=ws.recv()
            except websocket.WebSocketTimeoutException:
                continue
            if raw=="" or raw==b"":
                if not COORD_STOP.is_set():
                    COORD_ERROR="Coordinator closed"
                return
            if not isinstance(raw,bytes) or len(raw)<29:
                continue
            value=json.loads(AESGCM(KEY).decrypt(raw[:12],raw[12:],b"p2p-ci-v1"))
            COORD_INBOX.put_nowait(value)
    except Exception as exc:
        if not COORD_STOP.is_set():
            COORD_ERROR=type(exc).__name__

def receive():
    try:
        return COORD_INBOX.get(timeout=.2)
    except queue.Empty:
        if COORD_ERROR:
            raise AssertionError("Test coordinator reader failed: "+COORD_ERROR)
        return None

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
        REPORT["pairing_progress"]={"offers":0,"approved":False,"pending_seen":False}
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
                remaining=int(s.get("pairing_expires_at",0)-time.time())
                if s.get("pairing_code") and remaining>=75:
                    send({"type":"pair","number":s["device_number"],"code":s["pairing_code"],
                          "expires_at":s["pairing_expires_at"]})
                    offered=True
                    REPORT["pairing_progress"]["offers"]+=1
                    REPORT["pairing_progress"]["offer_remaining_seconds"]=remaining
                    STAGE="pairing_offer_sent"
            if offered and not approved and now-last>.5:
                s=bridge.cmd("snapshot")
                REPORT["pairing_progress"]["host_running"]=bool(s.get("host_running"))
                REPORT["pairing_progress"]["window_remaining_seconds"]=max(0,int(s.get("pairing_expires_at",0)-time.time()))
                if s.get("pending_pairing_request_id"):
                    REPORT["pairing_progress"]["pending_seen"]=True
                    # Only the fresh CI identity and its privately delivered OTP are approved.
                    bridge.cmd("approvePairingRequest",requestId=s["pending_pairing_request_id"])
                    approved=True
                    REPORT["pairing_progress"]["approved"]=True
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
                require(approved and enabled and counts=={"clicks":3,"keys":2},
                        "Host did not observe exactly three remote clicks and two keys")
                stats=bridge.cmd("snapshot")
                REPORT["video_transport"]={key:stats[key] for key in (
                    "host_lan_video_bytes","host_relay_video_bytes","host_lan_video_frames",
                    "host_relay_video_frames","host_relay_suppressed_frames")}
                require(REPORT["video_transport"]["host_relay_video_bytes"]>0,
                        "Host did not record encrypted relay video sends")
                require(REPORT["video_transport"]["host_lan_video_bytes"]==0,
                        "Unexpected real host LAN video in emulator-local fixture test")
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
    # Only milestone screenshots and structured reports; no video frame sequence.
    listing=adb("shell","run-as","com.p2p.remote","ls","files/e2e",check=False)
    for name in listing.stdout.decode(errors="replace").splitlines():
        name=name.strip()
        if name not in ("report.json","phase.txt","ready.jpg","clicked.jpg","key.jpg","recovered.jpg"):
            continue
        data=adb("exec-out","run-as","com.p2p.remote","cat","files/e2e/"+name,check=False)
        if data.returncode==0 and data.stdout:
            (OUT/name).write_bytes(data.stdout)
def make_lan_avc_fixture(ffmpeg,width=320,height=240):
    # Real encoded test pattern, not Windows capture or measured host performance.
    # AUD starts each access unit; closed GOPs repeat SPS/PPS for safe looping.
    result=subprocess.run([ffmpeg,"-hide_banner","-loglevel","error","-f","lavfi",
        "-i",f"testsrc2=size={width}x{height}:rate=10","-frames:v","20",
        "-c:v","libx264","-preset","ultrafast","-tune","zerolatency",
        "-pix_fmt","yuv420p","-x264-params","aud=1:repeat-headers=1:keyint=10:min-keyint=10:scenecut=0:open-gop=0:bframes=0",
        "-f","h264","pipe:1"],capture_output=True,check=True)
    raw=result.stdout
    starts=list(re.finditer(rb"\x00\x00(?:\x00)?\x01",raw))
    aud=[m.start() for m in starts if m.end()<len(raw) and raw[m.end()]&31==9]
    require(len(aud)==20 and aud[0]==0,"Unexpected AVC fixture access unit boundaries")
    units=[raw[a:b] for a,b in zip(aud,aud[1:]+[len(raw)])]
    for index in (0,10):
        nals=[units[index][m.end()]&31 for m in re.finditer(rb"\x00\x00(?:\x00)?\x01",units[index])
              if m.end()<len(units[index])]
        require(all(n in nals for n in (7,8,5)),"AVC fixture GOP lacks SPS/PPS/IDR")
    payload=b"".join(struct.pack(">IHH",len(unit)+4,width,height)+unit for unit in units)
    require(len(payload)<2*1024*1024,"AVC fixture unexpectedly large")
    return payload


def candidate_aliases(enable, installed, interface=None):
    # Root commands are restricted to the disposable hosted emulator; never
    # touch the Linux host network or a physical Android device.
    require(os.environ.get("GITHUB_ACTIONS")=="true" and
            os.environ.get("RUNNER_ENVIRONMENT")=="github-hosted" and
            os.environ.get("RUNNER_OS")=="Linux","CI emulator alias setup only")
    require(adb("shell","getprop","ro.kernel.qemu").stdout.strip()==b"1",
            "Network alias setup requires an emulator")
    if enable:
        require(interface and re.fullmatch(r"[A-Za-z0-9_.-]{1,15}",interface) and interface!="lo",
                "Invalid emulator interface for candidate test")
        require(adb("shell","su","0","id","-u").stdout.strip()==b"0",
                "Disposable emulator root shell unavailable")
        for address,device in (("10.203.71.1/24",interface),("10.203.71.2/32","lo"),
                               ("10.203.71.3/32","lo")):
            result=adb("shell","su","0","ip","addr","add",address,"dev",device,check=False)
            require(result.returncode==0,"Cannot install isolated emulator test IPv4 alias")
            installed.append((address,device))
    else:
        failures=[]
        for address,device in reversed(installed):
            result=adb("shell","su","0","ip","addr","del",address,"dev",device,check=False)
            if result.returncode: failures.append(address)
        installed.clear()
        require(not failures,"Cannot remove emulator test IPv4 alias")

class ReadmissionRelayFixture:
    """Independent ephemeral public room; actual encrypted test-pattern AVC.
    Never sends input or exposes root/room/token. Not a Windows Host."""
    def __init__(self, root, fixture):
        self.root=root
        self.units=[]
        offset=0
        while offset<len(fixture):
            size=struct.unpack_from(">I",fixture,offset)[0]
            self.units.append(fixture[offset+4:offset+4+size])
            offset+=4+size
        require(len(self.units)==20 and offset==len(fixture),"Invalid recovery fixture")
        self.stop=threading.Event()
        self.ready=threading.Event()
        self.error=None
        self.thread=threading.Thread(target=self.run,daemon=True)

    def run(self):
        ws=None
        try:
            room=hashlib.sha256(b"p2p-v02-room:"+self.root).hexdigest()
            ws=websocket.create_connection("wss://"+HOST+"/relay?room="+room+"&role=host",
                header=["Authorization: Bearer "+TOKEN],timeout=12,enable_multithread=True)
            ws.settimeout(.05)
            aes=AESGCM(self.root)
            def packet(kind,body):
                aad=bytes([kind])
                nonce=secrets.token_bytes(12)
                ws.send_binary(aad+nonce+aes.encrypt(nonce,body,aad))
            meta=json.dumps({"count":1,"selected":0,"allow_input":False,
                "allow_clipboard":False,"lan_readmission_v5":True,
                "lan_ipv4_candidates":["10.203.71.3"]}).encode()
            self.ready.set()
            index=0
            next_frame=next_meta=time.monotonic()
            while not self.stop.is_set():
                now=time.monotonic()
                if now>=next_meta:
                    packet(4,meta)
                    next_meta=now+.5
                if now>=next_frame:
                    packet(5,self.units[index])
                    index=(index+1)%len(self.units)
                    next_frame=now+.1
                try:
                    raw=ws.recv()
                except websocket.WebSocketTimeoutException:
                    continue
                if isinstance(raw,bytes) and len(raw)>=30 and raw[0]==8:
                    try:
                        body=aes.decrypt(raw[1:13],raw[13:],raw[:1])
                    except Exception:
                        continue
                    if len(body)==8: packet(9,body)
        except Exception as exc:
            if not self.stop.is_set(): self.error=type(exc).__name__
        finally:
            self.ready.set()
            if ws:
                try: ws.close()
                except Exception: pass

    def close(self):
        self.stop.set()
        self.thread.join(timeout=15)
        require(not self.thread.is_alive(),"Recovery fixture did not stop")
        require(self.error is None,"Recovery public fixture failed")


def android():
    global STAGE,WS
    STAGE="video_encoder_preflight"
    import imageio_ffmpeg
    encoder=subprocess.run([imageio_ffmpeg.get_ffmpeg_exe(),"-hide_banner","-encoders"],capture_output=True)
    require(encoder.returncode==0 and b"libx264" in encoder.stdout,"Bundled FFmpeg lacks H.264 encoder")
    STAGE="install"
    adb("install","-r","android/app/build/outputs/apk/debug/app-debug.apk")
    adb("install","-r","android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk")
    STAGE="lan_avc_fixture"
    fixture=make_lan_avc_fixture(imageio_ffmpeg.get_ffmpeg_exe())
    adb("shell","-T","run-as com.p2p.remote sh -c 'mkdir -p files; cat > files/ci-lan-avc.bin'",data=fixture)
    alternate=make_lan_avc_fixture(imageio_ffmpeg.get_ffmpeg_exe(),640,360)
    adb("shell","-T","run-as com.p2p.remote sh -c 'cat > files/ci-lan-avc-alt.bin'",data=alternate)
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
            # Ignore a queued offer that no longer has enough validity for
            # instrumentation startup, HTTPS proof and explicit approval.
            if int(m.get("expires_at",0))-time.time()<65:
                continue
            pair=m
            break
    require(pair,"Host did not provide a current CI pairing window")
    REPORT["pairing_offer_remaining_seconds"]=int(pair["expires_at"]-time.time())
    readmission_root=secrets.token_bytes(32)
    config={"server":"wss://"+HOST,"token":TOKEN,"number":pair["number"],"code":pair["code"],
            "readmission_key":readmission_root.hex(),
            "expires_at":pair["expires_at"],"disposable_emulator_network_test":True}
    # Transfer over adb stdin into the disposable app's private directory.
    # Never pass credentials as CLI arguments, test properties or artifacts.
    adb("shell","-T","run-as com.p2p.remote sh -c 'mkdir -p files; cat > files/ci-private.json'",
        data=json.dumps(config).encode())
    STAGE="native_instrumentation"
    with (OUT/"instrumentation.txt").open("wb") as log:
        process=subprocess.Popen(["adb","shell","am","instrument","-w","-r","-e","class",
            "com.p2p.remote.PublicRelayE2eTest","com.p2p.remote.test/androidx.test.runner.AndroidJUnitRunner"],
            stdout=log,stderr=subprocess.STDOUT)
        # Include mandatory emulator radio outage and bounded recovery.
        end=time.monotonic()+660
        readmission_fixture=None
        aliases=[]
        aliases_ready=False
        aliases_cleaned=False
        try:
            while process.poll() is None and time.monotonic()<end:
                phase=adb("shell","run-as","com.p2p.remote","cat","files/e2e/phase.txt",check=False).stdout.strip()
                if phase==b"enable":
                    send({"type":"enable"})
                elif phase==b"multi_candidate_setup" and not aliases_ready:
                    interface=adb("shell","run-as","com.p2p.remote","cat",
                                  "files/e2e/candidate-interface.txt").stdout.decode().strip()
                    candidate_aliases(True,aliases,interface)
                    adb("shell","run-as","com.p2p.remote","touch","files/e2e/candidate-ready.txt")
                    aliases_ready=True
                elif phase==b"readmission_fixture_start" and readmission_fixture is None:
                    readmission_fixture=ReadmissionRelayFixture(readmission_root,fixture)
                    readmission_fixture.thread.start()
                    require(readmission_fixture.ready.wait(20) and readmission_fixture.error is None,
                            "Recovery public fixture not ready")
                    adb("shell","run-as","com.p2p.remote","touch","files/e2e/readmission-ready.txt")
                elif phase==b"multi_candidate_cleanup" and not aliases_cleaned:
                    if readmission_fixture:
                        readmission_fixture.close()
                    candidate_aliases(False,aliases)
                    adb("shell","run-as","com.p2p.remote","touch","files/e2e/candidate-cleaned.txt")
                    aliases_cleaned=True
                time.sleep(1)
            require(process.poll() is not None,"Android instrumentation timed out")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            try:
                try:
                    if readmission_fixture and not readmission_fixture.stop.is_set():
                        readmission_fixture.close()
                finally:
                    if aliases: candidate_aliases(False,aliases)
            finally:
                STAGE="evidence_collection"
                pull_evidence()
    STAGE="native_assertions"
    require((OUT/"report.json").is_file(),"Native report missing; inspect instrumentation.txt")
    native=json.loads((OUT/"report.json").read_text())
    REPORT["native_passed"]=native.get("passed") is True
    REPORT["metrics"]=native.get("metrics",{})
    if native.get("failed_stage"):
        REPORT["native_failed_stage"]=native["failed_stage"]
    instrumentation=(OUT/"instrumentation.txt").read_text(errors="replace")
    # Fixed stage identifiers only; do not emit exception contents, secrets,
    # raw networking details or whole reports into the CI log.
    if native.get("passed") is not True:
        failed=native.get("failed_stage","unknown")
        print("Native failed stage:",failed if isinstance(failed,str) and re.fullmatch(r"[a-zA-Z0-9_]{1,100}",failed) else "unknown",flush=True)
    require(process.returncode==0 and native.get("passed") is True
            and "OK (1 test)" in instrumentation and "INSTRUMENTATION_CODE: -1" in instrumentation,
            "Android native E2E assertions failed; inspect instrumentation.txt")
    outage=native.get("metrics",{}).get("system_network_outage",{})
    require(all(outage.get(key) is True for key in (
        "default_network_lost","native_connection_retired","default_network_restored",
        "new_authenticated_video","sustained_video")),
        "Mandatory Android system network outage evidence incomplete")
    candidate=native.get("metrics",{}).get("multi_candidate_lan",{})
    require(candidate.get("candidate_attempts")==2 and candidate.get("first_video_packets")==0 and
            candidate.get("relay_socket_opens_during_batch")==0 and
            all(candidate.get(key) is True for key in ("first_authenticated_hello",
                "second_authenticated_hello","stable_second_candidate_video",
                "verified_candidate_disconnect_recovers_public")),
            "Mandatory actual multi-candidate evidence incomplete")
    require(aliases_ready and aliases_cleaned and not aliases,"Candidate alias lifecycle incomplete")
    recovery=native.get("metrics",{}).get("lan_auto_readmission",{})
    require(recovery.get("automatic_socket_replacement") is True and
            recovery.get("v5_hellos")==1 and recovery.get("v5_commits")==1 and
            recovery.get("v5_pongs",0)>0 and recovery.get("v5_codec_requests",0)>0,
            "Mandatory automatic v5 readmission evidence incomplete")
    boundaries=recovery.get("boundaries",[])
    require([v.get("name") for v in boundaries]==["initial_lan","relay_video","restored_lan","stable_lan"],
            "Mandatory readmission boundaries missing")
    initial,relay,restored,stable=boundaries
    require([v.get("phase") for v in boundaries]==["lan_video","relay_video","lan_video","lan_video"] and
            all(v.get("codec")=="H264" and v.get("connected") is True and v.get("input_allowed") is False
                and v.get("surface_errors")==0 for v in boundaries) and
            initial["relay_opens"]==0 and relay["relay_opens"]==restored["relay_opens"]==stable["relay_opens"]==1 and
            stable["relay_bytes"]==restored["relay_bytes"] and stable["lan_bytes"]>restored["lan_bytes"] and
            stable["rendered"]>restored["rendered"] and stable["surface_frames"]>restored["surface_frames"] and
            stable["batches"]==1,"Automatic readmission transport/decoder evidence invalid")
    switches=native.get("metrics",{}).get("lan_avc_resolution_switches",[])
    require(len(switches)==2 and [(s.get("width"),s.get("height")) for s in switches]==[(640,360),(320,240)],
            "Mandatory AVC resolution switch evidence missing")
    for switch in switches:
        require(switch.get("predictive_only_packets",0)>=5 and switch.get("predictive_only_render_delta")==0
                and switch.get("predictive_only_connected") is False
                and switch.get("predictive_only_h264_active") is False
                and switch.get("predictive_only_codec")=="none"
                and switch.get("predictive_only_phase")=="lan_verified"
                and switch.get("predictive_only_received_avc_bytes",0)>0
                and switch.get("recovered_codec")=="H264" and switch.get("recovered_phase")=="lan_video"
                and switch.get("render_delta",0)>=3 and switch.get("surface_delta",0)>0
                and switch.get("recovery_ms",-1)>=0 and switch.get("same_socket") is True
                and switch.get("relay_socket_opens")==0 and switch.get("relay_video_bytes")==0
                and switch.get("surface_errors")==0,
                "Mandatory AVC resolution recovery evidence incomplete")
    REPORT["recording_enabled"]=False
    STAGE="host_confirmation"
    end=time.monotonic()+15
    last=0
    while time.monotonic()<end:
        # Drain an already received acknowledgement before resending. The host
        # may close immediately after its authenticated final acknowledgement.
        m=receive() if last else None
        if m and m.get("type")=="ack":
            REPORT["passed"]=True
            return
        if time.monotonic()-last>=2:
            send({"type":"done"})
            last=time.monotonic()
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
    COORD_STOP.set()
    if WS:
        # Give the sole receiver time to leave its short timeout before close.
        if COORD_READER:
            COORD_READER.join(timeout=1)
        try: WS.close()
        except Exception: pass
    if COORD_ERROR and not REPORT["passed"]:
        REPORT["coordinator_error"]=COORD_ERROR
    (OUT/"job-report.json").write_text(json.dumps(REPORT,indent=2),encoding="utf-8")
    print("PASS" if REPORT["passed"] else "FAILED",ROLE,flush=True)
sys.exit(0 if REPORT["passed"] else 1)
