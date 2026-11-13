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


class AnatomyTests(unittest.TestCase):
    def run_lines(self, usage, model_usage, tools=3, mcp=0):
        return [json.dumps({"type": "system", "subtype": "init", "tools": ["t"] * tools, "mcp_servers": [{}] * mcp}),
                "not json",
                json.dumps({"type": "result", "usage": usage, "modelUsage": model_usage, "total_cost_usd": 0.5})]

    def test_first_request_is_fresh_plus_cache_write_plus_cache_read(self):
        u = {"input_tokens": 2, "cache_creation_input_tokens": 3106, "cache_read_input_tokens": 10, "output_tokens": 44}
        mu = {"opus": {"inputTokens": 2, "cacheCreationInputTokens": 3106, "cacheReadInputTokens": 10, "outputTokens": 44}}
        a = b.parse_anatomy(self.run_lines(u, mu, tools=3, mcp=1))
        self.assertEqual((a["first_request_tokens"], a["tools"], a["mcp_servers"], a["side_tokens"]), (3118, 3, 1, 0))

    def test_tokens_billed_to_another_model_are_side_tokens(self):
        u = {"input_tokens": 2, "cache_creation_input_tokens": 1000, "output_tokens": 5}
        mu = {"opus": {"inputTokens": 2, "cacheCreationInputTokens": 1000, "outputTokens": 5}, "haiku": {"inputTokens": 300, "outputTokens": 20}}
        self.assertEqual(b.parse_anatomy(self.run_lines(u, mu))["side_tokens"], 320)

    def test_no_result_event_is_none_and_malformed_usage_reads_zero(self):
        self.assertIsNone(b.parse_anatomy(["not json", json.dumps({"type": "system", "subtype": "init"})]))
        a = b.parse_anatomy(self.run_lines({"input_tokens": -4, "output_tokens": "9"}, {}))
        self.assertEqual(a["first_request_tokens"], 0)

    def test_the_shipped_arms_file_is_valid_and_keeps_the_leads_settings_off(self):
        arms = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "anatomy_arms.json")))
        self.assertEqual(arms["plain_cli"], [])
        self.assertIn("--strict-mcp-config", arms["lead"])
        self.assertEqual(arms["lead"][arms["lead"].index("--setting-sources") + 1], "")


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
