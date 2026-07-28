# Official FHIR example corpus baselines

Baselines for the sweep in [`../../spec_examples.rs`](../../spec_examples.rs),
which validates every published FHIR example resource against the embedded
core schema packs.

## What a baseline is

The corpus is the set of example resources the FHIR spec publishes, vendored
at `crates/fhir/tests/data/json/<VERSION>/`. Every one of them is *supposed*
to be valid against the core spec. So each entry in `knownFailures` is a
suspected **false positive in our engine** — not a defect in the example.

That makes these files a running count of the engine's known
over-reporting, per FHIR version, in a form review can see:

| File | Corpus |
|---|---|
| `known-failures-r4.json` | `crates/fhir/tests/data/json/R4` |
| `known-failures-r4b.json` | `crates/fhir/tests/data/json/R4B` |
| `known-failures-r5.json` | `crates/fhir/tests/data/json/R5` |

R6 has no baseline on purpose: `crates/fhir/build.rs` wipes and re-downloads
`tests/data/json/R6` from `build.fhir.org` whenever the R6 feature is on and
the local copy is over 24 hours old, so its content is volatile and nothing
stable can be pinned to it.

## The ratchet

The test fails on divergence in **either** direction:

- a file that fails and is not in the baseline (regression, or a real bug the
  sweep just found);
- a baseline entry that now validates clean (stale — delete the entry);
- a file failing with different error kinds or a different issue count than
  recorded (one bug traded for another);
- a change in the resource count or the non-resource file list (the corpus
  moved; regenerate).

Entries may only be removed by fixing the engine. Adding one is a deliberate,
reviewable act — it is a record that we report an issue on a valid published
resource, never a way to silence one. Nothing here suppresses output: the
issues are still emitted, still counted, and still printed by the sweep.

## Regenerating

Every run writes its freshly computed manifest to
`target/spec-examples/<version>.actual.json`, pass or fail. The
`Validator Conformance` workflow uploads that directory as the
**spec-example-manifests** artifact.

```bash
cargo test -p helios-fhir-validator --all-features --test spec_examples \
  -- --ignored --nocapture

# inspect the diff, then accept it
cp target/spec-examples/r4.actual.json \
   crates/fhir-validator/tests/fixtures/spec-examples/known-failures-r4.json
```

The optional per-entry `reason` field is never generated — add it by hand when
a group of entries shares an understood root cause, so the next reader knows
which are explained and which are still unexamined.

## Scope

Structural validation only (`Validator::validate_sync`): no FHIRPath
invariants, no terminology bindings. Sweeping the corpus through the deferred
effects pass is a worthwhile second tier and deliberately not attempted here —
it needs an async runtime and a terminology posture, and it would conflate
engine bugs with terminology-server availability.

Profiles named in `meta.profile` that we do ship are applied; ones we do not
ship (US Core, IHE, national IGs) are ignored rather than reported, since this
is a core-spec sweep. Profile conformance is the Inferno job's business
(issue #368).
