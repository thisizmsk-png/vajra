import unittest
from gate import check
from measure import clean_median, identity_key, base_helpers

ID = "25D1|20622.1.22|"  # macOS build | WebKit version


def bl(mib=150, helpers=2, key=ID):
    return {"identities": {key: {"helpers": helpers, "idle_mib": mib}}}


def run(v, helpers=2, key=ID):
    return {"median_of_medians_mib": v, "identity": key, "helpers": helpers}


class Gate(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(check(bl(150), run(152), {"slope_mib_per_min": 0.2}), [])

    def test_absolute_budget(self):
        self.assertEqual(len(check(bl(200), run(201), None)), 1)

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


class Identity(unittest.TestCase):
    def test_known_identity_is_compared_to_its_own_baseline(self):
        self.assertEqual(check(bl(150), run(152), None), [])
        self.assertEqual(len(check(bl(150), run(156), None)), 1)

    def test_other_identity_does_not_borrow_the_baseline_of_a_different_one(self):
        two = bl(150, 2)
        two["identities"]["B"] = {"helpers": 3, "idle_mib": 192}
        self.assertEqual(check(two, run(192, 3, "B"), None), [])
        self.assertEqual(len(check(two, run(192, 3, ID), None)), 2)  # over 150 baseline AND helper mismatch

    def test_unknown_identity_fails_closed_with_the_identity_in_the_message(self):
        f = check(bl(), run(100, 2, "NEW"), None)
        self.assertEqual(len(f), 1)
        self.assertIn("NEW", f[0])
        self.assertIn("baseline.json", f[0])

    def test_known_identity_with_a_different_helper_count_is_our_regression(self):
        f = check(bl(150, 2), run(151, 3), None)
        self.assertEqual(len(f), 1)
        self.assertIn("helper", f[0])

    def test_unknown_identity_still_enforces_the_absolute_budget_and_slope(self):
        self.assertEqual(len(check(bl(), run(201, 2, "NEW"), {"slope_mib_per_min": 2})), 3)

    def test_missing_identity_in_idle_json_fails_closed(self):
        self.assertEqual(len(check(bl(), {"median_of_medians_mib": 100}, None)), 1)

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
