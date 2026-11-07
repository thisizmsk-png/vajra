import unittest
from gate import check
from measure import clean_median


class Gate(unittest.TestCase):
    idle = lambda self, v: {"median_of_medians_mib": v}

    def test_ok(self):
        self.assertEqual(check({"idle_mib": 150}, self.idle(152), {"slope_mib_per_min": 0.2}), [])

    def test_absolute_budget(self):
        self.assertEqual(len(check({"idle_mib": None}, self.idle(201), None)), 1)

    def test_regression_over_five(self):
        self.assertEqual(len(check({"idle_mib": 150}, self.idle(156), None)), 1)
        self.assertEqual(check({"idle_mib": 150}, self.idle(155), None), [])

    def test_tolerance_is_the_larger_of_5_mib_and_3_percent(self):
        self.assertEqual(check({"idle_mib": 174}, self.idle(179.2), None), [])   # 3% = 5.22
        self.assertEqual(len(check({"idle_mib": 174}, self.idle(179.3), None)), 1)
        self.assertEqual(check({"idle_mib": 100}, self.idle(105), None), [])     # 5 MiB floor
        self.assertEqual(len(check({"idle_mib": 100}, self.idle(105.1), None)), 1)

    def test_slope(self):
        self.assertEqual(len(check({"idle_mib": None}, self.idle(100), {"slope_mib_per_min": 1.5})), 1)


class Measure(unittest.TestCase):
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
