import unittest
from gate import check


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


if __name__ == "__main__":
    unittest.main()
