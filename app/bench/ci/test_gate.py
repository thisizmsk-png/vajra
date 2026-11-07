import unittest
from gate import check, evaluate
from measure import clean_median, identity_key, base_helpers

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
    two_three = {"identities": {ID: {"flavours": {"2": {"idle_mib": 175.0}, "3": {"idle_mib": 201.1}}}}}

    def test_each_flavour_is_compared_to_its_own_baseline(self):
        self.assertEqual(check(self.two_three, run(176, 2), None), [])
        self.assertEqual(check(self.two_three, run(201.5, 3), None), [])
        self.assertEqual(len(check(self.two_three, run(201.1, 2), None)), 2)  # over 200 AND over 175 + 5.25

    def test_three_helper_flavour_over_200_is_a_warning_with_the_number_not_a_failure(self):
        fails, warns = evaluate(self.two_three, run(201.5, 3), None)
        self.assertEqual(fails, [])
        self.assertEqual(len(warns), 1)
        self.assertIn("201.5", warns[0])
        self.assertIn("200", warns[0])

    def test_three_helper_flavour_under_200_has_no_warning(self):
        t = {"identities": {ID: {"flavours": {"3": {"idle_mib": 190.0}}}}}
        self.assertEqual(evaluate(t, run(191, 3), None), ([], []))

    def test_three_helper_flavour_still_gates_a_regression(self):
        fails, _ = evaluate(self.two_three, run(201.1 + 6.1, 3), None)
        self.assertEqual(len(fails), 1)

    def test_three_helper_flavour_still_gates_growth(self):
        fails, _ = evaluate(self.two_three, run(201.1, 3), {"slope_mib_per_min": 2})
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
    def test_base_helpers_is_the_smallest_count_seen_in_any_launch(self):
        self.assertEqual(base_helpers([{"helpers": [3, 6]}, {"helpers": [3]}, {"helpers": [4, 7]}]), 3)

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
