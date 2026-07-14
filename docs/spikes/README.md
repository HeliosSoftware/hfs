# Spikes

Throwaway code kept for the record. **None of this is production code**, none of it
is wired into the build, and none of it is maintained. It exists so a design
document can point at a result and say "this was measured, not assumed".

## `resource-editor-addable.js` (#264)

Answers the one question the schema-driven resource editor rests on:

> Given a cursor somewhere inside a FHIR resource, **what nodes can I add here?**

It runs the projection algorithm against the **real R4 schema pack** from
`feat/fhir-validator` (PR #232) — nothing mocked — and prints, for several cursor
positions in a Patient, the set of addable elements: spec-ordered, with cardinality
already spent excluded, choice types offered as a type pick, and extensions
(including nested ones) falling out of the same walk.

Deliberately written outside Rust: it tests the **IR**, not an integration. If the
IR can answer the question from a Node script, it can answer it from the UI crate.

```bash
# extract the pack from the validator branch, then run against it
git show origin/feat/fhir-validator:crates/fhir-validator/packs/fhir_schemas_r4.json.gz > /tmp/r4.json.gz
node docs/spikes/resource-editor-addable.js /tmp
```

Findings are written up in [`../resource-editor-design.md`](../resource-editor-design.md) §2.
