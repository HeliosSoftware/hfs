import unittest

import host_changes as hc

CH = {"at": "2026-10-28T14:45:00Z", "approximate": True, "note": "the Docker host CPUs were raised from 4 to 8"}


class T(unittest.TestCase):
    def test_changes_between_inside_and_outside(self):
        self.assertEqual(hc.changes_between("2026-10-27T10:00:00Z", "2026-10-29T10:00:00Z"), hc.HOST_CHANGES)
        self.assertEqual(hc.changes_between("2026-10-20", "2026-10-28T14:00:00Z"), [])
        self.assertEqual(hc.changes_between("2026-10-28T15:00:00Z", "2026-10-30"), [])

    def test_changes_between_bounds_and_dates(self):
        # exclusive at the start, inclusive at the end; bare dates mean midnight
        self.assertEqual(hc.changes_between("2026-10-28T14:45:00Z", "2026-10-29"), [])
        self.assertEqual(len(hc.changes_between("2026-10-27", "2026-10-28T14:45:00Z")), 1)
        self.assertEqual(hc.changes_between("2026-10-27", "2026-10-28"), [])
        self.assertEqual(hc.changes_between("", "2026-10-30"), [])
        self.assertEqual(hc.changes_between("2026-10-27", None), [])

    def test_before_change(self):
        self.assertTrue(hc.before_change("2026-10-28T14:44:59Z", CH))
        self.assertTrue(hc.before_change("2026-10-20", CH))
        self.assertFalse(hc.before_change("2026-10-28T14:45:00Z", CH))
        self.assertFalse(hc.before_change("2026-10-29", CH))
        self.assertFalse(hc.before_change("", CH))

    def test_legend(self):
        t = hc.legend(CH)
        self.assertIn("2026-10-28 14:45 UTC (approximate)", t)
        self.assertIn("raised from 4 to 8", t)
        self.assertTrue(t.startswith("Runs before"))
        exact = hc.legend(dict(CH, approximate=False))
        self.assertNotIn("approximate", exact)


if __name__ == "__main__":
    unittest.main()
