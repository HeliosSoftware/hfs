import json
import unittest

from build_features import features_for


def m(*legs):
    return {"include": [{"backend": b, "port": 1} for b in legs]}


class BuildFeatures(unittest.TestCase):
    def test_single_legs(self):
        self.assertEqual(features_for(m("sqlite")), "R4,sqlite")
        self.assertEqual(features_for(m("postgres")), "R4,postgres")
        self.assertEqual(features_for(m("mongodb")), "R4,mongodb")

    def test_composite_legs(self):
        self.assertEqual(features_for(m("sqlite-elasticsearch")), "R4,sqlite,elasticsearch")
        self.assertEqual(features_for(m("postgres-elasticsearch")), "R4,postgres,elasticsearch")
        self.assertEqual(features_for(m("mongodb-elasticsearch")), "R4,mongodb,elasticsearch")

    def test_core_is_todays_list(self):
        legs = ["sqlite", "postgres", "mongodb", "sqlite-elasticsearch",
                "postgres-elasticsearch", "mongodb-elasticsearch"]
        self.assertEqual(features_for(m(*legs)), "R4,sqlite,postgres,mongodb,elasticsearch")

    def test_order_independent(self):
        self.assertEqual(features_for(m("postgres", "sqlite")), "R4,sqlite,postgres")

    def test_elasticsearch_group(self):
        legs = ["sqlite-elasticsearch", "postgres-elasticsearch", "mongodb-elasticsearch"]
        self.assertEqual(features_for(m(*legs)), "R4,sqlite,postgres,mongodb,elasticsearch")

    def test_errors(self):
        with self.assertRaises(ValueError):
            features_for(m("redis"))
        with self.assertRaises(ValueError):
            features_for({"include": []})

    def test_json_roundtrip(self):
        self.assertEqual(features_for(json.loads(json.dumps(m("sqlite")))), "R4,sqlite")


if __name__ == "__main__":
    unittest.main()
