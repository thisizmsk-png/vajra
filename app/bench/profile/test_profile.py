"""Unit tests for the profile sampler's pure parts. Run: python3 -m unittest app/bench/profile/test_fg12_profile.py"""
import os, signal, subprocess, sys, time, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fg12_profile as P  # noqa: E402

KB = 1024
# pid: (ppid, rss_kb, cpu_s, comm)
ROWS = {
    100: (1, 100 * KB, 10.0, "vajra-desktop"),
    101: (100, 20 * KB, 1.0, "python3"),      # run 1 root
    102: (101, 5 * KB, 0.5, "node"),          # run 1 child
    103: (100, 14 * KB, 0.2, "-zsh"),         # terminal 1
    104: (103, 3 * KB, 0.1, "sleep"),         # terminal 1 job
    105: (100, 4 * KB, 0.0, "-zsh"),          # terminal 2
    200: (1, 60 * KB, 5.0, "/x/com.apple.WebKit.WebContent"),
    201: (1, 22 * KB, 1.0, "/x/com.apple.WebKit.Networking"),
    202: (1, 99 * KB, 1.0, "/x/com.apple.WebKit.WebContent"),  # foreign: existed before launch
}


class Classify(unittest.TestCase):
    def test_given_runs_and_terminals_when_classified_then_each_component_is_separate(self):
        comp, total, helpers = P.classify(ROWS, 100, [101], before={202})
        self.assertEqual(comp["app"][0], 100)
        self.assertEqual([round(r) for r, _ in comp["runs"]], [25])  # root + child
        self.assertEqual([round(r) for r, _ in comp["terminals"]], [17, 4])  # per terminal, job included
        self.assertEqual({k: round(v[0]) for k, v in comp["webkit"].items()}, {"Networking": 22, "WebContent": 60})
        self.assertEqual(helpers, 2)
        self.assertEqual(total, 100 + 25 + 21 + 82)

    def test_given_a_foreign_webkit_helper_when_classified_then_it_is_not_counted(self):
        comp, _, _ = P.classify(ROWS, 100, [101], before={202, 200, 201})
        self.assertEqual(comp["webkit"], {})

    def test_given_a_run_root_that_is_gone_when_classified_then_it_is_skipped_not_guessed_by_name(self):
        comp, _, _ = P.classify(ROWS, 100, [999], before={200, 201, 202})
        self.assertEqual(comp["runs"], [])
        self.assertEqual(len(comp["terminals"]), 3)  # 101 is now just another child; nothing matches by process name

    def test_helper_types_come_from_the_name(self):
        self.assertEqual(P.helper_type("/a/com.apple.WebKit.GPU"), "GPU")
        self.assertEqual(P.helper_type("zsh"), "Unknown")


def fake_sample(t, total, helpers=2, load=1.0, app=100.0):
    comp = {"app": (app, t), "runs": [(25.0, 0)], "terminals": [], "webkit": {"WebContent": (60.0, 0)}}
    return {"t": t, "comp": comp, "total_mib": total, "helpers": helpers, "cpu_all": t * 0.01, "load_1m": load, "front": "Terminal"}


class Summarize(unittest.TestCase):
    def test_given_noisy_samples_when_summarized_then_foreign_helpers_and_busy_load_are_left_out(self):
        s = [fake_sample(0, 200), fake_sample(2, 202), fake_sample(4, 400, helpers=4), fake_sample(6, 900, load=9.0), fake_sample(8, 201)]
        r = P.summarize(s)
        self.assertEqual(r["clean_samples"], 3)
        self.assertEqual(r["rss_mib"]["total"], 201)
        self.assertEqual(r["total_min_max"], [200, 202])

    def test_given_no_clean_sample_when_summarized_then_all_samples_are_used(self):
        r = P.summarize([fake_sample(0, 200, load=9.0), fake_sample(2, 210, load=9.0)])
        self.assertEqual(r["clean_samples"], 0)
        self.assertEqual(r["rss_mib"]["total"], 205)

    def test_given_no_samples_when_summarized_then_it_says_so(self):
        self.assertEqual(P.summarize([]), {"samples": 0})

    def test_cpu_is_the_delta_over_the_window(self):
        r = P.summarize([fake_sample(0, 1), fake_sample(10, 1)])
        self.assertEqual(r["cpu_percent_of_one_core"]["app"], 100.0)


class Bounds(unittest.TestCase):
    def test_the_limits_are_finite(self):
        self.assertLessEqual(P.MAX_SECONDS, 900)
        self.assertLessEqual(P.MAX_SAMPLES * P.STEP_S, 1200)

    def test_cancel_wakes_a_nap_within_a_fraction_of_a_second(self):
        P.CANCEL["flag"] = False
        t0 = time.time()
        import threading
        threading.Timer(0.2, lambda: P.CANCEL.update(flag=True)).start()
        P.nap(30)
        self.assertLess(time.time() - t0, 2)
        P.CANCEL["flag"] = False

    def test_sigterm_stops_a_real_run_and_still_writes_partial_results(self):
        """A stand-in 'app' (sleep) is sampled; SIGTERM mid-run ends quickly, kills only its own group, writes cancelled."""
        import tempfile, json
        out = os.path.join(tempfile.mkdtemp(), "o.json")
        env = dict(os.environ, FG12_FIXTURE="/nonexistent")
        script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fg12_profile.py")
        fake = os.path.join(tempfile.mkdtemp(), "harness")
        open(fake, "w").write("#!/bin/sh\necho PID $$\necho 'RUNS []'\nexec sleep 300\n")
        os.chmod(fake, 0o755)
        # settle is 25 s by default: shorten it for the test through the environment-free module constant
        runner = subprocess.Popen([sys.executable, "-c", f"import sys; sys.path.insert(0, {os.path.dirname(script)!r}); import fg12_profile as P; P.SETTLE_S=0; P.STEP_S=0.2; sys.argv=['fg12_profile.py','core',{fake!r},'idle',{out!r},'60']; P.main()"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2)
        runner.send_signal(signal.SIGTERM)
        runner.wait(timeout=15)
        data = json.load(open(out))
        self.assertTrue(data["summary"]["cancelled"])
        self.assertGreater(len(data["samples"]), 0)
        self.assertEqual(subprocess.run(["pgrep", "-f", fake], capture_output=True).returncode, 1, "the harness stand-in must be gone")

    def test_cancel_also_ends_session_leaders_the_harness_started(self):
        """Terminal shells are their own sessions, so killing the harness's process group misses them: a harness that dies
        on SIGTERM without cleanup must not leave its descendants behind."""
        import tempfile
        out = os.path.join(tempfile.mkdtemp(), "o.json")
        script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fg12_profile.py")
        marker = "301"  # a unique sleep length to find the orphan by
        fake = os.path.join(tempfile.mkdtemp(), "harness")
        with open(fake, "w") as f:
            # like the real harness: a child in its own session, still our child when we are told to stop; then keep running
            f.write("#!/usr/bin/env python3\nimport os, subprocess\nprint('PID', os.getpid(), flush=True)\nprint('RUNS []', flush=True)\n"
                    f"subprocess.Popen(['sleep', '{marker}'], start_new_session=True)\nos.execvp('sleep', ['sleep', '300'])\n")
        os.chmod(fake, 0o755)
        env = dict(os.environ, FG12_FIXTURE="/nonexistent")
        runner = subprocess.Popen([sys.executable, "-c", f"import sys; sys.path.insert(0, {os.path.dirname(script)!r}); import fg12_profile as P; P.SETTLE_S=0; P.STEP_S=0.2; sys.argv=['x','core',{fake!r},'idle',{out!r},'60']; P.main()"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2.5)
        self.assertEqual(subprocess.run(["pgrep", "-f", f"sleep {marker}"], capture_output=True).returncode, 0, "the session leader exists before the cancel")
        runner.send_signal(signal.SIGTERM)
        runner.wait(timeout=30)
        time.sleep(0.5)
        self.assertEqual(subprocess.run(["pgrep", "-f", f"sleep {marker}"], capture_output=True).returncode, 1, "the harness's own session leader must be gone after cancel")


if __name__ == "__main__":
    unittest.main()
