import unittest
from gate import check, evaluate
from measure import clean_median, identity_key, flavour_summary

ID = "25D1|20622.1.22"  # macOS build | WebKit version


def bl(mib=150, helpers=2, key=ID):
    return {"identities": {key: {"flavours": {str(helpers): {"idle_mib": mib}}}}}


def run(v, helpers=2, key=ID):
    return {"median_of_medians_mib": v, "identity": key, "helpers": helpers}


class Gate(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(check(bl(150), run(152), {"slope_mib_per_min": 0.2}), [])

    def test_absolute_budget_gates_the_two_helper_flavour(self):
        self.assertEqual(len(check(bl(200), run(201), None)), 1)
        self.assertEqual(check(bl(200), run(200), None), [])

    def test_regression_over_five(self):
        self.assertEqual(len(check(bl(150), run(156), None)), 1)
        self.assertEqual(check(bl(150), run(155), None), [])

    def test_tolerance_is_the_larger_of_5_mib_and_3_percent(self):
        self.assertEqual(check(bl(174), run(179.2), None), [])   # 3% = 5.22
        self.assertEqual(len(check(bl(174), run(179.3), None)), 1)
        self.assertEqual(check(bl(100), run(105), None), [])     # 5 MiB floor
        self.assertEqual(len(check(bl(100), run(105.1), None)), 1)

    def test_slope(self):
        self.assertEqual(len(check(bl(), run(100), {"slope_mib_per_min": 1.5})), 1)


class Flavours(unittest.TestCase):
    two_three = {"identities": {ID: {"flavours": {"2": {"idle_mib": 175.0}, "3": {"idle_mib": 190.0}}}}}

    def test_each_flavour_is_compared_to_its_own_baseline(self):
        self.assertEqual(check(self.two_three, run(176, 2), None), [])
        self.assertEqual(check(self.two_three, run(191, 3), None), [])
        self.assertEqual(len(check(self.two_three, run(190.0 + 5.8, 3), None)), 1)   # 3% of 190 = 5.7
        self.assertEqual(len(check(self.two_three, run(190.0, 2), None)), 1)          # over 175 + 5.25

    def test_the_200_ceiling_fails_every_flavour_including_three_helpers(self):
        t = {"identities": {ID: {"flavours": {"2": {"idle_mib": 199.0}, "3": {"idle_mib": 199.0}}}}}
        for n in (2, 3):
            self.assertEqual(check(t, run(200.0, n), None), [], n)
            f = check(t, run(200.5, n), None)
            self.assertEqual(len(f), 1, n)
            self.assertIn("200", f[0])
            self.assertEqual(evaluate(t, run(200.5, n), None)[1], [], n)  # a failure, not a warning

    def test_a_provisional_entry_only_warns_on_regression(self):
        t = {"identities": {ID: {"flavours": {"3": {"idle_mib": 150.0, "provisional": True}}}}}
        fails, warns = evaluate(t, run(190, 3), None)
        self.assertEqual(fails, [])
        self.assertEqual(len(warns), 1)
        self.assertIn("provisional", warns[0])
        self.assertIn("190", warns[0])

    def test_a_provisional_entry_still_fails_the_ceiling_growth_and_unknowns(self):
        t = {"identities": {ID: {"flavours": {"3": {"idle_mib": 150.0, "provisional": True}}}}}
        self.assertEqual(len(evaluate(t, run(201, 3), None)[0]), 1)                         # ceiling
        self.assertEqual(len(evaluate(t, run(150, 3), {"slope_mib_per_min": 2})[0]), 1)     # growth
        self.assertEqual(len(evaluate(t, run(150, 2), None)[0]), 1)                         # no 2-helper entry
        self.assertEqual(len(evaluate(t, run(150, 3, "NEW"), None)[0]), 1)                  # unknown identity

    def test_a_non_provisional_entry_gates_the_regression(self):
        t = {"identities": {ID: {"flavours": {"3": {"idle_mib": 150.0, "provisional": False}}}}}
        self.assertEqual(len(evaluate(t, run(190, 3), None)[0]), 1)

    def test_three_helper_flavour_still_gates_growth(self):
        fails, _ = evaluate(self.two_three, run(190, 3), {"slope_mib_per_min": 2})
        self.assertEqual(len(fails), 1)

    def test_any_other_helper_count_fails_closed(self):
        for n in (0, 1, 4, 7):
            f = check(self.two_three, run(150, n), None)
            self.assertEqual(len(f), 1, n)
            self.assertIn("helper", f[0])

    def test_a_known_identity_without_that_flavour_fails_closed(self):
        f = check(bl(150, 2), run(150, 3), None)
        self.assertEqual(len(f), 1)
        self.assertIn("baseline.json", f[0])

    def test_unknown_identity_fails_closed_with_the_identity_in_the_message(self):
        f = check(bl(), run(100, 2, "NEW"), None)
        self.assertEqual(len(f), 1)
        self.assertIn("NEW", f[0])
        self.assertIn("baseline.json", f[0])

    def test_other_identity_does_not_borrow_a_baseline(self):
        t = bl(150, 2)
        t["identities"]["B"] = {"flavours": {"3": {"idle_mib": 192}}}
        self.assertEqual(check(t, run(192, 3, "B"), None), [])
        self.assertEqual(len(check(t, run(192, 3, ID), None)), 1)

    def test_unknown_identity_still_enforces_the_budget_for_two_helpers_and_slope(self):
        self.assertEqual(len(check(bl(), run(201, 2, "NEW"), {"slope_mib_per_min": 2})), 3)

    def test_missing_identity_in_idle_json_fails_closed(self):
        self.assertEqual(len(check(bl(), {"median_of_medians_mib": 100, "helpers": 2}, None)), 1)

    def test_key_is_os_build_and_webkit_not_the_image_version(self):
        self.assertEqual(identity_key("25D1", "20622.1.22"), "25D1|20622.1.22")


class Measure(unittest.TestCase):
    def r(self, median, *helpers):
        return {"median_mib": median, "helpers": list(helpers)}

    def test_a_launch_belongs_to_the_smallest_helper_count_it_saw(self):
        self.assertEqual(flavour_summary([self.r(201, 3, 6), self.r(202, 3), self.r(203, 4, 7)])[0], 3)

    def test_the_median_is_taken_only_over_launches_of_the_reported_flavour(self):
        # one 2-helper launch at 175 followed by four 3-helper launches: report 3 helpers and 201, never helpers=2 with 201
        runs = [self.r(175, 2), self.r(201, 3), self.r(202, 3), self.r(200.8, 3), self.r(201.1, 3)]
        self.assertEqual(flavour_summary(runs), (3, 201.05, 4))

    def test_the_majority_flavour_wins_and_the_minority_launch_is_left_out(self):
        runs = [self.r(175, 2), self.r(174, 2), self.r(176, 2), self.r(201, 3), self.r(202, 3)]
        self.assertEqual(flavour_summary(runs), (2, 175, 3))

    def test_a_tie_between_flavours_reports_the_larger_helper_count(self):
        self.assertEqual(flavour_summary([self.r(175, 2), self.r(201, 3)]), (3, 201, 1))

    def s(self, total, helpers):
        return {"total_mib": total, "helpers": helpers}

    def test_samples_with_foreign_helpers_are_dropped(self):
        samples = [self.s(170, 2), self.s(172, 2), self.s(171, 2), self.s(260, 7), self.s(250, 7)]
        self.assertEqual(clean_median(samples), (171, 3))

    def test_a_launch_with_no_foreign_helpers_keeps_everything(self):
        self.assertEqual(clean_median([self.s(100, 2), self.s(102, 2)]), (101, 2))

    def test_the_smallest_helper_count_of_the_launch_is_the_baseline_even_if_it_is_not_2(self):
        self.assertEqual(clean_median([self.s(180, 3), self.s(300, 6)]), (180, 1))


if __name__ == "__main__":
    unittest.main()
