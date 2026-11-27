import json
import unittest
from summarize_evidence import summarize_g1, summarize_release


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


class EvidenceTests(unittest.TestCase):
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
