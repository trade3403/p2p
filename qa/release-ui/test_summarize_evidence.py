import json
import unittest
from summarize_evidence import summarize_g1, summarize_g5b2, summarize_release


def g1():
    def host(active, lan, relay, suppressed, opened=True):
        return dict(active=active, lan_bytes=lan*100, lan_frames=lan,
                    relay_bytes=relay*100, relay_frames=relay, suppressed=suppressed,
                    relay_socket_open=opened, relay_socket_opens=1)
    def peer(n):
        return dict(bytes=n*100, frames=n, pong=True)
    def row(stage, data):
        return dict(stage=stage, scope="windows_runner_loopback_ws_jpeg", data=data)
    return [
        row("start", {}),
        row("relay_baseline", dict(before=host(False, 0, 0, 0), after=host(False, 0, 3, 0), relay=peer(3))),
        row("lan_settle", {}),
        row("lan_stable", dict(before=host(True, 1, 3, 1), after=host(True, 4, 3, 4), lan=peer(3), relay=peer(0))),
        row("clipboard_ownership", dict(lan_applied=True, relay_blocked=True)),
        row("relay_resumed", dict(before=host(False, 4, 3, 4), after=host(False, 4, 6, 4), relay=peer(3))),
        row("passed", dict(host=host(False, 4, 6, 4, False))),
    ]


def report():
    def live(i):
        return dict(codec="H264", transport_phase="relay_video", host_metadata_current="true",
                    frames=100+i*300, host_encoded_frames=200+i*300, bps=400000, fps=10,
                    rtt=20+i, relay_socket_opens=1, relay_retry_events=0, drops=0,
                    viewer_relay_video_bytes=1000+i*1000, host_relay_video_bytes=2000+i*1000,
                    viewer_lan_video_bytes=0, host_lan_video_bytes=0, private="PRIVATE_MARKER")
    return dict(role="android", passed=True, requested_stability_seconds=300,
                recording=dict(frames=20, duration_seconds=5, capture_fps=4),
                actual_avc_metrics=dict(first=live(0), second=live(1)),
                avc_stability=dict(minimum_elapsed_seconds=300, expected_samples=10,
                    baseline=live(0), samples=[dict(live(i), elapsed_seconds=i*30) for i in range(1, 11)],
                    elapsed_seconds=300, rendered_frame_delta=3000, decoder_drop_delta=0,
                    observed_average_render_fps=10, rtt_min_ms=21, rtt_max_ms=30, remote_text_verified=True))


def actual_g5b2():
    # Actual Windows job 140767464384, not simulated performance.
    # Unused timing arrays outside the stable window were omitted.
    raw = """{"data":{"host":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":0,"relay_frames":0,"relay_socket_open":false,"relay_socket_opens":0,"suppressed":0}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"start"}
{"data":{"after":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":1218284,"relay_frames":40,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":0},"before":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":0,"relay_frames":0,"relay_socket_open":false,"relay_socket_opens":0,"suppressed":0},"relay":{"bytes":1068214,"decoded_idr":1,"decoded_predictive":32,"decoded_vcl":39,"dimensions":[960,720],"elapsed_ms":10001,"first_decoded_idr_ms":916,"idr_packets":7,"jpeg_packets":1,"packets":39,"pong":true,"pps_packets":7,"sps_packets":7,"vcl_packets":39,"ws_pings":0}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"relay_baseline"}
{"data":{"host":{"active":true,"lan_bytes":843021,"lan_frames":25,"relay_bytes":1218284,"relay_frames":40,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":38},"lan":{"bytes":692951,"decoded_idr":1,"decoded_predictive":20,"decoded_vcl":24,"dimensions":[960,720],"elapsed_ms":10000,"first_decoded_idr_ms":900,"idr_packets":4,"jpeg_packets":1,"packets":24,"pong":true,"pps_packets":4,"sps_packets":4,"vcl_packets":24,"ws_pings":2},"relay":{"bytes":0,"decoded_idr":0,"decoded_predictive":0,"decoded_vcl":0,"dimensions":null,"elapsed_ms":10001,"first_decoded_idr_ms":null,"idr_packets":0,"jpeg_packets":0,"packets":0,"pong":false,"pps_packets":0,"sps_packets":0,"vcl_packets":0,"ws_pings":0}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"lan_startup"}
{"data":{"after":{"active":true,"lan_bytes":1634728,"lan_frames":51,"relay_bytes":1218284,"relay_frames":40,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":77},"before":{"active":true,"lan_bytes":843021,"lan_frames":25,"relay_bytes":1218284,"relay_frames":40,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":38},"lan":{"bytes":791707,"decoded_idr":1,"decoded_predictive":21,"decoded_vcl":26,"dimensions":[960,720],"elapsed_ms":10010,"first_decoded_idr_ms":333,"idr_packets":5,"jpeg_packets":0,"packets":26,"pong":true,"pps_packets":5,"sps_packets":5,"timing_samples":[{"capture_backend":"screenshots compatibility","encoder":"OpenH264 software","observed_ms":1998,"timing":{"capture":{"samples":31,"total_us":1298299},"encode":{"samples":30,"total_us":10577031},"queue_drops":0,"queue_wait":{"samples":30,"total_us":3028},"send":{"samples":30,"total_us":147340}}},{"capture_backend":"screenshots compatibility","encoder":"OpenH264 software","observed_ms":4003,"timing":{"capture":{"samples":36,"total_us":1501763},"encode":{"samples":35,"total_us":12257532},"queue_drops":0,"queue_wait":{"samples":35,"total_us":3674},"send":{"samples":35,"total_us":165532}}},{"capture_backend":"screenshots compatibility","encoder":"OpenH264 software","observed_ms":6011,"timing":{"capture":{"samples":41,"total_us":1716321},"encode":{"samples":40,"total_us":13997103},"queue_drops":0,"queue_wait":{"samples":40,"total_us":4266},"send":{"samples":40,"total_us":191019}}},{"capture_backend":"screenshots compatibility","encoder":"OpenH264 software","observed_ms":8004,"timing":{"capture":{"samples":46,"total_us":1916255},"encode":{"samples":46,"total_us":16042781},"queue_drops":0,"queue_wait":{"samples":46,"total_us":4936},"send":{"samples":46,"total_us":206390}}},{"capture_backend":"screenshots compatibility","encoder":"OpenH264 software","observed_ms":9997,"timing":{"capture":{"samples":52,"total_us":2194092},"encode":{"samples":51,"total_us":17768167},"queue_drops":0,"queue_wait":{"samples":51,"total_us":5530},"send":{"samples":51,"total_us":234107}}}],"vcl_packets":26,"ws_pings":2},"performance":{"decoded_vcl":26,"decoded_vcl_per_second":2.5974025974025974,"encrypted_avc_bps":632732.8671328671,"encrypted_avc_bytes":791707,"received_avc_packets":26,"received_avc_packets_per_second":2.5974025974025974,"receiver_window_ms":10010,"scope":"windows_runner_loopback_real_capture_session_timing","session_timing":{"capture":{"mean_us":42656.80952380953,"samples":21,"total_us":895793},"capture_backend_first":"screenshots compatibility","capture_backend_last":"screenshots compatibility","elapsed_observation_ms":7999,"encode":{"mean_us":342435.04761904763,"samples":21,"total_us":7191136},"encoder_first":"OpenH264 software","encoder_last":"OpenH264 software","metadata_samples":5,"queue_drops":0,"queue_wait":{"mean_us":119.14285714285714,"samples":21,"total_us":2502},"send":{"mean_us":4131.761904761905,"samples":21,"total_us":86767}}},"relay":{"bytes":0,"decoded_idr":0,"decoded_predictive":0,"decoded_vcl":0,"dimensions":null,"elapsed_ms":10010,"first_decoded_idr_ms":null,"idr_packets":0,"jpeg_packets":0,"packets":0,"pong":false,"pps_packets":0,"sps_packets":0,"timing_samples":[{"capture_backend":"DXGI","encoder":"OpenH264 software","observed_ms":1997,"timing":{"capture":{"samples":84,"total_us":627128},"encode":{"samples":83,"total_us":14537981},"queue_drops":0,"queue_wait":{"samples":40,"total_us":15619},"send":{"samples":40,"total_us":134770}}},{"capture_backend":"DXGI","encoder":"OpenH264 software","observed_ms":4003,"timing":{"capture":{"samples":93,"total_us":692960},"encode":{"samples":93,"total_us":15935894},"queue_drops":0,"queue_wait":{"samples":40,"total_us":15619},"send":{"samples":40,"total_us":134770}}},{"capture_backend":"DXGI","encoder":"OpenH264 software","observed_ms":5996,"timing":{"capture":{"samples":101,"total_us":731437},"encode":{"samples":101,"total_us":17309701},"queue_drops":0,"queue_wait":{"samples":40,"total_us":15619},"send":{"samples":40,"total_us":134770}}},{"capture_backend":"DXGI","encoder":"OpenH264 software","observed_ms":8004,"timing":{"capture":{"samples":109,"total_us":777490},"encode":{"samples":108,"total_us":18358353},"queue_drops":0,"queue_wait":{"samples":40,"total_us":15619},"send":{"samples":40,"total_us":134770}}},{"capture_backend":"DXGI","encoder":"OpenH264 software","observed_ms":9995,"timing":{"capture":{"samples":117,"total_us":808304},"encode":{"samples":117,"total_us":19784091},"queue_drops":0,"queue_wait":{"samples":40,"total_us":15619},"send":{"samples":40,"total_us":134770}}}],"vcl_packets":0,"ws_pings":0}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"lan_stable"}
{"data":{"after":{"active":false,"lan_bytes":1634728,"lan_frames":51,"relay_bytes":2256200,"relay_frames":79,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":77},"before":{"active":false,"lan_bytes":1634728,"lan_frames":51,"relay_bytes":1218284,"relay_frames":40,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":77},"relay":{"bytes":1037916,"decoded_idr":1,"decoded_predictive":32,"decoded_vcl":39,"dimensions":[960,720],"elapsed_ms":10001,"first_decoded_idr_ms":483,"idr_packets":7,"jpeg_packets":0,"packets":39,"pong":true,"pps_packets":7,"sps_packets":7,"vcl_packets":39,"ws_pings":0}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"relay_resumed"}
{"data":{"host":{"active":false,"lan_bytes":1634728,"lan_frames":51,"relay_bytes":2256200,"relay_frames":79,"relay_socket_open":false,"relay_socket_opens":1,"suppressed":77}},"scope":"windows_runner_loopback_ws_avc_handoff","stage":"passed"}"""
    return [json.loads(line) for line in raw.splitlines()]


class EvidenceTests(unittest.TestCase):
    def test_actual_g5b2(self):
        result = summarize_g5b2(actual_g5b2())
        self.assertEqual(result["received_avc_packets"], 26)
        self.assertEqual(result["encrypted_avc_bytes"], 791707)
        self.assertEqual(result["session_timing"]["encode"]["total_us"], 7191136)
        self.assertAlmostEqual(result["session_timing"]["encode"]["mean_us"], 342435.04761904763)
        self.assertEqual(result["host_delta"]["relay_bytes"], 0)

    def test_g5b2_rejects_missing_or_inconsistent_measurements(self):
        def stable(rows): return rows[3]["data"]
        mutations = [
            lambda r: r.pop(3),
            lambda r: stable(r)["lan"].update(timing_samples=[]),
            lambda r: stable(r)["lan"]["timing_samples"][1]["timing"]["encode"].update(samples=0),
            lambda r: stable(r)["lan"]["timing_samples"][-1]["timing"]["queue_wait"].update(samples=0),
            lambda r: stable(r)["performance"].update(encrypted_avc_bps=1),
            lambda r: stable(r)["performance"]["session_timing"]["capture"].update(mean_us=0),
            lambda r: stable(r)["relay"].update(bytes=1),
            lambda r: stable(r)["after"].update(relay_frames=41),
            lambda r: r[4]["data"]["after"].update(relay_socket_opens=2),
            lambda r: r[5]["data"]["host"].update(relay_socket_open=True),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutations.index(mutate)):
                rows = actual_g5b2()
                mutate(rows)
                with self.assertRaises(ValueError):
                    summarize_g5b2(rows)

    def test_g1(self):
        result = summarize_g1(g1())
        self.assertEqual(result["phases"]["lan_stable"]["relay_bytes"], 0)

    def test_actual_g1_regression(self):
        # Numeric/boolean CI log evidence, job 140574912947; not a new measurement.
        raw = """{"data":{"host":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":0,"relay_frames":0,"relay_socket_open":false,"relay_socket_opens":0,"suppressed":0}},"scope":"windows_runner_loopback_ws_jpeg","stage":"start"}
{"data":{"after":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":3445753,"relay_frames":23,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":0},"before":{"active":false,"lan_bytes":0,"lan_frames":0,"relay_bytes":0,"relay_frames":0,"relay_socket_open":false,"relay_socket_opens":0,"suppressed":0},"relay":{"bytes":3445753,"frames":23,"pong":true,"ws_pings":0}},"scope":"windows_runner_loopback_ws_jpeg","stage":"relay_baseline"}
{"data":{"host":{"active":true,"lan_bytes":1198532,"lan_frames":8,"relay_bytes":3445753,"relay_frames":23,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":8},"lan":{"bytes":1198532,"frames":8,"pong":false,"ws_pings":0},"relay":{"bytes":0,"frames":0,"pong":false,"ws_pings":0}},"scope":"windows_runner_loopback_ws_jpeg","stage":"lan_settle"}
{"data":{"accepted_jpeg_codec_controls":1,"after":{"active":true,"lan_bytes":4494601,"lan_frames":30,"relay_bytes":3445753,"relay_frames":23,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":30},"before":{"active":true,"lan_bytes":1198532,"lan_frames":8,"relay_bytes":3445753,"relay_frames":23,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":8},"lan":{"bytes":3296069,"frames":22,"pong":true,"ws_pings":3},"relay":{"bytes":0,"frames":0,"pong":false,"ws_pings":0}},"scope":"windows_runner_loopback_ws_jpeg","stage":"lan_stable"}
{"data":{"lan_applied":true,"relay_blocked":true},"scope":"windows_runner_loopback_ws_jpeg","stage":"clipboard_ownership"}
{"data":{"after":{"active":false,"lan_bytes":6442189,"lan_frames":43,"relay_bytes":6891506,"relay_frames":46,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":43},"before":{"active":false,"lan_bytes":6442189,"lan_frames":43,"relay_bytes":3445753,"relay_frames":23,"relay_socket_open":true,"relay_socket_opens":1,"suppressed":43},"relay":{"bytes":3445753,"frames":23,"pong":true,"ws_pings":0}},"scope":"windows_runner_loopback_ws_jpeg","stage":"relay_resumed"}
{"data":{"host":{"active":false,"lan_bytes":6442189,"lan_frames":43,"relay_bytes":6891506,"relay_frames":46,"relay_socket_open":false,"relay_socket_opens":1,"suppressed":43}},"scope":"windows_runner_loopback_ws_jpeg","stage":"passed"}"""
        result = summarize_g1([json.loads(line) for line in raw.splitlines()])
        self.assertEqual(result["phases"]["lan_stable"]["lan_bytes"], 3296069)
        self.assertEqual(result["phases"]["lan_stable"]["relay_bytes"], 0)
        self.assertEqual(result["clipboard_ownership"],
                         {"lan_applied": True, "relay_blocked": True})

    def test_g1_bad_peer(self):
        rows = g1()
        rows[3]["data"]["relay"]["bytes"] = 1
        with self.assertRaises(ValueError):
            summarize_g1(rows)

    def test_missing_or_misordered_ownership_rejected(self):
        rows = g1()
        del rows[4]
        with self.assertRaises(ValueError):
            summarize_g1(rows)
        rows = g1()
        rows[4], rows[5] = rows[5], rows[4]
        with self.assertRaises(ValueError):
            summarize_g1(rows)

    def test_invalid_ownership_rejected(self):
        for key in ("lan_applied", "relay_blocked"):
            rows = g1()
            rows[4]["data"][key] = False
            with self.assertRaises(ValueError):
                summarize_g1(rows)

    def test_release(self):
        result = summarize_release(report(), "android")
        self.assertEqual(result["stability"]["average_output_fps"], 10)
        self.assertNotIn("PRIVATE_MARKER", str(result))

    def test_release_recording_disabled(self):
        raw = report()
        del raw["recording"]
        raw["recording_enabled"] = False
        result = summarize_release(raw, "android")
        self.assertFalse(result["recording_enabled"])
        self.assertNotIn("recording", result)
        self.assertEqual(result["stability"]["average_output_fps"], 10)
        raw["recording"] = {"frames": 20}
        with self.assertRaises(ValueError):
            summarize_release(raw, "android")

    def test_windows_recording_disabled(self):
        raw = dict(role="windows", passed=True, requested_stability_seconds=300,
                   recording_enabled=False, peer_stability_elapsed_seconds=301,
                   peer_stability_sample_count=10, avc_stability_text_verified=True)
        self.assertFalse(summarize_release(raw, "windows")["recording_enabled"])
        raw["avc_stability_text_verified"] = False
        with self.assertRaises(ValueError):
            summarize_release(raw, "windows")

    def test_reconnect_rejected(self):
        raw = report()
        raw["avc_stability"]["samples"][3]["relay_socket_opens"] = 2
        with self.assertRaises(ValueError):
            summarize_release(raw, "android")

    def test_bad_summary_rejected(self):
        raw = report()
        raw["avc_stability"]["rendered_frame_delta"] = 3001
        with self.assertRaises(ValueError):
            summarize_release(raw, "android")

    def test_windows(self):
        raw = dict(role="windows", passed=True, requested_stability_seconds=300,
                   recording=dict(frames=20, duration_seconds=5, capture_fps=4),
                   peer_stability_elapsed_seconds=301, peer_stability_sample_count=10,
                   avc_stability_text_verified=True)
        self.assertEqual(summarize_release(raw, "windows")["peer_stability"]["sample_count"], 10)
        raw["avc_stability_text_verified"] = False
        with self.assertRaises(ValueError):
            summarize_release(raw, "windows")


if __name__ == "__main__":
    unittest.main()
