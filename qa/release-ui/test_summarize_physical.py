"""Synthetic unit cases only; never physical performance evidence."""
import unittest
from datetime import datetime, timedelta, timezone
from summarize_physical import summarize


def series(role, transport="lan"):
    started = datetime(2026, 1, 1, tzinfo=timezone.utc)
    result = dict(schema_version=1, sample_kind=role+"_series",
                  started_at_utc=started.isoformat(), nominal_interval_ms=2000,
                  sample_limit=300, stop_reason="user", samples=[], analysis={"untrusted": True})
    for i in range(6):
        m = dict()
        for path in ("lan", "relay"):
            m[role+"_"+path+"_video_bytes"] = 100+i*1000 if path == transport else 50
            m[role+"_"+path+"_video_frames"] = 10+i*4 if path == transport else 5
        if role == "host":
            m.update(host_running=True, host_lan_active=transport=="lan",
                     host_relay_socket_open=True, host_relay_socket_opens=1,
                     host_relay_suppressed_frames=i if transport=="lan" else 0)
        else:
            m.update(viewer_connected=True, viewer_transport_phase=transport+"_video",
                     active_video_codec="H264", avc_decoder_recovering=False,
                     relay_socket_opens=1, relay_retry_events=0, avc_rendered_frames=10+i*3,
                     avc_decoder_drops=0, rtt_ms=10+i)
        m["private"] = "SECRET_MARKER"
        result["samples"].append(dict(schema_version=1, sample_kind=role+"_snapshot",
            captured_at_utc=(started+timedelta(seconds=i*2)).isoformat(),
            elapsed_ms=i*2000, metrics=m))
    return result


class PairedSeriesTests(unittest.TestCase):
    def test_both_transports(self):
        for transport in ("lan", "relay"):
            r = summarize(series("host",transport),series("viewer",transport),transport)
            self.assertEqual(r["host"]["active_encrypted_video_bps"],4000)
            self.assertEqual(r["viewer"]["avc_output_per_second"],1.5)
            self.assertEqual(r["utc_overlap_hint_ms"],10000)
            self.assertFalse(r["physical_device_acceptance_verified"])
            self.assertNotIn("SECRET_MARKER",str(r))
            self.assertNotIn("untrusted",str(r))

    def test_measured_resource_arithmetic_and_missing(self):
        h,v=series("host"),series("viewer")
        for i,s in enumerate(v["samples"]):
            s["metrics"].update(process_cpu_ms=100+i*3000, process_rss_bytes=4096+i*1024,
                video_encoder="OpenH264 software", capture_backend="DXGI",
                decoder_backend_class="software", process_gpu_utilization_percent=99)
        r=summarize(h,v,"lan")
        self.assertEqual(r["viewer"]["resources"]["process_cpu_one_core_percent_mean"],150)
        self.assertEqual(r["viewer"]["resources"]["process_rss_max_bytes"],9216)
        self.assertEqual(r["viewer"]["resources"]["video_encoder_sample_counts"],{"OpenH264 software":6})
        self.assertIsNone(r["viewer"]["resources"]["process_gpu_utilization_percent"])
        self.assertIsNone(r["host"]["resources"]["process_cpu_one_core_percent_mean"])
        v["samples"][2]["metrics"]["process_cpu_ms"]=None
        r=summarize(h,v,"lan")
        self.assertIsNone(r["viewer"]["resources"]["process_cpu_one_core_percent_mean"])
        self.assertEqual(r["viewer"]["resources"]["process_cpu_interval_count"],3)

    def test_invalid_resources(self):
        for key,value in (("process_cpu_ms",True),("process_rss_bytes",0),
                          ("video_encoder","PRIVATE_MARKER"),("decoder_backend_class","unknown")):
            h,v=series("host"),series("viewer")
            v["samples"][1]["metrics"][key]=value
            with self.assertRaises(ValueError):
                summarize(h,v,"lan")
        h,v=series("host"),series("viewer")
        v["samples"][0]["metrics"]["process_cpu_ms"]=100
        v["samples"][2]["metrics"]["process_cpu_ms"]=99
        with self.assertRaises(ValueError):
            summarize(h,v,"lan")

    def test_independent_windows(self):
        h,v=series("host"),series("viewer")
        for i,s in enumerate(v["samples"]):
            s["elapsed_ms"]=i*2100
        r=summarize(h,v,"lan")
        self.assertEqual(r["host"]["duration_ms"],10000)
        self.assertEqual(r["viewer"]["duration_ms"],10500)

    def test_bad_metrics(self):
        for key,value in (("viewer_lan_video_bytes",False),("viewer_lan_video_bytes",None),
                          ("viewer_lan_video_bytes",0),("viewer_lan_video_bytes",2100),
                          ("viewer_relay_video_bytes",51),("viewer_relay_video_frames",6),
                          ("viewer_transport_phase","relay_video"),("active_video_codec","JPEG"),
                          ("avc_rendered_frames",10),("avc_decoder_recovering",True),
                          ("relay_socket_opens",2),("relay_retry_events",1),("rtt_ms",float("nan"))):
            with self.subTest(key=key,value=value):
                h,v=series("host"),series("viewer")
                v["samples"][3]["metrics"][key]=value
                with self.assertRaises(ValueError):
                    summarize(h,v,"lan")

    def test_bad_series(self):
        for mutate in (lambda v: v.update(stop_reason=None),
                       lambda v: v.update(stop_reason="snapshot_failed"),
                       lambda v: v["samples"].pop(),
                       lambda v: v["samples"][3].update(elapsed_ms=12000),
                       lambda v: v["samples"][3].update(elapsed_ms=4000),
                       lambda v: v["samples"][3].update(captured_at_utc="2026-01-01T00:00:01Z"),
                       lambda v: v["samples"][3].update(schema_version=True)):
            h,v=series("host"),series("viewer")
            mutate(v)
            with self.assertRaises(ValueError):
                summarize(h,v,"lan")

    def test_wrong_role_and_transport(self):
        for h,v,mode in ((series("viewer"),series("host"),"lan"),
                         (series("host"),series("viewer","relay"),"lan")):
            with self.assertRaises(ValueError):
                summarize(h,v,mode)


if __name__ == "__main__":
    unittest.main()
