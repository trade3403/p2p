"""Targeted Linux egress impairment on the disposable GitHub Android UI runner.

The emulator's console accepted commands but did not affect the authenticated
session in run 45485046902. Match actual Relay destination addresses and port
443 on the host route, and verify kernel packet counters. No server changes.
"""
import json
import os
import socket
import subprocess
from contextlib import contextmanager

def command(*args,check=True):
    result=subprocess.run(list(args),capture_output=True,text=True,timeout=20)
    if check and result.returncode:
        raise AssertionError("Network shaping command failed: "+" ".join(args)+"; "+result.stderr.strip()[:350])
    return result

def tc(*args,check=True):
    return command("sudo","-n","tc",*args,check=check)

def routes(host):
    ips=sorted({x[4][0] for x in socket.getaddrinfo(host,443,type=socket.SOCK_STREAM)})
    found={}
    for ip in ips:
        family="-6" if ":" in ip else "-4"
        route=command("ip",family,"-j","route","get",ip,check=False)
        if route.returncode:
            continue
        data=json.loads(route.stdout)
        if not data or not data[0].get("dev") or data[0]["dev"]=="lo":
            continue
        found.setdefault(data[0]["dev"],[]).append(ip)
    if not found:
        raise AssertionError("No routable public Relay addresses for network test")
    return found

@contextmanager
def shape_relay_egress(host,coordinator_port,evidence,*,outage=False):
    if os.environ.get("GITHUB_ACTIONS")!="true" or os.environ.get("RUNNER_ENVIRONMENT")!="github-hosted" or os.environ.get("RUNNER_OS")!="Linux":
        raise AssertionError("Traffic shaping restricted to disposable GitHub-hosted Linux runners")
    if not 1<=coordinator_port<=65535:
        raise AssertionError("Unable to exclude test coordinator connection")
    targets=routes(host)
    evidence["mechanism"]="Linux host tc prio + netem; Relay TCP/443 egress only"
    evidence["profile"]={"egress_loss_percent":100} if outage else {"egress_delay_ms":350,"egress_rate_kbps":1000}
    evidence["interfaces"]=[]
    installed=[]
    try:
        for dev,ips in targets.items():
            before=json.loads(tc("-j","qdisc","show","dev",dev).stdout)
            roots=[q for q in before if q.get("root")]
            # Do not overwrite custom shaping configurations, even on CI.
            if len(roots)!=1 or roots[0].get("handle")!="0:" or roots[0].get("kind") not in ("mq","fq_codel","fq","pfifo_fast","noqueue"):
                raise AssertionError("Unexpected existing root qdisc; refusing to replace it")
            item={"device":dev,"destinations":ips,"original_root":roots[0]["kind"]}
            evidence["interfaces"].append(item)
            tc("qdisc","replace","dev",dev,"root","handle","1:","prio","bands","3","priomap",*["0"]*16)
            installed.append(dev)
            options=["loss","100%"] if outage else ["delay","350ms","rate","1000kbit","limit","1000"]
            tc("qdisc","add","dev",dev,"parent","1:3","handle","30:","netem",*options)
            # The Python rendezvous socket is not part of the AVC session.
            for family in ("ip","ipv6"):
                if any((":" in ip)==(family=="ipv6") for ip in ips):
                    tc("filter","add","dev",dev,"parent","1:","protocol",family,"pref","1","flower",
                        "ip_proto","tcp","src_port",str(coordinator_port),"dst_port","443","flowid","1:1")
            for i,ip in enumerate(ips,10):
                family="ipv6" if ":" in ip else "ip"
                tc("filter","add","dev",dev,"parent","1:","protocol",family,"pref",str(i),"flower",
                    "dst_ip",ip,"ip_proto","tcp","dst_port","443","flowid","1:3")
        yield
    finally:
        errors=[]
        for dev in reversed(installed):
            try:
                stats=json.loads(tc("-s","-j","qdisc","show","dev",dev).stdout)
                q=next((q for q in stats if q.get("handle")=="30:"),{})
                item=next(x for x in evidence["interfaces"] if x["device"]==dev)
                item["netem_packets"]=int(q.get("packets",0))
                item["netem_bytes"]=int(q.get("bytes",0))
                item["netem_drops"]=int(q.get("drops",0))
            except Exception as e:
                errors.append("Cannot read netem counters: "+str(e))
            finally:
                result=tc("qdisc","del","dev",dev,"root",check=False)
                if result.returncode:
                    errors.append("Cannot remove test root qdisc: "+result.stderr.strip()[:200])
                else:
                    item=next(x for x in evidence["interfaces"] if x["device"]==dev)
                    item["restored"]=True
        if errors:
            raise AssertionError("; ".join(errors))
