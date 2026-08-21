# Vendor provenance — fhir-developer

| Field | Value |
|-------|-------|
| Upstream | [anthropics/healthcare](https://github.com/anthropics/healthcare) |
| Path | `plugins/healthcare/skills/fhir-developer/` |
| Upstream skill name | `fhir-developer` / `fhir-developer-skill` |
| License | Provided under [Anthropic's terms of service](https://www.anthropic.com/legal/consumer-terms) (per upstream README) |
| Vendored for | Cursor project skills (`.cursor/skills/`) |

## Contents

- `SKILL.md` — main skill body (with a short Helios HFS adaptation note)
- `references/` — bundles, pagination, resource examples, SMART auth
- `scripts/setup_fhir_project.py` — FastAPI scaffold helper from upstream

## Refresh

```powershell
$raw = "https://raw.githubusercontent.com/anthropics/healthcare/main/plugins/healthcare/skills/fhir-developer"
# Re-download SKILL.md, references/*, and scripts/* into this folder, then re-apply the
# Helios HFS notes section in SKILL.md if upstream overwrote it.
```
