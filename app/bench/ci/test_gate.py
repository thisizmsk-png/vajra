import unittest
from gate import check, evaluate, idle_summary, post_only
from measure import GATED, LAUNCHES, WARMUP, classify, flavour_key, flavour_summary, idle_report, identity_key, launch_flavours, launch_summary, helper_type, postload_summary

ID = "25D1|20622.1.22"  # macOS build | WebKit version
NW = "Networking+WebContent"
GPU = "GPU+Networking+WebContent"


def bl(mib=150, flavour=NW, key=ID, **extra):
    return {"identities": {key: {"flavours": {flavour: dict({"idle_mib": mib}, **extra)}}}}


def run(v, flavour=NW, key=ID, **extra):
    return dict({"median_of_medians_mib": v, "identity": key, "flavour": flavour, "launch_flavours": [flavour]}, **extra)


class Gate(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(check(bl(150), run(152), {"slope_mib_per_min": 0.2}, ci=False), [])

    def test_absolute_budget_gates_the_no_gpu_flavour(self):
        self.assertEqual(len(check(bl(200), run(201), None, ci=False)), 1)
        self.assertEqual(check(bl(200), run(200), None, ci=False), [])

    def test_regression_over_five(self):
        self.assertEqual(len(check(bl(150), run(156), None, ci=False)), 1)
        self.assertEqual(check(bl(150), run(155), None, ci=False), [])

    def test_tolerance_is_the_larger_of_5_mib_and_3_percent(self):
        self.assertEqual(check(bl(174), run(179.2), None, ci=False), [])   # 3% = 5.22
        self.assertEqual(len(check(bl(174), run(179.3), None, ci=False)), 1)
        self.assertEqual(check(bl(100), run(105), None, ci=False), [])     # 5 MiB floor
        self.assertEqual(len(check(bl(100), run(105.1), None, ci=False)), 1)

    def test_slope(self):
        self.assertEqual(len(check(bl(), run(100), {"slope_mib_per_min": 1.5}, ci=False)), 1)


class Budget(unittest.TestCase):
    """Owner ruling: 240 MiB on CI only when the GPU process is present (raised from 212, owner fine with idle above 220); real machines and the no-GPU flavour stay at 200."""
    gpu = bl(204.0, GPU)

    def test_gpu_on_ci_at_205_passes(self):
        self.assertEqual(check(self.gpu, run(205, GPU), None, ci=True), [])

    def test_gpu_on_ci_over_240_fails(self):
        f = check(bl(238.0, GPU), run(240.1, GPU), None, ci=True)
        self.assertTrue(any("240" in x for x in f), f)
        self.assertEqual(check(bl(238.0, GPU), run(240.0, GPU), None, ci=True), [])

    def test_gpu_on_ci_at_the_old_212_ceiling_and_the_212_6_reading_now_pass(self):
        self.assertEqual(check(bl(212.0, GPU), run(212.6, GPU), None, ci=True), [])

    def test_gpu_not_on_ci_at_205_fails(self):
        f = check(self.gpu, run(205, GPU), None, ci=False)
        self.assertTrue(any("200" in x for x in f), f)

    def test_no_gpu_at_205_fails_even_on_ci(self):
        f = check(bl(204.0, NW), run(205, NW), None, ci=True)
        self.assertTrue(any("200" in x for x in f), f)

    def test_ci_is_read_from_github_actions_by_default(self):
        import os
        old = os.environ.pop("GITHUB_ACTIONS", None)
        try:
            self.assertEqual(len(check(self.gpu, run(205, GPU), None)), 1)
            os.environ["GITHUB_ACTIONS"] = "true"
            self.assertEqual(check(self.gpu, run(205, GPU), None), [])
        finally:
            os.environ.pop("GITHUB_ACTIONS", None)
            if old is not None:
                os.environ["GITHUB_ACTIONS"] = old

    def test_a_provisional_entry_still_fails_the_budget_not_just_warns(self):
        t = bl(150.0, GPU, provisional=True)
        self.assertEqual(len(evaluate(t, run(241, GPU), None, ci=True)[0]), 1)


class Flavours(unittest.TestCase):
    two_three = {"identities": {ID: {"flavours": {NW: {"idle_mib": 175.0}, GPU: {"idle_mib": 190.0}}}}}

    def test_each_flavour_is_compared_to_its_own_baseline(self):
        self.assertEqual(check(self.two_three, run(176, NW), None, ci=False), [])
        self.assertEqual(check(self.two_three, run(191, GPU), None, ci=False), [])
        self.assertEqual(len(check(self.two_three, run(190.0 + 5.8, GPU), None, ci=True)), 1)   # 3% of 190 = 5.7
        self.assertEqual(len(check(self.two_three, run(190.0, NW), None, ci=False)), 1)          # over 175 + 5.25

    def test_a_provisional_entry_only_warns_on_regression(self):
        t = bl(150.0, GPU, provisional=True)
        fails, warns = evaluate(t, run(190, GPU), None, ci=True)
        self.assertEqual(fails, [])
        self.assertEqual(len(warns), 1)
        self.assertIn("provisional", warns[0])
        self.assertIn("190", warns[0])

    def test_a_provisional_entry_still_fails_growth_and_unknowns(self):
        t = bl(150.0, GPU, provisional=True)
        self.assertEqual(len(evaluate(t, run(150, GPU), {"slope_mib_per_min": 2}, ci=True)[0]), 1)  # growth
        self.assertEqual(len(evaluate(t, run(150, NW), None, ci=True)[0]), 1)                       # no such flavour entry
        self.assertEqual(len(evaluate(t, run(150, GPU, "NEW"), None, ci=True)[0]), 1)               # unknown identity

    def test_a_non_provisional_entry_gates_the_regression(self):
        t = bl(150.0, GPU, provisional=False)
        self.assertEqual(len(evaluate(t, run(190, GPU), None, ci=True)[0]), 1)

    def test_gpu_flavour_still_gates_growth(self):
        fails, _ = evaluate(self.two_three, run(190, GPU), {"slope_mib_per_min": 2}, ci=True)
        self.assertEqual(len(fails), 1)

    def test_an_unknown_helper_type_fails_closed(self):
        f = check(self.two_three, run(150, NW, unknown_types=["Plugin"]), None, ci=True)
        self.assertEqual(len(f), 1)
        self.assertIn("Plugin", f[0])
        f = check(self.two_three, run(150, "Networking+Plugin+WebContent"), None, ci=True)
        self.assertEqual(len(f), 1)
        self.assertIn("fails closed", f[0])

    def test_an_unknown_launch_flavour_hidden_behind_the_majority_fails(self):
        f = check(self.two_three, run(176, NW, launch_flavours=[NW, "Plugin+WebContent"]), None, ci=True)
        self.assertEqual(len(f), 1)

    def test_no_usable_launch_fails_closed(self):
        self.assertEqual(len(check(self.two_three, run(None, None, launch_flavours=[GPU]), None, ci=True)), 1)

    def test_churn_warns_but_does_not_fail(self):
        fails, warns = evaluate(self.two_three, run(176, NW, churn_launches=2, excluded_launches=1), None, ci=True)
        self.assertEqual(fails, [])
        self.assertTrue(any("churn" in w for w in warns), warns)

    def test_a_known_identity_without_that_flavour_fails_closed(self):
        f = check(bl(150, NW), run(150, GPU), None, ci=True)
        self.assertEqual(len(f), 1)
        self.assertIn("baseline.json", f[0])

    def test_unknown_identity_fails_closed_with_the_identity_in_the_message(self):
        f = check(bl(), run(100, NW, "NEW"), None, ci=False)
        self.assertEqual(len(f), 1)
        self.assertIn("NEW", f[0])
        self.assertIn("baseline.json", f[0])

    def test_other_identity_does_not_borrow_a_baseline(self):
        t = bl(150, NW)
        t["identities"]["B"] = {"flavours": {GPU: {"idle_mib": 192}}}
        self.assertEqual(check(t, run(192, GPU, "B"), None, ci=True), [])
        self.assertEqual(len(check(t, run(192, GPU, ID), None, ci=True)), 1)

    def test_unknown_identity_still_enforces_the_budget_for_no_gpu_and_slope(self):
        self.assertEqual(len(check(bl(), run(201, NW, "NEW"), {"slope_mib_per_min": 2}, ci=False)), 3)

    def test_missing_identity_in_idle_json_fails_closed(self):
        self.assertEqual(len(check(bl(), {"median_of_medians_mib": 100, "flavour": NW, "launch_flavours": [NW]}, None, ci=False)), 1)

    def test_key_is_os_build_and_webkit_not_the_image_version(self):
        self.assertEqual(identity_key("25D1", "20622.1.22"), "25D1|20622.1.22")


class Measure(unittest.TestCase):
    def r(self, median, flavour):
        return {"median_mib": median, "flavour": flavour}

    def s(self, total, **types):
        return {"total_mib": total, "helpers": sum(types.values()), "types": types}

    def test_helper_type_from_a_process_path(self):
        self.assertEqual(helper_type("/S/L/F/WebKit.framework/XPCServices/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU"), "GPU")
        self.assertEqual(helper_type("com.apple.WebKit.WebContent"), "WebContent")

    def test_the_type_set_decides_the_flavour_not_the_count(self):
        self.assertEqual(flavour_key({"WebContent": 1, "GPU": 1, "Networking": 1}), GPU)
        self.assertEqual(flavour_key({"WebContent": 2, "Networking": 1}), NW)

    def test_five_helper_churn_with_two_duplicate_gpu_classifies_as_the_gpu_flavour(self):
        key, churn, unknown = classify({"GPU": 3, "Networking": 1, "WebContent": 1})   # 3 + 2 duplicate GPU = 5 helpers
        self.assertEqual((key, churn, unknown), (GPU, True, []))
        samples = [self.s(201, GPU=1, Networking=1, WebContent=1), self.s(410, GPU=3, Networking=1, WebContent=1),
                   self.s(202, GPU=1, Networking=1, WebContent=1), self.s(203, GPU=1, Networking=1, WebContent=1)]
        ls = launch_summary(samples)
        self.assertEqual((ls["flavour"], ls["median"], ls["clean_samples"], ls["churn"]), (GPU, 202, 3, True))

    def test_a_launch_that_churns_in_every_sample_is_excluded_but_still_classified(self):
        samples = [self.s(400, GPU=3, Networking=1, WebContent=1), self.s(410, GPU=3, Networking=1, WebContent=1)]
        ls = launch_summary(samples)
        self.assertEqual((ls["flavour"], ls["median"], ls["churn"]), (GPU, None, True))

    def test_churned_launches_are_left_out_of_the_median(self):
        runs = [self.r(201, GPU), self.r(None, GPU), self.r(203, GPU)]
        self.assertEqual(flavour_summary(runs), (GPU, 202, 2, 1))

    def test_foreign_helpers_of_a_known_type_are_dropped(self):
        samples = [self.s(170, Networking=1, WebContent=1), self.s(172, Networking=1, WebContent=1),
                   self.s(260, Networking=2, WebContent=2, GPU=1)]
        ls = launch_summary(samples)
        self.assertEqual((ls["flavour"], ls["median"], ls["clean_samples"]), (NW, 171, 2))

    def test_an_unknown_type_only_in_a_dropped_sample_still_fails_the_gate(self):
        samples = [self.s(175, Networking=1, WebContent=1), self.s(176, Networking=1, WebContent=1),
                   self.s(260, Networking=1, WebContent=1, Plugin=1)]
        ls = launch_summary(samples)
        self.assertEqual((ls["flavour"], ls["median"], ls["unknown_types"]), (NW, 175.5, ["Plugin"]))
        runs = [dict(ls, median_mib=ls["median"])]
        key, med, _, _ = flavour_summary(runs)
        idle = {"identity": ID, "flavour": key, "median_of_medians_mib": med, "launch_flavours": launch_flavours(runs),
                "unknown_types": sorted({t for r in runs for t in r["unknown_types"]})}
        fails, _ = evaluate(bl(175.0, NW), idle, None, ci=True)
        self.assertTrue(any("Plugin" in f for f in fails), fails)

    def test_an_unknown_type_in_the_counted_helpers_is_reported(self):
        ls = launch_summary([self.s(170, Networking=1, WebContent=1, Plugin=1)])
        self.assertEqual(ls["unknown_types"], ["Plugin"])
        self.assertEqual(classify({"Plugin": 1})[2], ["Plugin"])

    def test_the_majority_flavour_wins_and_the_minority_launch_is_left_out(self):
        runs = [self.r(175, NW), self.r(174, NW), self.r(176, NW), self.r(201, GPU), self.r(202, GPU)]
        self.assertEqual(flavour_summary(runs), (NW, 175, 3, 0))

    def test_the_median_is_taken_only_over_launches_of_the_reported_flavour(self):
        runs = [self.r(175, NW), self.r(201, GPU), self.r(202, GPU), self.r(200.8, GPU), self.r(201.1, GPU)]
        self.assertEqual(flavour_summary(runs), (GPU, 201.05, 4, 0))

    def test_a_tie_between_flavours_reports_the_larger_type_set(self):
        self.assertEqual(flavour_summary([self.r(175, NW), self.r(201, GPU)]), (GPU, 201, 1, 0))

    def test_launch_flavours_lists_every_flavour_seen(self):
        self.assertEqual(launch_flavours([self.r(175, NW), self.r(None, GPU)]), [GPU, NW])

    def test_a_flavour_without_a_baseline_entry_fails_even_when_the_majority_has_one(self):
        runs = [self.r(175, NW), self.r(174, NW), self.r(176, NW), self.r(201, GPU), self.r(202, GPU)]
        key, med, _, _ = flavour_summary(runs)
        idle = {"identity": ID, "flavour": key, "median_of_medians_mib": med, "launch_flavours": launch_flavours(runs)}
        fails, _ = evaluate(bl(flavour=NW), idle, None, ci=True)
        self.assertTrue(any("no baseline entry" in f for f in fails), fails)


IDENT = {"key": ID}


def launches(meds, flavours=None, footprints=None, unknown=None):
    """Launch records as idle() writes them: the first one is the warm-up."""
    flavours = flavours or [NW] * len(meds)
    footprints = footprints or [90.0] * len(meds)
    unknown = unknown or [[] for _ in meds]
    return [{"run": i + 1, "warmup": i == 0, "median_mib": m, "flavour": f, "clean_samples": 15, "churn": False,
             "unknown_types": u, "footprint_mib": fp} for i, (m, f, fp, u) in enumerate(zip(meds, flavours, footprints, unknown))]


class WarmupAndMedian(unittest.TestCase):
    def test_launch_1_is_left_out_of_the_median(self):
        # the shape of a real CI run: a high first launch, then a mix of settled and high ones (#166 read 201.6 first)
        rep = idle_report(launches([201.6, 184.8, 191.6, 185.7, 191.2, 185.0]), IDENT)
        self.assertEqual((rep["gated_launches"], rep["warmup_launches"], rep["median_of_medians_mib"]), (5, 1, 185.7))
        self.assertEqual(rep["gated_spread_mib"], [184.8, 191.6])
        # the reference fields keep describing every usable launch, warm-up included
        self.assertEqual(rep["spread_mib"], [184.8, 201.6])
        self.assertEqual(rep["all_launches_median_mib"], 188.45)

    def test_the_median_of_the_launches_after_the_warmup_is_what_is_gated(self):
        rep = idle_report(launches([201.6, 184.8, 191.6, 185.7, 191.2, 185.0]), IDENT)
        self.assertEqual(check(bl(181.6), rep, None, ci=False), [])

    def test_a_real_regression_still_fails(self):
        # every gated launch is 8 MiB over the 181.6 baseline: leaving launch 1 out must not hide it
        rep = idle_report(launches([201.6, 190.0, 190.4, 189.6, 190.2, 189.9]), IDENT)
        self.assertEqual(len(check(bl(181.6), rep, None, ci=False)), 1)

    def test_the_baseline_and_the_tolerance_are_unchanged(self):
        self.assertEqual(check(bl(181.6), run(186.6), None, ci=False), [])
        self.assertEqual(len(check(bl(181.6), run(187.1), None, ci=False)), 1)

    def test_an_unknown_helper_type_in_the_warmup_still_fails(self):
        rep = idle_report(launches([201.6, 185, 185, 185, 185, 185], unknown=[["Plugin"], [], [], [], [], []]), IDENT)
        self.assertEqual(rep["unknown_types"], ["Plugin"])
        self.assertTrue(any("Plugin" in f for f in check(bl(181.6), rep, None, ci=False)))

    def test_an_unbaselined_flavour_in_the_warmup_still_fails(self):
        rep = idle_report(launches([201.6, 185, 185, 185, 185, 185], flavours=[GPU, NW, NW, NW, NW, NW]), IDENT)
        self.assertEqual(rep["launch_flavours"], [GPU, NW])
        self.assertTrue(any("no baseline entry" in f for f in check(bl(181.6), rep, None, ci=False)))

    def test_footprint_is_recorded_beside_rss_and_never_gated(self):
        rep = idle_report(launches([201.6, 185, 185, 185, 185, 185], footprints=[500.0, 90.1, 90.3, 89.9, 90.2, 90.0]), IDENT)
        self.assertEqual(rep["footprint_median_mib"], 90.1)
        self.assertEqual(check(bl(181.6), dict(rep, footprint_median_mib=900.0), None, ci=False), [])
        self.assertIn("90.1 MiB", idle_summary(rep))

    def test_a_missing_footprint_tool_does_not_fail_the_gate(self):
        rep = idle_report(launches([201.6, 185, 185, 185, 185, 185], footprints=[None] * 6), IDENT)
        self.assertIsNone(rep["footprint_median_mib"])
        self.assertEqual(check(bl(181.6), rep, None, ci=False), [])

    def test_an_idle_json_from_before_the_rule_has_no_extra_summary(self):
        self.assertEqual(idle_summary(run(150)), "")

    def test_launch_count_is_one_warmup_plus_five_gated(self):
        self.assertEqual((WARMUP, GATED, LAUNCHES), (1, 5, 6))


POST = {"pre_load_mib": 150.0, "peak_mib": 210.0, "plus2_mib": 190.0, "plus10_mib": 195.0, "retained_mib": 45.0, "helpers": 2}


class PostLoad(unittest.TestCase):
    def idle(self, **extra):
        return {**run(150), **extra}

    def test_summary_takes_medians_and_the_peak_and_the_retained_growth(self):
        s = postload_summary([{"total_mib": 149, "helpers": 2}, {"total_mib": 151, "helpers": 2}],
                             [{"total_mib": 205, "helpers": 2}, {"total_mib": 212, "helpers": 2}],
                             [{"total_mib": 188, "helpers": 2}, {"total_mib": 192, "helpers": 2}, {"total_mib": 190, "helpers": 2}],
                             [{"total_mib": 194, "helpers": 2}, {"total_mib": 196, "helpers": 2}, {"total_mib": 195, "helpers": 2}])
        self.assertEqual((s["pre_load_mib"], s["peak_mib"], s["plus2_mib"], s["plus10_mib"], s["retained_mib"]), (150.0, 212, 190, 195, 45.0))

    def test_no_baseline_means_reported_only_never_a_failure(self):
        fails, warns = evaluate(bl(), self.idle(post_load=POST), None)
        self.assertEqual(fails, [])
        self.assertTrue(any("post-load" in w and "no baseline" in w for w in warns), warns)

    def test_over_the_baseline_or_the_ceiling_only_warns(self):
        b = bl()
        b["identities"][ID]["flavours"][NW]["post_load_mib"] = 150
        fails, warns = evaluate(b, self.idle(post_load={**POST, "plus10_mib": 205}), None)
        self.assertEqual(fails, [])
        self.assertTrue(any("post-load" in w and "200" in w for w in warns), warns)
        self.assertTrue(any("post-load" in w and "baseline 150" in w for w in warns), warns)

    def test_a_run_without_a_post_load_section_is_unchanged(self):
        fails, warns = evaluate(bl(), self.idle(), None)
        self.assertEqual((fails, warns), ([], []))


class PostOnly(unittest.TestCase):
    def test_a_standalone_post_load_file_reports_and_never_fails(self):
        warns, md = post_only(bl(), {"identity": ID, "helpers": 2, "flavour": NW, "post_load": POST})
        self.assertTrue(any("no baseline" in w for w in warns), warns)
        self.assertIn("| 150.0 | 210.0 | 190.0 | 195.0 | 45.0 MiB |", md)

    def test_an_unknown_identity_still_reports(self):
        warns, md = post_only(bl(), {"identity": "other", "helpers": 3, "flavour": GPU, "post_load": POST})
        self.assertTrue(warns and md)


if __name__ == "__main__":
    unittest.main()
