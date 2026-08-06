# Sample FHIR NPM package fixtures

| Artifact | Purpose |
|----------|---------|
| `sample/` | Expanded source of truth (`package.json` + StructureDefinitions) |
| `sample.tgz` | FHIR NPM tarball (`package/…` layout) used by package validation tests |

Package id: **`example.fhir.r4.sample@0.1.0`**  
Canonical: `http://example.org/fhir/r4/sample`

Profiles exercise common IG patterns without vendor branding:

- required demographics (`identifier`, `name`, `gender`)
- identifier slice by `system` (MRN)
- required extension slice (`sample-facility-code`)
- encounter with `targetProfile` on `subject`

Rebuild the tarball after editing `sample/`:

```bash
./pack.sh
```
