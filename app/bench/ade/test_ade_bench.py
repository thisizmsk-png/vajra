import json, os, sys, tempfile, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ade_bench as b


class MethodTests(unittest.TestCase):
    def test_the_warmup_launch_is_dropped_before_the_median(self):
        m, kept = b.median_of_runs([99.0, 1.0, 2.0, 3.0, 4.0, 5.0])
        self.assertEqual((m, kept), (3.0, [1.0, 2.0, 3.0, 4.0, 5.0]))

    def test_a_failed_launch_is_skipped_and_all_failed_is_none(self):
        self.assertEqual(b.median_of_runs([9, None, 2.0, None, 4.0])[0], 3.0)
        self.assertEqual(b.median_of_runs([9, None, None]), (None, []))

    def test_the_weight_scenario_uses_the_memory_gates_footprint_function(self):
        self.assertIs(b.measure.footprint_mib, __import__("measure").footprint_mib)
        self.assertFalse(hasattr(b, "footprint_mib"))


class TokenTests(unittest.TestCase):
    def ledger(self, lines):
        d = tempfile.mkdtemp()
        os.makedirs(os.path.join(d, "control"))
        open(os.path.join(d, "control", "ledger.jsonl"), "w").write("\n".join(lines) + "\n")
        return d

    def test_only_lines_with_a_token_receipt_count_and_garbage_is_skipped(self):
        rec = lambda i, t: json.dumps({"action_id": i, "kind": "ask", "state": "done", "tokens": t})
        good = {"schema": "vajra.token_receipt/1", "fresh": 3, "cache_write": 10, "cache_read": 200, "output": 7, "side": 0, "total": 220}
        d = self.ledger([rec("a1", good), "not json", json.dumps({"action_id": "a2", "kind": "ask", "state": "queued"}),
                         rec("a3", {"schema": "other/1", "fresh": 1}), rec("a4", {**good, "output": 9, "total": 222})])
        turns = b.read_tokens(d)
        self.assertEqual([t["ask_id"] for t in turns], ["a1", "a4"])
        self.assertEqual(turns[0]["total"], 220)
        self.assertEqual(turns[0]["side"], 0)

    def test_a_missing_ledger_is_an_empty_list(self):
        self.assertEqual(b.read_tokens(tempfile.mkdtemp()), [])


class ReportTests(unittest.TestCase):
    def test_an_unmeasured_metric_is_listed_with_its_reason_never_as_a_number(self):
        md = b.render_report([{"scenario": "weight", "at_rest_footprint_mib": 150.0, "at_rest_rss_mib": 190.0, "idle_cpu_pct": 0.1,
                               "method": "1 warm-up, median of 5", "not_measured": b.NOT_MEASURED}])
        self.assertIn("| at rest, footprint (MiB) | 150.0 | beside RSS 190.0", md)
        self.assertIn("| first_word_ms | not measured | needs a live agent turn |", md)
        self.assertNotIn("| first_word_ms | 0", md)

    def test_tokens_with_no_finished_turn_say_none(self):
        md = b.render_report([{"scenario": "tokens", "hello": None, "real_prompt": None}])
        self.assertIn("| tokens, hello | none | fresh/cache write/cache read/output: no finished turn |", md)


if __name__ == "__main__":
    unittest.main()
