"""Tests for resolve-matrix.sh: the `backend` input as a list with exclusions."""
import json
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "resolve-matrix.sh")

ORDER = ["sqlite", "postgres", "mongodb", "sqlite-elasticsearch",
         "postgres-elasticsearch", "mongodb-elasticsearch"]
PORTS = dict(zip(ORDER, range(8081, 8087)))


def resolve(backend):
    """Run the script; return (exit code, legs list or None, stdout)."""
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "github_output")
        open(out, "w").close()
        env = {"PATH": os.environ["PATH"], "GITHUB_OUTPUT": out, "IN_BACKEND": backend}
        p = subprocess.run(["bash", SCRIPT], env=env, capture_output=True, text=True)
        legs = None
        with open(out) as f:
            lines = f.read().splitlines()
        for line in lines:
            if line.startswith("matrix="):
                m = json.loads(line[len("matrix="):])
                legs = [(i["backend"], i["port"]) for i in m["include"]]
        return p.returncode, legs, p.stdout


def names(backend):
    rc, legs, out = resolve(backend)
    assert rc == 0, out
    return [n for n, _ in legs]


class ResolveMatrix(unittest.TestCase):
    def test_exclusions_from_core(self):
        self.assertEqual(
            names("core,-sqlite,-sqlite-elasticsearch"),
            ["postgres", "mongodb", "postgres-elasticsearch", "mongodb-elasticsearch"])

    def test_alias_excluded(self):
        self.assertEqual(names("all,-elasticsearch"), ["sqlite", "postgres", "mongodb"])
        self.assertEqual(
            names("elasticsearch,-mongodb-elasticsearch"),
            ["sqlite-elasticsearch", "postgres-elasticsearch"])

    def test_dedupe(self):
        self.assertEqual(names("sqlite,sqlite"), ["sqlite"])

    def test_canonical_order_and_ports(self):
        rc, legs, _ = resolve("mongodb-elasticsearch, sqlite ,postgres")
        self.assertEqual(rc, 0)
        self.assertEqual([n for n, _ in legs], ["sqlite", "postgres", "mongodb-elasticsearch"])
        for n, port in legs:
            self.assertEqual(port, PORTS[n])

    def test_only_exclusions_start_from_core(self):
        self.assertEqual(names("-sqlite"), ORDER[1:])

    def test_whitespace_ignored(self):
        self.assertEqual(names("  postgres , mongodb "), ["postgres", "mongodb"])

    def test_inclusion_after_exclusion_order_irrelevant(self):
        self.assertEqual(names("-sqlite,all"), ORDER[1:])

    def test_errors(self):
        for bad in ["bogus", "sqlite,-sqlite", "sqlite,,postgres", "core,-core", "sqlite,bogus", "-bogus"]:
            with self.subTest(bad=bad):
                rc, legs, out = resolve(bad)
                self.assertNotEqual(rc, 0)
                self.assertIsNone(legs)
                self.assertIn("::error::", out)

    def test_legacy_values(self):
        self.assertEqual(names("core"), ORDER)
        self.assertEqual(names("all"), ORDER)
        self.assertEqual(names("sqlite-postgres"), ["sqlite", "postgres"])
        self.assertEqual(names("elasticsearch"), ORDER[3:])
        for leg in ORDER:
            self.assertEqual(names(leg), [leg])

    def test_legacy_matrix_json_exact(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "o")
            open(out, "w").close()
            env = {"PATH": os.environ["PATH"], "GITHUB_OUTPUT": out, "IN_BACKEND": "sqlite-postgres"}
            subprocess.run(["bash", SCRIPT], env=env, capture_output=True, text=True, check=True)
            with open(out) as f:
                lines = f.read().splitlines()
            self.assertIn(
                'matrix={"include":[{"backend":"sqlite","port":8081},{"backend":"postgres","port":8082}]}',
                lines)


if __name__ == "__main__":
    unittest.main()
