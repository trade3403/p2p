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
import queue
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
COORD_INBOX=queue.Queue(maxsize=256)
COORD_STOP=threading.Event()
COORD_READER=None
COORD_ERROR=None
MARKER="P2P-V021-PUBLIC-UI-"+os.environ.get("GITHUB_RUN_ID","local")
PACKAGE="com.p2p.remote.p2p_remote_ui"
STABILITY_SECONDS=int(os.environ.get("P2P_STABILITY_SECONDS","300"))
if STABILITY_SECONDS not in (300,1800):
    raise ValueError("Stability duration must be 300 or 1800 seconds")
REPORT["requested_stability_seconds"]=STABILITY_SECONDS

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
    global KEY,WS,COORD_READER
    require(os.environ.get("GITHUB_ACTIONS")=="true","Dedicated CI runner only")
    require(32<=len(TOKEN)<=512,"Missing RELAY_ACCESS_TOKEN secret")
    with urllib.request.urlopen("https://"+HOST+"/healthz",timeout=15) as r:
        require(r.status==200 and r.read().strip()==b"ok","Public Relay healthz failed")
    ctx="|".join(["p2p-release-ui-v1",os.environ["GITHUB_REPOSITORY"],
                 os.environ["GITHUB_RUN_ID"],os.environ["GITHUB_RUN_ATTEMPT"]])
    KEY=hmac.digest(TOKEN.encode(),ctx.encode(),"sha256")
    room=hashlib.sha256(b"p2p-release-ui:"+KEY).hexdigest()
    WS=websocket.create_connection("wss://"+HOST+"/relay?room="+room+"&role="+role,
        header=["Authorization: Bearer "+TOKEN],timeout=15,enable_multithread=True)
    WS.settimeout(.3)
    # The Relay's read deadline is refreshed only by a PONG. websocket-client
    # processes server PING and sends PONG while reading, not while UI sampling.
    # One reader owns the socket for the entire test, including recordings and
    # weak-network sampling. The UI thread consumes only authenticated messages.
    COORD_READER=threading.Thread(target=coordinator_reader,name="coordinator-reader",daemon=True)
    COORD_READER.start()

def coordinator_reader():
    global COORD_ERROR
    try:
        while not COORD_STOP.is_set():
            try:
                packet=WS.recv()
            except websocket.WebSocketTimeoutException:
                continue
            if packet=="" or packet==b"":
                if not COORD_STOP.is_set():
                    COORD_ERROR="Coordinator closed"
                return
            if not isinstance(packet,bytes) or len(packet)<29:
                continue
            result=json.loads(AESGCM(KEY).decrypt(packet[:12],packet[12:],b"release-ui-v1"))
            COORD_INBOX.put_nowait(result)
    except Exception as exc:
        if not COORD_STOP.is_set():
            # Record only exception class; never log connection/credential details.
            COORD_ERROR=type(exc).__name__

def send(kind,**payload):
    nonce=secrets.token_bytes(12)
    WS.send_binary(nonce+AESGCM(KEY).encrypt(nonce,json.dumps({"type":kind,**payload}).encode(),b"release-ui-v1"))

def recv():
    try:
        result=COORD_INBOX.get(timeout=.3)
    except queue.Empty:
        if COORD_ERROR:
            raise AssertionError("Test coordinator reader failed: "+COORD_ERROR)
        return None
    if result.get("type")=="failed":
        raise AssertionError("Peer UI test failed: "+str(result.get("stage","unknown")))
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
    raise AssertionError("Timed out waiting for "+kind)

# Low-resolution fingerprints of actual OS screenshots. These are only used
# in the disposable test desktop, and are never treated as a video frame.
SIGNATURE_SIZE=(40,24)

def visual_signature(image,bounds=None):
    if bounds is not None:
        x1,y1,x2,y2=bounds
        if not (0<=x1<x2<=image.width and 0<=y1<y2<=image.height):
            return None
        image=image.crop(bounds)
    return list(image.convert("L").resize(SIGNATURE_SIZE,Image.Resampling.BOX).getdata())

def mean_distance(a,b,indices):
    return sum(abs(a[i]-b[i]) for i in indices)/len(indices)

def changed_cells(before,after):
    n=SIGNATURE_SIZE[0]*SIGNATURE_SIZE[1]
    require(len(before)==len(after)==n,"Invalid visual reference dimensions")
    cells=[i for i in range(n) if abs(before[i]-after[i])>=40]
    require(len(cells)>=12,"Actual Windows state change lacks visual contrast")
    return cells

class Recording:
    """Actual OS screen captures at a fixed sampling rate; no remote-frame fabrication."""
    def __init__(self,capture,name):
        self.capture=capture
        self.name=name
        self.stop=threading.Event()
        self.error=None
        self.frames=0
        self.worker=None
        self.observation_lock=threading.Lock()
        self.observation=None
    def observe(self,bounds):
        with self.observation_lock:
            self.observation=(tuple(bounds),[])
    def samples(self):
        with self.observation_lock:
            return list(self.observation[1]) if self.observation else []
    def stop_observing(self):
        with self.observation_lock:
            self.observation=None
    def start(self):
        self.worker=threading.Thread(target=self.run,daemon=True)
        self.worker.start()
    def run(self):
        writer=None
        try:
            img=self.capture().convert("RGB")
            # Landscape canvas gives full-screen remote operation a readable recording.
            # Portrait pairing remains visible, centered without cropping or stretching.
            w,h=(1920,1080) if ROLE=="android" else (img.width//2*2,img.height//2*2)
            writer=imageio_ffmpeg.write_frames(str(OUT/self.name),(w,h),fps=4,
                codec="libx264",pix_fmt_in="rgb24",pix_fmt_out="yuv420p",
                output_params=["-movflags","+faststart","-crf","19"])
            writer.send(None)
            while not self.stop.is_set():
                start=time.monotonic()
                img=self.capture().convert("RGB")
                captured=time.monotonic()
                # Reuse the real recording capture rather than running another
                # screenshot loop concurrently with the remote interaction.
                with self.observation_lock:
                    if self.observation is not None:
                        bounds,samples=self.observation
                        if len(samples)<120:
                            sig=visual_signature(img,bounds)
                            if sig is not None:
                                samples.append((start,captured,sig))
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
        candidates=[]
        nodes=android_tree()
        # A failed UI dump is not evidence that the control is off-screen.
        if not nodes:
            return None
        attempts+=1
        for node in nodes:
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
            # Cycle back toward the top instead of getting stuck at the bottom
            # while a dynamic control becomes enabled or a UI dump misses it.
            y1,y2=(.34,.79) if (attempts-1)%6 in (0,1) else (.77,.46)
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

def visual_click_trials(rec):
    results=[]
    for trial in ("maximize","restore"):
        target=await_message("visual_target",seconds=35)
        require(target["trial"]==trial,"Unexpected visual trial")
        b=find_android("p2p_remote_video",enabled=False)
        before=target["before"]
        def baseline():
            img=android_capture()
            sig=visual_signature(img,b)
            if sig is not None and len(sig)==len(before) and mean_distance(sig,before,range(len(before)))<=24:
                return sig
            return None
        visible_before=wait("Actual before-state visible on Android: "+trial,baseline,15)
        rec.observe(b)
        try:
            t0=time.monotonic()
            adb("shell","input","tap",*remote_point(b,target["click"]))
            returned=time.monotonic()
            send("visual_tap_sent",trial=trial)
            verified=await_message("visual_verified",seconds=30)
            require(verified["trial"]==trial,"Unexpected host verification")
            after=verified["after"]
            cells=changed_cells(before,after)
            contrast=mean_distance(before,after,cells)
            require(mean_distance(visible_before,before,cells)<=max(28,contrast*.45)
                    and mean_distance(visible_before,after,cells)>=contrast*.55,
                    "Android baseline did not match actual pre-click window")
            def first_visible():
                samples=rec.samples()
                for i,(began,ended,sig) in enumerate(samples):
                    if ended<t0: continue
                    old=mean_distance(sig,before,cells)
                    new=mean_distance(sig,after,cells)
                    if new<=max(25,contrast*.42) and old>=contrast*.58 and old-new>=contrast*.25:
                        previous=samples[i-1][1] if i>0 else None
                        return {"trial":trial,
                            "tap_to_verified_sample_end_ms":round((ended-t0)*1000,1),
                            "adb_command_ms":round((returned-t0)*1000,1),
                            "sample_capture_ms":round((ended-began)*1000,1),
                            "sample_gap_ms":round((ended-previous)*1000,1) if previous is not None else None,
                            "changed_cells":len(cells),
                            "distance_to_before":round(old,2),
                            "distance_to_after":round(new,2),
                            "verified_host_rect":verified["rect"]}
                return None
            result=wait("Actual new window state in Android recording: "+trial,first_visible,15)
            results.append(result)
            REPORT["visual_feedback_trials"]=results
            snapshot(android_capture,"04b-visual-"+trial)
            send("visual_observed",trial=trial)
        finally:
            rec.stop_observing()
    REPORT["visual_feedback_scope"]="Android monotonic tap dispatch to completion of first matched real OS screenshot; includes ADB and sampling overhead; not precise glass-to-glass or input-to-photon"
    passed("Real Notepad maximize and restore verified in Android screenshot samples")

def read_avc_metrics(strict=True):
    android_tap("视频性能")
    def extract():
        fields=[]
        for node in android_tree():
            fields.extend([node.get("text",""),node.get("content-desc","")])
        for field in fields:
            if "codec=" in field and "decoder_mode=" in field:
                # Disconnected native sessions legitimately expose an empty decoder.
                # Keep the field present; strict live AVC validation below
                # still rejects blank/unknown encoder and decoder names.
                items=dict(re.findall(r"(?m)^([a-z_]+)=([^\r\n]*)",field))
                if all(k in items for k in ("codec","frames","fps","bps","rtt","relay_socket_opens","relay_retry_events","target_bps","target_fps","drops","encoder","decoder","decoder_mode","decoder_recovering",
                        "capture_backend","capture_avg_us","encode_avg_us","send_avg_us","fresh_frames","reused_frames","resize_cache_hits","yuv_cache_hits","host_encoded_frames",
                        "transport","viewer_lan_video_bytes","viewer_relay_video_bytes","host_lan_video_bytes","host_relay_video_bytes","host_relay_suppressed_frames",
                        "transport_phase","host_metadata_current","lan_attempt_ms","lan_fallback_reason","lan_fallback_started",
                        "lan_fallback_relay_open_ms","lan_fallback_first_video_ms","host_lan_video_frames",
                        "host_relay_video_frames","host_relay_socket_open","host_relay_socket_opens")):
                    return items
        return None
    try:
        data=wait("Actual release video diagnostic data",extract,25)
    except AssertionError:
        # Save actual diagnostic dialog only (contains no connection secret).
        snapshot(android_capture,"diagnostic-read-failed-"+str(time.monotonic_ns()))
        raise
    finally:
        android_tap("关闭性能数据")
    for key in ("frames","bps","rtt","relay_socket_opens","relay_retry_events","target_bps","target_fps","drops","capture_avg_us","encode_avg_us","send_avg_us","fresh_frames","reused_frames","resize_cache_hits","yuv_cache_hits","host_encoded_frames",
                "viewer_lan_video_bytes","viewer_relay_video_bytes","host_lan_video_bytes","host_relay_video_bytes","host_relay_suppressed_frames",
                "lan_attempt_ms","lan_fallback_relay_open_ms","lan_fallback_first_video_ms","host_lan_video_frames","host_relay_video_frames","host_relay_socket_opens"):
        data[key]=int(data[key])
    data["fps"]=float(data["fps"])
    # Persist only these fixed diagnostic fields before assertions, including
    # a failing sample. No credential, clipboard or arbitrary UI text.
    REPORT["last_video_diagnostic"]={key:data[key] for key in (
        "codec","frames","fps","bps","rtt","drops","encoder","decoder","decoder_mode",
        "decoder_recovering","capture_backend","host_metadata_current","transport_phase",
        "relay_socket_opens","relay_retry_events","host_encoded_frames")}
    if not strict:
        return data
    require(data["codec"]=="H264" and data["frames"]>0,"Actual MediaCodec AVC playback not verified")
    require(data["transport_phase"]=="relay_video" and data["host_metadata_current"]=="true" and data["host_relay_socket_open"]=="true"
            and data["host_relay_socket_opens"]>=1 and data["host_relay_video_frames"]>=0,
            "Actual release WSS transport diagnostics not verified")
    if data["lan_fallback_first_video_ms"]>=0:
        require(data["lan_fallback_started"]=="true" and data["lan_fallback_relay_open_ms"]>=0
                and data["lan_fallback_first_video_ms"]>=data["lan_fallback_relay_open_ms"],
                "Invalid measured LAN recovery ordering")
    require(data["encoder"] in ("OpenH264 software","Media Foundation hardware","Media Foundation D3D11 texture") and data["decoder"] not in ("","unknown"),"Actual encoder and decoder not reported")
    require(data["decoder_mode"] in ("hardware","software","unknown"),"Invalid decoder classification")
    require(data["decoder_recovering"] in ("true","false"),"Missing AVC decoder recovery state")
    require(data["capture_backend"] in ("DXGI","screenshots compatibility"),"Actual host capture backend not reported")
    require(data["capture_avg_us"]>0 and data["encode_avg_us"]>0 and data["send_avg_us"]>0
            and data["fresh_frames"]>0 and data["host_encoded_frames"]>0,"No real host stage measurements")
    require(data["decoder_mode"]!="hardware","CI emulator must not claim physical hardware decoding")
    if any(name in data["decoder"].lower() for name in ("goldfish","ranchu")):
        require(data["decoder_mode"]=="software","Virtual emulator AVC decoder misclassified")
    return data

def weak_network_step(baseline):
    """Impair real public-session packets on the Linux emulator host."""
    from network_shape import shape_relay_egress
    evidence={"baseline":baseline,
              "scope":"Actual host egress shaping of Relay TCP traffic; not physical network, downstream bitrate cap, or input-to-photon"}
    REPORT["weak_network_step"]=evidence
    def samples():
        result=[]
        for _ in range(2):
            time.sleep(8)
            result.append(read_avc_metrics())
            # Maintain the independent test coordinator while the app runs.
            send("weak_progress",frames=result[-1]["frames"])
        return result
    # Capture the actual coordinator source port to exclude it from impairment.
    coordinator_port=WS.sock.getsockname()[1]
    with shape_relay_egress(HOST,coordinator_port,evidence):
        evidence["shaped"]=samples()
    require(sum(x.get("netem_packets",0) for x in evidence["interfaces"])>0,
            "No actual Relay packets matched kernel shaping")
    evidence["restored"]=samples()
    shaped=evidence["shaped"]
    restored=evidence["restored"]
    require(shaped[-1]["frames"]>shaped[0]["frames"]>baseline["frames"],
            "No actual AVC render progress during shaped network")
    require(restored[-1]["frames"]>restored[0]["frames"]>shaped[-1]["frames"],
            "No actual AVC render progress after restoring network")
    require(all(m["fps"]>0 and m["bps"]>0 for m in shaped+restored),
            "No measured live video throughput in network step")
    require(min(m["rtt"] for m in shaped)>baseline["rtt"]+100,
            "Shaped network did not measurably increase authenticated RTT")
    require(max(m["rtt"] for m in restored)<min(m["rtt"] for m in shaped),
            "Authenticated RTT did not recover after network restoration")
    require(shaped[-1]["target_bps"]<=baseline["target_bps"] and
            shaped[-1]["target_fps"]<=baseline["target_fps"],
            "Congestion increased video target")
    passed("Actual Relay egress impairment and restored AVC rendering")
    return restored[-1]

def recovery_step(baseline):
    """Real egress blackout; observe native retry, new WSS, and new AVC output."""
    from network_shape import shape_relay_egress
    evidence={"baseline":baseline,"scope":"Actual Relay egress blackout and automatic reconnect in installed release APK; observation time includes UI automation"}
    REPORT["avc_recovery"]=evidence
    require(baseline["relay_socket_opens"]>=1,"Actual initial WSS open not reported")
    coordinator_port=WS.sock.getsockname()[1]
    with shape_relay_egress(HOST,coordinator_port,evidence,outage=True):
        outage_started=time.monotonic()
        deadline=outage_started+80
        evidence["outage_samples"]=[]
        while time.monotonic()<deadline:
            time.sleep(5)
            sample=read_avc_metrics(strict=False)
            evidence["outage_samples"].append(sample)
            if sample["relay_retry_events"]>baseline["relay_retry_events"] and sample["codec"]=="none" and sample["host_metadata_current"]=="false":
                require(sample["fps"]==0 and sample["bps"]==0 and sample["encoder"]==""
                        and sample["host_relay_socket_open"]=="unknown" and sample["host_relay_video_bytes"]==-1,
                        "Stale current-socket diagnostics during actual blackout")
                evidence["interrupted"]=sample
                snapshot(android_capture,"05a-network-interrupted")
                break
        else:
            raise AssertionError("No actual native reconnect event observed during blackout")
        evidence["blackout_observed_ms"]=round((time.monotonic()-outage_started)*1000)
    restored_at=time.monotonic()
    require(sum(x.get("netem_drops",0) for x in evidence["interfaces"])>0,
            "No actual Relay packets dropped in blackout")
    require(all(x.get("restored") for x in evidence["interfaces"]),"Blackout qdisc not cleaned up")
    deadline=restored_at+120
    samples=[]
    while time.monotonic()<deadline:
        time.sleep(3)
        sample=read_avc_metrics(strict=False)
        samples.append(sample)
        evidence["recovery_samples"]=samples
        if (sample["relay_socket_opens"]>baseline["relay_socket_opens"] and
            sample["relay_retry_events"]>baseline["relay_retry_events"] and
            sample["codec"]=="H264" and sample["frames"]>evidence["interrupted"]["frames"]
            and sample["fps"]>0 and sample["bps"]>0 and sample["rtt"]>=0):
            evidence["restore_to_observed_avc_ms"]=round((time.monotonic()-restored_at)*1000)
            break
    else:
        raise AssertionError("No authenticated live AVC on a newly opened WSS after blackout")
    time.sleep(6)
    verified=read_avc_metrics()
    evidence["verified"]=verified
    require(verified["frames"]>sample["frames"] and verified["fps"]>0 and verified["bps"]>0,
            "Recovered AVC did not continue rendering")
    snapshot(android_capture,"05b-network-recovered")
    passed("Actual Relay blackout, automatic WSS reconnect and sustained AVC rendering")
    send("recovery_ready")
    target=await_message("recovery_target",seconds=45)
    b=find_android("p2p_remote_video",enabled=False)
    adb("shell","input","tap",*remote_point(b,target["edit"]))
    android_tap("远程输入文字")
    android_field("远程文字内容","-RECOVERED",identifier="p2p_remote_text")
    android_tap("发送远程文字")
    send("recovery_text_sent")
    await_message("recovery_text_ok",seconds=45)
    snapshot(android_capture,"05c-recovered-remote-text")
    evidence["remote_text_verified"]=True
    passed("Exact post-reconnect remote text independently verified by Windows")
    return verified

def stability_step():
    """Continuous real AVC observation after reconnect; default 5 min, extended 30 min."""
    baseline=read_avc_metrics()
    intervals=STABILITY_SECONDS//30
    evidence={"scope":"Continuous post-recovery release UI observation; sampled diagnostics, not continuous photon measurements or 24h proof",
              "minimum_elapsed_seconds":STABILITY_SECONDS,"expected_samples":intervals,"baseline":baseline,"samples":[]}
    REPORT["avc_stability"]=evidence
    started=time.monotonic()
    previous=baseline
    # The same connection and native counters must survive the entire interval.
    # Poll at 30s cadence using monotonic deadlines, not frame count or MP4 duration.
    for index in range(1,intervals+1):
        scheduled=started+30*index
        time.sleep(max(0,scheduled-time.monotonic()))
        sample=read_avc_metrics()
        elapsed=time.monotonic()-started
        evidence["samples"].append({"elapsed_seconds":round(elapsed,3),**sample})
        print("STABILITY",index,"/",intervals,"elapsed_s",round(elapsed,1),"frames",sample["frames"],"fps",sample["fps"],"rtt_ms",sample["rtt"],flush=True)
        require(sample["relay_socket_opens"]==baseline["relay_socket_opens"] and
                sample["relay_retry_events"]==baseline["relay_retry_events"],
                "Unexpected reconnect during continuous AVC stability interval")
        require(sample["frames"]>previous["frames"] and sample["host_encoded_frames"]>previous["host_encoded_frames"],
                "No real AVC frame progress in stability interval")
        require(sample["fps"]>0 and sample["bps"]>0 and sample["rtt"]>=0,
                "No valid live throughput/RTT in stability interval")
        require(sample["drops"]>=previous["drops"],"Decoder drop counter unexpectedly reset")
        previous=sample
        send("stability_progress",index=index,frames=sample["frames"])
    elapsed=time.monotonic()-started
    require(elapsed>=STABILITY_SECONDS and len(evidence["samples"])==intervals,"Stability interval too short")
    evidence["elapsed_seconds"]=round(elapsed,3)
    evidence["rendered_frame_delta"]=previous["frames"]-baseline["frames"]
    evidence["observed_average_render_fps"]=evidence["rendered_frame_delta"]/elapsed
    evidence["decoder_drop_delta"]=previous["drops"]-baseline["drops"]
    evidence["rtt_min_ms"]=min(m["rtt"] for m in evidence["samples"])
    evidence["rtt_max_ms"]=max(m["rtt"] for m in evidence["samples"])
    snapshot(android_capture,"05d-stability-complete")
    passed(f"{STABILITY_SECONDS//60}-minute continuous AVC observation without additional WSS retry")
    send("stability_ready",elapsed_seconds=evidence["elapsed_seconds"],sample_count=len(evidence["samples"]),requested_seconds=STABILITY_SECONDS)
    target=await_message("stability_target",seconds=45)
    b=find_android("p2p_remote_video",enabled=False)
    adb("shell","input","tap",*remote_point(b,target["edit"]))
    android_tap("远程输入文字")
    android_field("远程文字内容","-STABLE",identifier="p2p_remote_text")
    android_tap("发送远程文字")
    send("stability_text_sent")
    await_message("stability_text_ok",seconds=45)
    snapshot(android_capture,"05e-stability-remote-text")
    evidence["remote_text_verified"]=True
    passed("Exact remote text after continuous AVC observation verified by Windows")

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
        try:
            android_tap("全屏远控",seconds=65,scroll=True,identifier="p2p_remote_fullscreen")
        except AssertionError:
            snapshot(android_capture,"fullscreen-entry-failed")
            raise
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
        snapshot(android_capture,"03a-landscape-text-form")
        android_field("远程文字内容",MARKER,identifier="p2p_remote_text")
        snapshot(android_capture,"03b-landscape-text-entered")
        text_started=time.monotonic()
        android_tap("发送远程文字")
        send("text_sent")
        await_message("text_ok")
        REPORT["text_ui_to_host_confirmation_ms"]=round((time.monotonic()-text_started)*1000)
        # Includes automation and rendezvous acknowledgment; not glass-to-glass latency.
        time.sleep(2)
        snapshot(android_capture,"04-notepad-remote-text")
        passed("Remote text verified by Windows independently")
        STAGE="visual_feedback"
        visual_click_trials(rec)
        STAGE="actual_avc_performance"
        # Read only user-visible diagnostics backed by actual decoder callbacks.
        # Existing functional assertions and videos remain mandatory.
        first=read_avc_metrics()
        time.sleep(10)
        second=read_avc_metrics()
        require(second["frames"]>first["frames"] and second["fps"]>0 and second["bps"]>0
                and second["rtt"]>=0,"No measured live AVC progress")
        require(second["host_encoded_frames"]>first["host_encoded_frames"],"Host encoded frame count did not progress")
        require(second["transport"]=="VPS WSS 中转" and first["transport"]=="VPS WSS 中转",
                "Public release test must use actual WSS transport")
        require(second["viewer_relay_video_bytes"]>first["viewer_relay_video_bytes"]>=0
                and second["host_relay_video_bytes"]>first["host_relay_video_bytes"]>=0,
                "Actual release AVC relay video bytes did not progress on both ends")
        require(first["viewer_lan_video_bytes"]==second["viewer_lan_video_bytes"]==0
                and first["host_lan_video_bytes"]==second["host_lan_video_bytes"]==0,
                "Unexpected LAN video bytes in public release test")
        if second["capture_backend"]=="DXGI" and second["encoder"]!="Media Foundation D3D11 texture" and second["reused_frames"]>0:
            require(second["resize_cache_hits"]>0,"Idle DXGI frames did not use scaled pixel cache")
        if second["encoder"]=="OpenH264 software" and second["resize_cache_hits"]>0:
            require(second["yuv_cache_hits"]>0,"Software encoder did not reuse unchanged YUV frames")
        REPORT["actual_avc_metrics"]={"first":first,"second":second,
            "scope":"MediaCodec rendered output, received AVC payload bitrate, authenticated RTT; no synthetic data"}
        passed("Live AVC rendering, actual throughput and RTT measured in landscape release APK")
        STAGE="weak_network_step"
        restored=weak_network_step(second)
        STAGE="avc_recovery"
        recovery_step(restored)
        STAGE="avc_stability"
        stability_step()
        STAGE="direct_keyboard"
        android_tap("键盘")
        find_android("收起键盘",seconds=15)
        # Use actual Android IME/editor input, without the legacy send form.
        wait("Android system IME visible",lambda: b"mInputShown=true" in
             adb("shell","dumpsys","input_method").stdout,15)
        find_android("p2p_remote_video",enabled=False,seconds=15)
        snapshot(android_capture,"05f-direct-keyboard-visible")
        android_type("-LIVE9")
        send("direct_text_typed")
        await_message("direct_text_ok",seconds=45)
        for label,kind,value in [
            ("backspace","key","退格"),("space","key","空格"),
            ("word","text","OK"),("enter","key","回车"),("next","text","NEXT"),
        ]:
            if kind=="key":
                android_tap(value)
            else:
                android_type(value)
            send("direct_edit_typed",step=label)
            await_message("direct_edit_"+label+"_ok",seconds=45)
        snapshot(android_capture,"05g-direct-keyboard-typed")
        android_tap("收起键盘")
        find_android("键盘",seconds=15)
        REPORT["direct_keyboard_verified"]=True
        passed("Direct Android keyboard text and editing without send form verified by Windows")
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
        await_message("ack",seconds=300)
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
        STAGE="visual_feedback"
        def caption_button():
            # Read the actual accessibility bounds, not a guessed x-coordinate.
            try:
                buttons=desktop.window(handle=note._hWnd).descendants(control_type="Button")
                names=("maximize","restore","restore down")
                for button in buttons:
                    label=(button.element_info.name or "").strip().lower()
                    r=button.rectangle()
                    if label in names and button.is_visible() and button.is_enabled() and r.width()>4 and r.height()>4:
                        x,y=(r.left+r.right)//2,(r.top+r.bottom)//2
                        if 0<=x<w and 0<=y<h and note.top<=y<note.top+100:
                            return normal(x,y)
            except Exception:
                pass
            # Native title-bar accessibility geometry supports classic Notepad.
            # WM_GETTITLEBARINFOEX is read-only; it never activates a button.
            class TitleBarInfoEx(ctypes.Structure):
                _fields_=[("cbSize",wintypes.DWORD),("rcTitleBar",wintypes.RECT),
                          ("rgstate",wintypes.DWORD*6),("rgrect",wintypes.RECT*6)]
            info=TitleBarInfoEx()
            info.cbSize=ctypes.sizeof(info)
            query=ctypes.windll.user32.SendMessageTimeoutW
            query.argtypes=[wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM,
                            wintypes.UINT,wintypes.UINT,ctypes.POINTER(ctypes.c_size_t)]
            query.restype=wintypes.LPARAM
            response=ctypes.c_size_t()
            ok=query(note._hWnd,0x033f,0,ctypes.addressof(info),2,1000,ctypes.byref(response))
            r=info.rgrect[3]
            require(ok and not (info.rgstate[3]&0x18001) and r.right>r.left and r.bottom>r.top,
                    "Actual Notepad maximize/restore button not available")
            return normal((r.left+r.right)//2,(r.top+r.bottom)//2)
        REPORT["visual_clicks"]=[]
        for trial,expected in (("maximize",True),("restore",False)):
            require(bool(ctypes.windll.user32.IsZoomed(note._hWnd))!=expected,
                    "Unexpected initial Notepad window state")
            before_rect=rect(note)
            target=caption_button()
            before=visual_signature(capture())
            send("visual_target",trial=trial,before=before,click=target)
            tapped=await_message("visual_tap_sent",seconds=45)
            require(tapped["trial"]==trial,"Unexpected Android visual click")
            wait("Actual Notepad "+trial,lambda: bool(ctypes.windll.user32.IsZoomed(note._hWnd))==expected,15)
            time.sleep(.5)
            after_rect=rect(note)
            require(before_rect!=after_rect,"Native window geometry did not change")
            after=visual_signature(snapshot(capture,"08-visual-"+trial))
            changed_cells(before,after)
            send("visual_verified",trial=trial,after=after,rect=after_rect)
            observed=await_message("visual_observed",seconds=45)
            require(observed["trial"]==trial,"Android did not verify actual window state")
            REPORT["visual_clicks"].append({"trial":trial,"before":before_rect,"after":after_rect})
        passed("Remote maximize and restore independently verified by native window state")
        STAGE="avc_recovery"
        await_message("recovery_ready",seconds=420)
        require(ctypes.windll.user32.GetForegroundWindow()==note._hWnd,"Notepad no longer foreground after network recovery")
        # Click inside the actual editor, after the existing text. The local
        # Host never injects the recovery suffix; only the Android UI sends it.
        current=rect(note)
        send("recovery_target",edit=normal(current[0]+int(current[2]*.75),current[1]+int(current[3]*.65)))
        await_message("recovery_text_sent",seconds=60)
        def recovery_readback():
            if ctypes.windll.user32.GetForegroundWindow()!=note._hWnd: return False
            pyperclip.copy("")
            gui.hotkey("ctrl","a")
            gui.hotkey("ctrl","c")
            time.sleep(.25)
            value=pyperclip.paste()
            pyperclip.copy("")
            return value.strip()==MARKER+"-RECOVERED"
        wait("Exact post-recovery remote text in actual Notepad",recovery_readback,20)
        gui.press("end")
        snapshot(capture,"09-recovered-notepad-text")
        REPORT["avc_recovery_text_verified"]=True
        send("recovery_text_ok")
        passed("Exact post-reconnect remote text independently read from Notepad")
        STAGE="avc_stability"
        stable=await_message("stability_ready",seconds=STABILITY_SECONDS+300)
        require(stable["requested_seconds"]==STABILITY_SECONDS and stable["elapsed_seconds"]>=STABILITY_SECONDS
                and stable["sample_count"]==STABILITY_SECONDS//30,"Peer stability interval or sample count incorrect")
        require(ctypes.windll.user32.GetForegroundWindow()==note._hWnd,"Notepad lost foreground during stability interval")
        current=rect(note)
        send("stability_target",edit=normal(current[0]+int(current[2]*.75),current[1]+int(current[3]*.65)))
        await_message("stability_text_sent",seconds=60)
        def stable_readback():
            if ctypes.windll.user32.GetForegroundWindow()!=note._hWnd: return False
            pyperclip.copy("")
            gui.hotkey("ctrl","a")
            gui.hotkey("ctrl","c")
            time.sleep(.25)
            value=pyperclip.paste()
            pyperclip.copy("")
            return value.strip()==MARKER+"-RECOVERED-STABLE"
        wait("Exact post-stability remote text in Notepad",stable_readback,20)
        gui.press("end")
        snapshot(capture,"10-stability-notepad-text")
        REPORT["avc_stability_text_verified"]=True
        REPORT["peer_stability_elapsed_seconds"]=stable["elapsed_seconds"]
        REPORT["peer_stability_sample_count"]=stable["sample_count"]
        send("stability_text_ok")
        passed("Exact post-stability remote text independently read from Notepad")
        STAGE="direct_keyboard"
        # Read real Notepad content without selecting, copying, moving the
        # caret or sending local keyboard input while remote input is in flight.
        # The previous recording showed -LIVE9, -LIVE, and -LIVE OK, then just
        # T after the final poll. Ctrl+A/C polling could race remote typing.
        def direct_readback(expected):
            if ctypes.windll.user32.GetForegroundWindow()!=note._hWnd:
                return False
            try:
                candidates=desktop.window(handle=note._hWnd).descendants(control_type="Edit")
                for editor in candidates:
                    if not editor.is_visible():
                        continue
                    for read in (lambda: editor.iface_text.DocumentRange.GetText(-1),
                                 lambda: editor.iface_value.CurrentValue):
                        try:
                            value=read()
                            if isinstance(value,str):
                                value=value.replace("\r\n","\n")
                                # UIA TextPattern may expose an implicit final
                                # paragraph delimiter not present in the file.
                                if value==expected or value==expected+"\n":
                                    return True
                        except Exception:
                            continue
            except Exception:
                pass
            return False
        REPORT["direct_keyboard_readback"]="read-only Windows UIA Text/Value patterns"
        REPORT["direct_keyboard_steps"]=[]
        await_message("direct_text_typed",seconds=60)
        prefix=MARKER+"-RECOVERED-STABLE"
        wait("Direct typing arrived without send",lambda: direct_readback(prefix+"-LIVE9"),25)
        REPORT["direct_keyboard_steps"].append("initial_text")
        send("direct_text_ok")
        for step,expected in [
            ("backspace",prefix+"-LIVE"),
            ("space",prefix+"-LIVE "),
            ("word",prefix+"-LIVE OK"),
            ("enter",prefix+"-LIVE OK\n"),
            ("next",prefix+"-LIVE OK\nNEXT"),
        ]:
            message=await_message("direct_edit_typed",seconds=60)
            require(message.get("step")==step,"Unexpected direct keyboard step")
            # Newline needs special care: TextPattern can add its own trailing
            # delimiter. Independently verify actual native caret line below.
            if step=="enter":
                def entered_new_line():
                    if not direct_readback(expected):
                        return False
                    try:
                        labels=desktop.window(handle=note._hWnd).descendants()
                        return any(re.search(r"\bLn\s*2\s*,\s*Col\s*1\b",x.element_info.name or "",re.I) for x in labels)
                    except Exception:
                        return False
                wait("Direct keyboard Enter moved actual caret to line 2",entered_new_line,25)
            else:
                wait("Direct keyboard "+step,lambda: direct_readback(expected),25)
            REPORT["direct_keyboard_steps"].append(step)
            snapshot(capture,"11-direct-keyboard-"+step)
            send("direct_edit_"+step+"_ok")
        REPORT["direct_keyboard_verified"]=True
        passed("Exact live typing and editing independently read from Notepad")
        STAGE="host_confirmation"
        await_message("done",seconds=300)
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
    COORD_STOP.set()
    if WS:
        try: WS.close()
        except Exception: pass
    if COORD_READER:
        COORD_READER.join(timeout=3)
    if COORD_ERROR:
        REPORT["coordinator_reader_error"]=COORD_ERROR
    (OUT/"report.json").write_text(json.dumps(REPORT,indent=2,ensure_ascii=False),encoding="utf-8")
    print("PASS" if REPORT["passed"] else "FAILED",ROLE,flush=True)
sys.exit(0 if REPORT["passed"] else 1)
