import unittest
from gate import check, evaluate
from measure import classify, flavour_key, flavour_summary, identity_key, launch_flavours, launch_summary, helper_type

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
    """Owner ruling: about 212 MiB on CI only when the GPU process is present; real machines stay at 200."""
    gpu = bl(204.0, GPU)

    def test_gpu_on_ci_at_205_passes(self):
        self.assertEqual(check(self.gpu, run(205, GPU), None, ci=True), [])

    def test_gpu_on_ci_over_212_fails(self):
        f = check(self.gpu, run(212.1, GPU), None, ci=True)
        self.assertTrue(any("212" in x for x in f), f)
        self.assertEqual(check(bl(210.0, GPU), run(212.0, GPU), None, ci=True), [])

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
        self.assertEqual(len(evaluate(t, run(213, GPU), None, ci=True)[0]), 1)


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


if __name__ == "__main__":
    unittest.main()
