# UI dev tooling

Local helpers for exercising the web UI against realistic data. Nothing here
ships or runs in CI.

## Dashboard demo data

The Home chart (#555) plots cumulative growth from the immutable history log,
so a fresh store shows one vertical step at "now". These two scripts produce
30 days of believable curves across several resource types:

```bash
# 1. With the server running (sqlite backend, default tenant):
node crates/ui/dev/seed-dashboard.mjs          # ~1.2k resources via batch bundles

# 2. Stop the server, then spread the creations across the past 30 days:
python crates/ui/dev/backdate-dashboard.py     # rewrites history timestamps in fhir.db

# 3. Restart the server and open /ui.
```

`backdate-dashboard.py` edits `fhir.db` in the repo root (adjust `DB` at the
top for another path) and gives each type its own curve shape — accelerating,
steady, or front-loaded — so the comparison chart has something to say. It
only touches `last_updated` timestamps, and only for the default tenant.
