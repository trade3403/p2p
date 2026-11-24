"""Read-only Android foreground validation and bounded initial startup.

No dismissal of application ANRs; no direct calls to the application engine.
Use the full window dump: the windows subcommand can omit mCurrentFocus.
"""
import subprocess
import time

def query(*args):
    result=subprocess.run(["adb","shell",*args],capture_output=True,text=True,errors="replace",timeout=25)
    return result.stdout if result.returncode==0 else ""

def foreground_state(package):
    activities=query("dumpsys","activity","activities")
    windows=query("dumpsys","window")
    resumed=[line.strip() for line in activities.splitlines()
             if "ResumedActivity" in line or "topResumedActivity" in line]
    focused=[line.strip() for line in windows.splitlines() if "mCurrentFocus=" in line]
    # A focused ANR or other system dialog must never count as an app window.
    ok=any(package+"/" in line for line in resumed) and any(
        package+"/" in line and "Application Not Responding" not in line
        and "Application Error" not in line for line in focused)
    return ok,{"activity_resumed":any(package+"/" in line for line in resumed),
               "app_window_focused":any(package+"/" in line for line in focused),
               "focus_line_count":len(focused),
               "system_error_window":any("Application Not Responding" in line or "Application Error" in line for line in focused)}

def start_release_app(package,report):
    subprocess.run(["adb","shell","input","keyevent","KEYCODE_WAKEUP"],check=True,timeout=15)
    subprocess.run(["adb","shell","wm","dismiss-keyguard"],check=True,timeout=15)
    # The AOSP image has no Google Pixel Launcher. Wait for actual boot completion
    # instead of repeatedly force-stopping the launcher while it is initializing.
    deadline=time.monotonic()+90
    while time.monotonic()<deadline:
        if query("getprop","sys.boot_completed").strip()=="1": break
        time.sleep(2)
    else: raise AssertionError("Emulator did not finish booting")
    subprocess.run(["adb","shell","am","start","-W","-n",package+"/.MainActivity"],check=True,timeout=60)
    deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        ok,state=foreground_state(package)
        report["startup_focus"]=state
        if ok:
            time.sleep(3)
            ok,state=foreground_state(package)
            report["startup_focus"]=state
            if ok: return
        time.sleep(2)
    raise AssertionError("Release APK did not obtain stable foreground input focus")
