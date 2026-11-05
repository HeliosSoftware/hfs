# Backdates the seeded resources' history rows so the dashboard chart shows
# growth curves over the past 30 days instead of one vertical step at "now".
# Each type gets its own curve shape (accelerating, steady, bursty). Run with
# the server STOPPED. Only rows whose resource carries the seed marker family
# names / codes stay untouched apart from timestamps.
import sqlite3, random, math
from datetime import datetime, timedelta, timezone

DB = r"c:\dev\hfs\fhir.db"
DAYS = 30
now = datetime.now(timezone.utc).replace(microsecond=0)

# type -> (curve exponent, jitter). exponent > 1 back-loads creation toward
# "now" (accelerating growth); < 1 front-loads it (early adoption, flattening).
SHAPES = {
    "Patient": 0.8,
    "Observation": 1.6,
    "Encounter": 1.2,
    "Condition": 0.9,
    "MedicationRequest": 1.4,
    "DiagnosticReport": 1.1,
    "Practitioner": 0.5,
    "Organization": 0.4,
}

conn = sqlite3.connect(DB)
cur = conn.cursor()
random.seed(555)

total = 0
for rtype, exponent in SHAPES.items():
    rows = cur.execute(
        "SELECT rowid, id FROM resource_history "
        "WHERE tenant_id='default' AND resource_type=? AND version_id='1' "
        "ORDER BY rowid",
        (rtype,),
    ).fetchall()
    n = len(rows)
    if not n:
        continue
    for i, (rowid, rid) in enumerate(rows):
        # Position in [0,1): 0 = oldest, 1 = now; the exponent bends the curve.
        u = (i + random.random() * 0.8) / n
        age_days = DAYS * (1 - u ** exponent)
        at = now - timedelta(days=age_days, seconds=random.randint(0, 3600))
        stamp = at.strftime("%Y-%m-%dT%H:%M:%S+00:00")
        cur.execute(
            "UPDATE resource_history SET last_updated=? WHERE rowid=?",
            (stamp, rowid),
        )
        cur.execute(
            "UPDATE resources SET last_updated=? "
            "WHERE tenant_id='default' AND resource_type=? AND id=?",
            (stamp, rtype, rid),
        )
        total += 1
    print(rtype, n)

conn.commit()
conn.close()
print("backdated", total, "creations across", DAYS, "days")
