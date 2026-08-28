"""Build a 500-patient Patient Bundle + ViewDefinition for the DuckDB-WASM POC.

Writes view.json and bundle.json next to this script. The columns match the
POC's GROUP BY demo: id, gender, birth_date, active, city.
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
N = 500

VIEW = {
    "resourceType": "ViewDefinition",
    "name": "patient_demographics",
    "status": "active",
    "resource": "Patient",
    "select": [
        {
            "column": [
                {"name": "id", "path": "id"},
                {"name": "gender", "path": "gender"},
                {"name": "birth_date", "path": "birthDate"},
                {"name": "active", "path": "active"},
                {"name": "city", "path": "address.first().city"},
            ]
        }
    ],
}

# Weighted gender mix so the GROUP BY has clearly different bar heights.
GENDERS = (["female"] * 11 + ["male"] * 8 + ["other"] * 1)  # 55% / 40% / 5%
CITIES = ["Springfield", "Riverton", "Fairview", "Lakeside", "Kingsport"]

entries = []
for i in range(N):
    g = GENDERS[i % len(GENDERS)]
    entries.append(
        {
            "resource": {
                "resourceType": "Patient",
                "id": f"p{i:04d}",
                "gender": g,
                "birthDate": f"19{50 + (i % 50):02d}-{(i % 12) + 1:02d}-{(i % 28) + 1:02d}",
                "active": (i % 4) != 0,
                "name": [{"family": f"Family{i}", "given": [f"Given{i}"]}],
                "address": [{"city": CITIES[i % len(CITIES)]}],
            }
        }
    )

with open(os.path.join(HERE, "view.json"), "w") as f:
    json.dump(VIEW, f, indent=2)
with open(os.path.join(HERE, "bundle.json"), "w") as f:
    json.dump({"resourceType": "Bundle", "type": "collection", "entry": entries}, f)

print(f"wrote view.json and bundle.json ({N} patients)")
